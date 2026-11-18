use super::super::tests::observed;
use super::*;
use crate::adapters::test_support::read_rpc_server::{LocalServer, Reply};
use serde_json::{json, Value};
use sol_trade_sdk::swqos::TradeType;
use std::time::{Duration, Instant};

fn reply(mut result: Value) -> Reply {
    result["value"]["accounts"] = json!(vec![Value::Null; EVIDENCE_ACCOUNTS]);
    Reply::PumpNativeProbe {
        result,
        mode: Box::new(Reply::Success),
    }
}

#[tokio::test]
async fn pump_native_adapter_binds_actual_sdk_packet_bank_roles_and_same_response_output() {
    for side in [TradeType::Buy, TradeType::Sell] {
        let (probe, response) = observed(side).await;
        let frozen = bincode::serialize(probe.packet().unwrap()).unwrap();
        let keys = probe.evidence_accounts().map(|key| key.to_string());
        let server = LocalServer::new(reply(response)).await;
        let runtime = SharedReadRuntime::new(&[server.url.clone()]).unwrap();
        let observation = probe
            .shared_observation(&runtime, &server.url)
            .await
            .unwrap();
        assert_eq!(
            observation.output.get(),
            if matches!(side, TradeType::Buy) {
                251
            } else {
                500_000
            }
        );
        assert_eq!(observation.simulation_slot, 176);
        assert_eq!(
            bincode::serialize(observation.probe.packet().unwrap()).unwrap(),
            frozen
        );
        let expected = json!({"jsonrpc":"2.0","id":1,"method":"simulateTransaction","params":[
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &frozen),
            {"sigVerify":false,"replaceRecentBlockhash":false,"commitment":"processed","encoding":"base64",
                "accounts":{"encoding":"base64","addresses":keys},"minContextSlot":175,"innerInstructions":true}]});
        assert_eq!(*server.requests.lock().unwrap(), [expected]);
        assert!(runtime.aggregator_simulation_slot(&server.url).is_ok());
        server.finish().await;
    }
}

#[tokio::test]
async fn pump_native_adapter_failed_rpc_or_incomplete_evidence_never_becomes_an_observation() {
    for damage in 0..4 {
        let (probe, mut response) = observed(TradeType::Buy).await;
        match damage {
            0 => {
                response["value"].as_object_mut().unwrap().remove("fee");
            }
            1 => {
                response["value"]["postTokenBalances"][0]["mint"] =
                    json!(Pubkey::new_from_array([77; 32]).to_string())
            }
            2 => response["context"]["slot"] = json!(174),
            _ => {
                response["value"].as_object_mut().unwrap().remove("err");
            }
        }
        let server = LocalServer::new(reply(response)).await;
        let runtime = SharedReadRuntime::new(&[server.url.clone()]).unwrap();
        assert!(
            probe
                .shared_observation(&runtime, &server.url)
                .await
                .is_err(),
            "damage {damage}"
        );
        assert_eq!(server.requests.lock().unwrap().len(), 1);
        assert!(runtime.aggregator_simulation_slot(&server.url).is_ok());
        server.finish().await;
    }
}

#[tokio::test]
async fn pump_native_adapter_expired_snapshot_or_token_extension_is_rejected_before_io() {
    for extended in [false, true] {
        let (mut probe, response) = observed(TradeType::Buy).await;
        if extended {
            probe.funding.curve.base_mint_layout.program = spl_token_2022_interface::ID;
        } else {
            probe.funding.started =
                Instant::now() - super::super::super::MAX_BALANCE_AGE - Duration::from_secs(1);
        }
        let server = LocalServer::new(reply(response)).await;
        let runtime = SharedReadRuntime::new(&[server.url.clone()]).unwrap();
        assert!(probe
            .shared_observation(&runtime, &server.url)
            .await
            .is_err());
        assert!(server.requests.lock().unwrap().is_empty());
        assert!(runtime.aggregator_simulation_slot(&server.url).is_ok());
        server.finish().await;
    }
}
