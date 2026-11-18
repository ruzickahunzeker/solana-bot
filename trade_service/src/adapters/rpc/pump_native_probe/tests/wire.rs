use super::*;
use crate::adapters::test_support::read_rpc_server::{LocalServer, Reply};

fn reply(mode: Reply) -> Reply {
    Reply::PumpNativeProbe {
        result: json!({"context":{"slot":175},"value":{
        "err":null,"accounts":vec![Value::Null;EVIDENCE_ACCOUNTS]}}),
        mode: Box::new(mode),
    }
}

#[tokio::test]
async fn pump_native_wire_uses_real_fixed_http_and_both_complete_body_boundaries() {
    let (tx, keys) = fixture();
    for mode in [Reply::Success, Reply::AtLimit, Reply::Chunked(524_288)] {
        let server = LocalServer::new(reply(mode)).await;
        let runtime = SharedReadRuntime::new(&[server.url.clone()]).unwrap();
        let plan = packet(&tx, &keys);
        let expected = plan.body(1);
        let result = runtime
            .pump_native_probe(&server.url, plan)
            .await
            .unwrap()
            .for_probe(&tx, &keys, 175)
            .unwrap();
        assert_eq!(result["context"]["slot"], 175);
        assert_eq!(*server.requests.lock().unwrap(), [expected]);
        assert!(runtime.aggregator_simulation_slot(&server.url).is_ok());
        server.finish().await;
    }
}

#[tokio::test]
async fn pump_native_wire_blocks_bad_envelopes_body_overflow_redirect_and_shared_429() {
    let (tx, keys) = fixture();
    let backup = LocalServer::new(Reply::Success).await;
    for mode in [
        Reply::WrongId,
        Reply::InvalidJson,
        Reply::RpcError,
        Reply::ServerError,
        Reply::DeclaredOversize,
        Reply::Chunked(524_289),
        Reply::Disconnect,
        Reply::Redirect(backup.url.clone()),
        Reply::RateLimited(Some("60")),
    ] {
        let limited = matches!(&mode, Reply::RateLimited(_));
        let server = LocalServer::new(reply(mode)).await;
        let runtime = SharedReadRuntime::new(&[server.url.clone(), backup.url.clone()]).unwrap();
        let error = runtime
            .pump_native_probe(&server.url, packet(&tx, &keys))
            .await
            .err()
            .expect("bad HTTP accepted");
        assert!(!format!("{error:#}").contains("fixture-secret"));
        assert_eq!(server.requests.lock().unwrap().len(), 1);
        assert!(backup.requests.lock().unwrap().is_empty());
        if limited {
            assert!(runtime
                .pump_native_probe(&server.url, packet(&tx, &keys))
                .await
                .is_err());
            assert!(runtime.wsol_quote_slot(&server.url).is_err());
        }
        assert!(runtime.aggregator_simulation_slot(&backup.url).is_ok());
        server.finish().await;
    }
    assert_eq!(backup.connections.load(Ordering::SeqCst), 0);
    backup.finish().await;
}

#[tokio::test]
async fn pump_native_wire_cancellation_and_two_second_budget_release_original_permit() {
    let (tx, keys) = fixture();
    for mode in [Reply::SlowHeaders, Reply::SlowBody] {
        let server = LocalServer::new(reply(mode)).await;
        let runtime = SharedReadRuntime::new(&[server.url.clone()]).unwrap();
        {
            let read = runtime.pump_native_probe(&server.url, packet(&tx, &keys));
            tokio::pin!(read);
            tokio::select! {
                result=&mut read => panic!("held HTTP completed: {}",result.is_ok()),
                entered=tokio::time::timeout(Duration::from_secs(1),server.received.notified()) => entered.unwrap(),
            }
            assert!(runtime.aggregator_simulation_slot(&server.url).is_err());
        }
        assert!(runtime.aggregator_simulation_slot(&server.url).is_ok());
        let started = Instant::now();
        let plan =
            PumpNativeProbePacket::new(&tx, &keys, 175, started + Duration::from_secs(3)).unwrap();
        assert!(runtime.pump_native_probe(&server.url, plan).await.is_err());
        assert!(started.elapsed() >= Duration::from_millis(1800));
        assert!(runtime.aggregator_simulation_slot(&server.url).is_ok());
        assert_eq!(server.requests.lock().unwrap().len(), 2);
        server.finish().await;
    }
}
