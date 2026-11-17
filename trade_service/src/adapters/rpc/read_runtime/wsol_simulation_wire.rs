use super::*;
use crate::sdk_client::wsol_conversion::simulation::{
    self,
    tests::{self as fixture, result},
};
use solana_client::rpc_response::RpcSimulateTransactionResult;
use solana_commitment_config::CommitmentConfig;
fn reply(mode: Reply) -> Reply {
    Reply::WsolSimulation {
        value: result(serde_json::Value::Null),
        mode: Box::new(mode),
    }
}
async fn inspect(
    r: &SharedReadRuntime,
    url: &str,
    signed_mode: bool,
    partial: bool,
) -> Result<RpcSimulateTransactionResult> {
    if signed_mode {
        simulation::simulate_signed(
            &fixture::signed(partial),
            r,
            url,
            CommitmentConfig::confirmed(),
        )
        .await
    } else {
        let p = fixture::unsigned(partial);
        simulation::simulate_unsigned(
            &p,
            &fixture::quote(&p),
            r,
            url,
            CommitmentConfig::confirmed(),
        )
        .await
    }
}
pub(super) async fn success() {
    for signed_mode in [false, true] {
        for partial in [false, true] {
            for mode in [Reply::Success, Reply::AtLimit, Reply::Chunked(524_288)] {
                let server = LocalServer::new(reply(mode)).await;
                let r = runtime(&[&server]);
                let report = inspect(&r, &server.url, signed_mode, partial)
                    .await
                    .unwrap();
                assert_eq!(
                    serde_json::to_value(report).unwrap(),
                    result(serde_json::Value::Null)
                );
                assert_eq!(count(&server), 1);
                assert_eq!(method(&server, 0), "simulateTransaction");
                let bytes = base64::Engine::decode(
                    &base64::engine::general_purpose::STANDARD,
                    server.requests.lock().unwrap()[0]["params"][0]
                        .as_str()
                        .unwrap(),
                )
                .unwrap();
                let tx: solana_sdk::transaction::Transaction =
                    bincode::deserialize(&bytes).unwrap();
                assert_eq!(tx.signatures.len(), if partial { 2 } else { 1 });
                assert!(tx.signatures.iter().all(|s| *s
                    == if signed_mode {
                        solana_sdk::signature::Signature::from([7; 64])
                    } else {
                        solana_sdk::signature::Signature::default()
                    }));
                assert_eq!(r.slots[10].available_permits(), 1);
                server.finish().await;
            }
        }
    }
}
pub(super) async fn errors() {
    let sink = LocalServer::new(Reply::Success).await;
    for signed_mode in [false, true] {
        for mode in [
            Reply::WrongId,
            Reply::InvalidJson,
            Reply::RpcError,
            Reply::ServerError,
            Reply::DeclaredOversize,
            Reply::Chunked(524_289),
            Reply::Disconnect,
            Reply::Redirect(sink.url.clone()),
            Reply::RateLimited(Some("60")),
        ] {
            let limited = matches!(mode, Reply::RateLimited(_));
            let server = LocalServer::new(reply(mode)).await;
            let r = runtime(&[&server, &sink]);
            let error = inspect(&r, &server.url, signed_mode, true)
                .await
                .unwrap_err();
            assert!(!format!("{error:?}").contains("fixture-secret"));
            assert_eq!((count(&server), count(&sink)), (1, 0));
            assert_eq!(r.slots[10].available_permits(), 1);
            if limited {
                assert!(r.aggregator_simulation_slot(&server.url).is_err());
                assert!(r.wsol_quote_slot(&server.url).is_err());
            }
            server.finish().await;
        }
    }
    sink.finish().await;
}
pub(super) async fn timeout_cancel() {
    for signed_mode in [false, true] {
        for mode in [Reply::SlowHeaders, Reply::SlowBody] {
            let server = LocalServer::new(reply(mode)).await;
            let r = runtime(&[&server]);
            assert!(inspect(&r, &server.url, signed_mode, true).await.is_err());
            assert_eq!(count(&server), 1);
            assert_eq!(r.slots[10].available_permits(), 1);
            server.finish().await;
        }
        let server = LocalServer::new(reply(Reply::SlowBody)).await;
        let hash = LocalServer::new(Reply::Success).await;
        let r = runtime(&[&hash, &server]);
        {
            let pending = inspect(&r, &server.url, signed_mode, true);
            tokio::pin!(pending);
            tokio::select! {_=&mut pending=>panic!("slow simulation completed"),_=server.received.notified()=>{}}
            assert!(r.aggregator_simulation_slot(&server.url).is_err());
            assert_eq!(r.latest_blockhash().await.unwrap().to_bytes(), [7; 32]);
        }
        assert_eq!(
            r.slots
                .iter()
                .map(|s| s.available_permits())
                .collect::<Vec<_>>(),
            [1, 1, 1, 1, 1, 1, 1, 2, 1, 1, 1, 1]
        );
        assert_eq!((count(&server), count(&hash)), (1, 1));
        server.finish().await;
        hash.finish().await;
    }
}
