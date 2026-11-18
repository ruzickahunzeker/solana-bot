use super::*;
use crate::adapters::test_support::read_rpc_server::{LocalServer, Reply};
use std::sync::Arc;

fn reply(tip: bool, mode: Reply) -> Reply {
    Reply::PumpNativeFinal {
        result: json!({"context":{"slot":176},"value":{"err":null,"accounts":vec![Value::Null;if tip {12}else{11}]}}),
        mode: Box::new(mode),
    }
}

#[tokio::test]
async fn pump_final_wire_binds_complete_packets_and_accounts_through_real_bounded_http() {
    for buy in [false, true] {
        for tip in [false, true] {
            for mode in [Reply::Success, Reply::AtLimit, Reply::Chunked(524_288)] {
                let server = LocalServer::new(reply(tip, mode)).await;
                let runtime = SharedReadRuntime::new(&[server.url.clone()]).unwrap();
                let (tx, keys) = fixture(buy, tip);
                let plan = packet(&tx, &keys);
                let expected = plan.body(1);
                let result = runtime
                    .pump_native_final(&server.url, plan)
                    .await
                    .unwrap()
                    .for_template(&tx, &keys, 176)
                    .unwrap();
                assert_eq!(result["context"]["slot"], 176);
                assert_eq!(*server.requests.lock().unwrap(), [expected]);
                assert!(runtime.aggregator_simulation_slot(&server.url).is_ok());
                server.finish().await;
            }
        }
    }
    for damage in 0..4 {
        let server = LocalServer::new(reply(true, Reply::Success)).await;
        let runtime = SharedReadRuntime::new(&[server.url.clone()]).unwrap();
        let (mut tx, mut keys) = fixture(true, true);
        let result = runtime
            .pump_native_final(&server.url, packet(&tx, &keys))
            .await
            .unwrap();
        let mut minimum = 176;
        match damage {
            0 => {
                let VersionedMessage::Legacy(message) = &mut tx.message else {
                    unreachable!()
                };
                message.recent_blockhash = Hash::new_from_array([11; 32]);
            }
            1 => keys.swap(0, 1),
            2 => minimum = 177,
            _ => tokio::time::sleep(Duration::from_millis(3_050)).await,
        }
        assert!(result.for_template(&tx, &keys, minimum).is_err());
        assert_eq!(server.requests.lock().unwrap().len(), 1);
        server.finish().await;
    }
}

#[tokio::test]
async fn pump_final_wire_rejects_overflow_redirect_and_errors_with_cross_profile_429_cooldown() {
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
        let server = LocalServer::new(reply(true, mode)).await;
        let runtime = SharedReadRuntime::new(&[server.url.clone(), backup.url.clone()]).unwrap();
        let (tx, keys) = fixture(true, true);
        let error = runtime
            .pump_native_final(&server.url, packet(&tx, &keys))
            .await
            .err()
            .expect("bad HTTP accepted");
        assert!(!format!("{error:#}").contains("fixture-secret"));
        assert_eq!(server.requests.lock().unwrap().len(), 1);
        assert!(backup.requests.lock().unwrap().is_empty());
        if limited {
            assert!(runtime
                .pump_native_final(&server.url, packet(&tx, &keys))
                .await
                .is_err());
            assert!(runtime.wsol_quote_slot(&server.url).is_err());
            assert_eq!(server.requests.lock().unwrap().len(), 1);
        }
        assert!(runtime.aggregator_simulation_slot(&backup.url).is_ok());
        server.finish().await;
    }
    assert_eq!(backup.connections.load(Ordering::SeqCst), 0);
    backup.finish().await;
}

#[tokio::test]
async fn pump_final_wire_preserves_two_second_limit_original_expiry_cancel_and_pacing() {
    for cancel in [false, true] {
        let server = LocalServer::new(reply(true, Reply::SlowHeaders)).await;
        let runtime = Arc::new(SharedReadRuntime::new(&[server.url.clone()]).unwrap());
        let (tx, keys) = fixture(true, true);
        let plan = packet(&tx, &keys);
        let started = Instant::now();
        let task = {
            let runtime = runtime.clone();
            let url = server.url.clone();
            tokio::spawn(async move { runtime.pump_native_final(&url, plan).await })
        };
        server.received.notified().await;
        if cancel {
            task.abort();
            assert!(matches!(task.await, Err(error) if error.is_cancelled()));
        } else {
            assert!(task.await.unwrap().is_err());
            assert!(started.elapsed() >= Duration::from_millis(1_500));
            assert!(started.elapsed() < Duration::from_secs(3));
        }
        assert!(runtime.aggregator_simulation_slot(&server.url).is_ok());
        server.finish().await;
    }
    let server = LocalServer::new(reply(true, Reply::SlowHeaders)).await;
    let runtime = SharedReadRuntime::new(&[server.url.clone()]).unwrap();
    let (tx, keys) = fixture(true, true);
    let started = Instant::now();
    let plan =
        PumpNativeFinalPacket::new(&tx, &keys, 176, started + Duration::from_millis(80)).unwrap();
    assert!(runtime.pump_native_final(&server.url, plan).await.is_err());
    assert!(started.elapsed() < Duration::from_secs(1));
    assert!(runtime.aggregator_simulation_slot(&server.url).is_ok());
    server.finish().await;

    let server = LocalServer::new(reply(true, Reply::Success)).await;
    let runtime = SharedReadRuntime::new(&[server.url.clone()]).unwrap();
    let first = packet(&tx, &keys);
    let second = packet(&tx, &keys);
    let started = Instant::now();
    runtime
        .pump_native_final(&server.url, first)
        .await
        .unwrap()
        .for_template(&tx, &keys, 176)
        .unwrap();
    runtime
        .pump_native_final(&server.url, second)
        .await
        .unwrap()
        .for_template(&tx, &keys, 176)
        .unwrap();
    assert!(started.elapsed() >= Duration::from_millis(80));
    let mut ids: Vec<_> = server
        .requests
        .lock()
        .unwrap()
        .iter()
        .map(|request| request["id"].as_u64().unwrap())
        .collect();
    ids.sort();
    assert_eq!(ids, [1, 2]);
    server.finish().await;
}
