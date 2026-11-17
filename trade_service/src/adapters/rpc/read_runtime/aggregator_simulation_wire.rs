use super::*;
use crate::adapters::{
    rpc::aggregator_simulation::tests::{fixture, Fixture},
    sdk::aggregator_simulation::{self, AggregatorSimulation},
};
fn reply(f: &Fixture, baseline: Reply, simulation: Reply) -> Reply {
    Reply::AggregatorSimulation {
        baseline: Box::new(baseline),
        simulation: Box::new(simulation),
        baseline_value: f.baseline(),
        simulation_value: f.simulation(),
    }
}
async fn inspect(r: &SharedReadRuntime, url: &str, f: &Fixture) -> Result<AggregatorSimulation> {
    aggregator_simulation::simulate(
        r,
        url,
        f.wallet,
        &f.transaction,
        f.target_index,
        f.mint,
        f.target,
        f.program,
        &f.keys,
    )
    .await
}
pub(super) async fn success() {
    let f = fixture(spl_token_interface::ID, true);
    for mode in [Reply::Success, Reply::AtLimit, Reply::Chunked(524_288)] {
        let server = LocalServer::new(reply(&f, mode.clone(), mode)).await;
        let r = runtime(&[&server]);
        let report = inspect(&r, &server.url, &f).await.unwrap();
        assert!(report.ok);
        assert_eq!(report.target_token_delta_units, Some(20));
        let calls = server.requests.lock().unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0]["method"], "getMultipleAccounts");
        assert_eq!(calls[1]["method"], "simulateTransaction");
        assert_eq!(
            calls[0]["params"][0],
            serde_json::json!([f.wallet.to_string(), f.target.to_string()])
        );
        assert_eq!(calls[1]["params"][1]["minContextSlot"], 42);
        let packet = base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            calls[1]["params"][0].as_str().unwrap(),
        )
        .unwrap();
        assert_eq!(packet, bincode::serialize(&f.transaction).unwrap());
        drop(calls);
        assert_eq!(r.slots[10].available_permits(), 1);
        server.finish().await;
    }
}
pub(super) async fn errors() {
    let f = fixture(spl_token_interface::ID, false);
    let sink = LocalServer::new(Reply::Success).await;
    for stage in [false, true] {
        for mode in [
            Reply::WrongId,
            Reply::InvalidJson,
            Reply::RpcError,
            Reply::DeclaredOversize,
            Reply::Chunked(524_289),
            Reply::Redirect(sink.url.clone()),
        ] {
            let (baseline, simulation) = if stage {
                (Reply::Success, mode)
            } else {
                (mode, Reply::Success)
            };
            let server = LocalServer::new(reply(&f, baseline, simulation)).await;
            let r = runtime(&[&server, &sink]);
            let error = inspect(&r, &server.url, &f).await.err().unwrap();
            assert!(!format!("{error:?}").contains("fixture-secret"));
            assert_eq!((count(&server), count(&sink)), (usize::from(stage) + 1, 0));
            assert_eq!(r.slots[10].available_permits(), 1);
            server.finish().await;
        }
    }
    for stage in [false, true] {
        let mode = Reply::RateLimited(Some("60"));
        let (baseline, simulation) = if stage {
            (Reply::Success, mode)
        } else {
            (mode, Reply::Success)
        };
        let server = LocalServer::new(reply(&f, baseline, simulation)).await;
        let r = runtime(&[&server, &sink]);
        assert!(inspect(&r, &server.url, &f).await.is_err());
        r.latest_blockhash().await.unwrap();
        assert_eq!(count(&server), usize::from(stage) + 1);
        server.finish().await;
    }
    assert_eq!(count(&sink), 2);
    sink.finish().await;
}
pub(super) async fn timeout_cancel() {
    let f = fixture(spl_token_interface::ID, false);
    for stage in [false, true] {
        let (baseline, simulation) = if stage {
            (Reply::Success, Reply::SlowBody)
        } else {
            (Reply::SlowHeaders, Reply::Success)
        };
        let server = LocalServer::new(reply(&f, baseline, simulation)).await;
        let r = runtime(&[&server]);
        assert!(inspect(&r, &server.url, &f).await.is_err());
        assert_eq!(count(&server), usize::from(stage) + 1);
        assert_eq!(r.slots[10].available_permits(), 1);
        server.finish().await;
    }
    for stage in [false, true] {
        let (baseline, simulation) = if stage {
            (Reply::Success, Reply::SlowHeaders)
        } else {
            (Reply::SlowHeaders, Reply::Success)
        };
        let slow = LocalServer::new(reply(&f, baseline, simulation)).await;
        let r = runtime(&[&slow]);
        let task = {
            let r = r.clone();
            let url = slow.url.clone();
            let f = f.clone();
            tokio::spawn(async move { inspect(&r, &url, &f).await })
        };
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if count(&slow) == usize::from(stage) + 1 {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(r.slots[10].available_permits(), 0);
        r.latest_blockhash().await.unwrap();
        task.abort();
        assert!(task.await.err().unwrap().is_cancelled());
        assert_eq!(r.slots[10].available_permits(), 1);
        assert_eq!(count(&slow), usize::from(stage) + 2);
        slow.finish().await;
    }
}
