use super::*;
use crate::adapters::test_support::read_rpc_server::{LocalServer, Reply};
use serde_json::{json, Value};

fn response(sharing: Option<UiAccount>) -> Value {
    let mut values = snapshot(None, spl_token_interface::ID);
    values[3] = sharing;
    json!({"context":{"slot":123},"value":values})
}

fn fixture(result: Value, mode: Reply) -> Reply {
    Reply::FixedAccountSnapshot {
        count: ACCOUNT_COUNT,
        result,
        mode: Box::new(mode),
    }
}

fn runtime(servers: &[&LocalServer]) -> SharedReadRuntime {
    SharedReadRuntime::new(&servers.iter().map(|s| s.url.clone()).collect::<Vec<_>>()).unwrap()
}

fn until() -> Instant {
    Instant::now() + Duration::from_secs(4)
}

fn count(server: &LocalServer) -> usize {
    server.requests.lock().unwrap().len()
}

#[tokio::test]
async fn one_exact_snapshot_binds_slot_and_explicit_sharing_state() {
    let plan = sol();
    for present in [false, true] {
        let config = present.then(|| ui(accounts::FEE_PROGRAM, &sharing(1)));
        let server = LocalServer::new(fixture(response(config), Reply::Success)).await;
        let runtime = runtime(&[&server]);
        let actual = plan
            .read(&runtime, &server.url, 122, until())
            .await
            .unwrap();
        assert_eq!(actual.keys, *plan.keys());
        assert_eq!(actual.slot, 123);
        assert_eq!(
            actual.sharing,
            if present {
                SharingPrefix::Present {
                    version: 1,
                    bump: sharing_bump(),
                    status: 1,
                }
            } else {
                SharingPrefix::Absent
            }
        );
        let keys = plan.keys().map(|key| key.to_string());
        assert_eq!(
            *server.requests.lock().unwrap(),
            [
                json!({"jsonrpc":"2.0","id":1,"method":"getMultipleAccounts","params":[keys,
                {"encoding":"base64","commitment":"processed","dataSlice":null,"minContextSlot":122}]})
            ]
        );
        assert!(runtime.parameter_slot().is_ok());
        server.finish().await;
    }
}

#[tokio::test]
async fn wire_error_and_old_slot_never_become_absent_or_fall_back() {
    let plan = sol();
    let backup = LocalServer::new(fixture(response(None), Reply::Success)).await;
    let mut short = response(None);
    short["value"].as_array_mut().unwrap().pop();
    let missing = json!({"context":{"slot":123}});
    let malformed = response(Some(ui(accounts::FEE_PROGRAM, &sharing(1)[..42])));
    for (reply, minimum) in [
        (fixture(response(None), Reply::Success), 124),
        (fixture(short, Reply::Success), 122),
        (fixture(missing, Reply::Success), 122),
        (fixture(malformed, Reply::Success), 122),
        (fixture(response(None), Reply::WrongId), 122),
        (fixture(response(None), Reply::RpcError), 122),
        (fixture(response(None), Reply::Disconnect), 122),
        (fixture(response(None), Reply::DeclaredOversize), 122),
        (fixture(response(None), Reply::RateLimited(Some("60"))), 122),
        (
            fixture(response(None), Reply::Redirect(backup.url.clone())),
            122,
        ),
    ] {
        let limited = matches!(&reply, Reply::FixedAccountSnapshot {mode, ..} if matches!(mode.as_ref(), Reply::RateLimited(_)));
        let primary = LocalServer::new(reply).await;
        let runtime = runtime(&[&primary, &backup]);
        let error = plan
            .read(&runtime, &primary.url, minimum, until())
            .await
            .unwrap_err();
        assert!(!format!("{error:#}").contains("fixture-secret"));
        if limited {
            assert!(plan
                .read(&runtime, &primary.url, 122, until())
                .await
                .is_err());
        }
        assert_eq!((count(&primary), count(&backup)), (1, 0));
        assert!(runtime.parameter_slot().is_ok());
        primary.finish().await;
    }
    backup.finish().await;
}

#[tokio::test]
async fn caller_deadline_and_cancellation_release_the_existing_parameter_permit() {
    let plan = sol();
    let server = LocalServer::new(fixture(response(None), Reply::SlowHeaders)).await;
    let runtime = runtime(&[&server]);
    assert!(plan.read(&runtime, &server.url, 0, until()).await.is_err());
    assert!(plan
        .read(&runtime, &server.url, 122, Instant::now())
        .await
        .is_err());
    assert_eq!(count(&server), 0);
    {
        let read = plan.read(&runtime, &server.url, 122, until());
        tokio::pin!(read);
        let started = tokio::time::timeout(Duration::from_secs(1), server.received.notified());
        tokio::select! {
            result = &mut read => panic!("held Pump.fun snapshot completed: {result:?}"),
            result = started => result.unwrap(),
        }
        assert!(runtime.parameter_slot().is_err());
    }
    assert!(runtime.parameter_slot().is_ok());
    let short = Instant::now() + Duration::from_secs(1);
    assert!(plan.read(&runtime, &server.url, 122, short).await.is_err());
    assert!(Instant::now() >= short);
    let requests: Vec<Value> = server.requests.lock().unwrap().clone();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        (requests[0]["id"].as_u64(), requests[1]["id"].as_u64()),
        (Some(1), Some(2))
    );
    assert!(runtime.parameter_slot().is_ok());
    server.finish().await;
}
