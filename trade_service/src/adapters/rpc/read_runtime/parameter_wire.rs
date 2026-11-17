use super::*;
use crate::{
    adapters::rpc::parameter_reads::{ParameterReadScope, ParameterReadSession},
    domain::trade::{Address, Pool},
};
use solana_commitment_config::CommitmentConfig;
use solana_sdk::pubkey::Pubkey;

fn key(n: u8) -> Pubkey {
    Pubkey::new_from_array([n; 32])
}
fn reply(other: Reply) -> Reply {
    use base64::Engine;
    let account = serde_json::json!({"lamports":1000,"owner":key(8).to_string(),"data":[base64::engine::general_purpose::STANDARD.encode([1,2,3]),"base64"],"space":3,"executable":false,"rentEpoch":0});
    Reply::Parameters {
        accounts: std::collections::HashMap::from([
            (key(1).to_string(), account.clone()),
            (key(2).to_string(), account),
        ]),
        epoch: Some(17),
        other: Box::new(other),
    }
}
fn session(r: Arc<SharedReadRuntime>, url: String) -> ParameterReadSession {
    ParameterReadSession::new(
        r,
        url,
        CommitmentConfig::processed(),
        ParameterReadScope::new(
            Pool::MeteoraDammV2,
            Address::from_bytes([1; 32]),
            Some(Address::from_bytes([2; 32])),
        ),
    )
}
pub(super) async fn success() {
    for mode in [Reply::Success, Reply::AtLimit, Reply::Chunked(524_288)] {
        let server = LocalServer::new(reply(mode)).await;
        let r = runtime(&[&server]);
        let s = session(r.clone(), server.url.clone());
        assert_eq!(s.rpc().get_account(&key(1)).await.unwrap().data, [1, 2, 3]);
        assert_eq!(
            s.rpc()
                .get_multiple_accounts(&[key(1), key(2)])
                .await
                .unwrap()
                .len(),
            2
        );
        assert_eq!(s.rpc().get_epoch_info().await.unwrap().epoch, 17);
        s.finish(Ok(())).unwrap();
        assert_eq!(count(&server), 3);
        assert_eq!(method(&server, 0), "getAccountInfo");
        assert_eq!(method(&server, 1), "getMultipleAccounts");
        assert_eq!(method(&server, 2), "getEpochInfo");
        assert!(r.parameter_slot().is_ok());
        server.finish().await;
    }
}
pub(super) async fn errors() {
    for mode in [
        Reply::WrongId,
        Reply::InvalidJson,
        Reply::RpcError,
        Reply::ServerError,
        Reply::Disconnect,
        Reply::DeclaredOversize,
        Reply::Chunked(524_289),
    ] {
        let server = LocalServer::new(reply(mode)).await;
        let backup = LocalServer::new(reply(Reply::Success)).await;
        let r = runtime(&[&server, &backup]);
        let s = session(r.clone(), server.url.clone());
        assert!(s.rpc().get_account(&key(1)).await.is_err());
        let error = s.finish(Ok("ignored SDK optional failure")).err().unwrap();
        assert!(!format!("{error:?}").contains("fixture-secret"));
        assert_eq!((count(&server), count(&backup)), (1, 0));
        assert!(r.parameter_slot().is_ok());
        server.finish().await;
        backup.finish().await;
    }
}
pub(super) async fn timeouts() {
    for mode in [Reply::SlowHeaders, Reply::SlowBody] {
        let server = LocalServer::new(reply(mode)).await;
        let r = runtime(&[&server]);
        let s = session(r.clone(), server.url.clone());
        let started = Instant::now();
        assert!(s.rpc().get_account(&key(1)).await.is_err());
        assert!(s.finish(Ok(())).is_err());
        assert!(started.elapsed() < Duration::from_secs(3));
        assert_eq!(count(&server), 1);
        assert!(r.parameter_slot().is_ok());
        server.finish().await;
    }
    let sink = LocalServer::new(reply(Reply::Success)).await;
    let redirect = LocalServer::new(reply(Reply::Redirect(sink.url.clone()))).await;
    let s = session(runtime(&[&redirect, &sink]), redirect.url.clone());
    assert!(s.rpc().get_account(&key(1)).await.is_err());
    assert!(s.finish(Ok(())).is_err());
    assert_eq!((count(&redirect), count(&sink)), (1, 0));
    redirect.finish().await;
    sink.finish().await;
}
pub(super) async fn rate_cancel() {
    let limited = LocalServer::new(reply(Reply::RateLimited(Some("60")))).await;
    let backup = LocalServer::new(Reply::Success).await;
    let r = runtime(&[&limited, &backup]);
    let s = session(r.clone(), limited.url.clone());
    assert!(s.rpc().get_account(&key(1)).await.is_err());
    assert!(s.finish(Ok(())).is_err());
    assert!(r
        .preparation_account_exists(
            &limited.url,
            Address::from_bytes([1; 32]),
            Address::from_bytes([2; 32])
        )
        .await
        .is_err());
    r.latest_blockhash().await.unwrap();
    assert_eq!((count(&limited), count(&backup)), (1, 1));
    limited.finish().await;
    backup.finish().await;
    let slow = LocalServer::new(reply(Reply::SlowHeaders)).await;
    let backup = LocalServer::new(Reply::Success).await;
    let r = runtime(&[&slow, &backup]);
    let s = session(r.clone(), slow.url.clone());
    let task = tokio::spawn(async move {
        let value = s.rpc().get_account(&key(1)).await;
        s.finish(value.map_err(anyhow::Error::from))
    });
    tokio::time::timeout(Duration::from_secs(1), slow.received.notified())
        .await
        .unwrap();
    assert!(r.parameter_slot().is_err());
    r.latest_blockhash().await.unwrap();
    assert!(!task.is_finished());
    task.abort();
    assert!(task.await.err().unwrap().is_cancelled());
    assert!(r.parameter_slot().is_ok());
    assert_eq!((count(&slow), count(&backup)), (2, 1));
    slow.finish().await;
    backup.finish().await;
}

pub(super) async fn discovery_success() {
    use crate::adapters::rpc::discovery_reads::{DiscoveryProtocol, DiscoveryReadSession};
    let server = LocalServer::new(reply(Reply::Success)).await;
    let r = runtime(&[&server]);
    let s = DiscoveryReadSession::new(
        r.clone(),
        server.url.clone(),
        CommitmentConfig::processed(),
        DiscoveryProtocol::Pump,
        Address::from_bytes([1; 32]),
        Instant::now() + Duration::from_secs(3),
    );
    assert_eq!(s.rpc().get_account(&key(1)).await.unwrap().data, [1, 2, 3]);
    s.finish(Ok(())).unwrap();
    let s = DiscoveryReadSession::new(
        r.clone(),
        server.url.clone(),
        CommitmentConfig::processed(),
        DiscoveryProtocol::Dbc,
        Address::from_bytes([1; 32]),
        Instant::now() + Duration::from_secs(3),
    );
    let result =
        sol_trade_sdk::instruction::utils::meteora_dbc::find_pool_by_base_mint(s.rpc(), &key(1))
            .await;
    let error = s.finish(result).err().unwrap();
    assert!(error
        .downcast_ref::<crate::domain::parameter_read::ParameterReadFailure>()
        .is_none());
    assert_eq!(count(&server), 2);
    assert_eq!(method(&server, 1), "getProgramAccounts");
    assert_eq!(
        server.requests.lock().unwrap()[1]["params"][1]["filters"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    let a = r.discovery_slot().unwrap();
    let b = r.discovery_slot().unwrap();
    drop((a, b));
    assert!(r.parameter_slot().is_ok());
    server.finish().await;
}

pub(super) async fn discovery_errors() {
    use crate::adapters::rpc::discovery_reads::{DiscoveryProtocol, DiscoveryReadSession};
    let sink = LocalServer::new(reply(Reply::Success)).await;
    for mode in [
        Reply::WrongId,
        Reply::InvalidJson,
        Reply::DeclaredOversize,
        Reply::Chunked(1_048_577),
        Reply::RateLimited(Some("60")),
        Reply::Redirect(sink.url.clone()),
    ] {
        let server = LocalServer::new(reply(mode)).await;
        let r = runtime(&[&server, &sink]);
        let s = DiscoveryReadSession::new(
            r.clone(),
            server.url.clone(),
            CommitmentConfig::processed(),
            DiscoveryProtocol::Dbc,
            Address::from_bytes([1; 32]),
            Instant::now() + Duration::from_secs(3),
        );
        let result = sol_trade_sdk::instruction::utils::meteora_dbc::find_pool_by_base_mint(
            s.rpc(),
            &key(1),
        )
        .await;
        let error = s.finish(result).err().unwrap();
        assert!(error
            .downcast_ref::<crate::domain::parameter_read::ParameterReadFailure>()
            .is_some());
        assert!(!format!("{error:?}").contains("fixture-secret"));
        assert_eq!((count(&server), count(&sink)), (1, 0));
        let a = r.discovery_slot().unwrap();
        let b = r.discovery_slot().unwrap();
        drop((a, b));
        server.finish().await;
    }
    sink.finish().await;
}

fn hop_scan(reverse: bool) -> solana_client::rpc_config::RpcProgramAccountsConfig {
    use sol_trade_sdk::instruction::utils::raydium_cpmm_types::POOL_STATE_DISCRIMINATOR;
    use solana_client::{
        rpc_config::{RpcAccountInfoConfig, RpcProgramAccountsConfig},
        rpc_filter::{Memcmp, RpcFilterType},
    };
    let (a, b) = if reverse { (200, 168) } else { (168, 200) };
    RpcProgramAccountsConfig {
        filters: Some(vec![
            RpcFilterType::Memcmp(Memcmp::new_base58_encoded(0, &POOL_STATE_DISCRIMINATOR)),
            RpcFilterType::Memcmp(Memcmp::new_base58_encoded(a, key(1).as_ref())),
            RpcFilterType::Memcmp(Memcmp::new_base58_encoded(
                b,
                sol_trade_sdk::constants::WSOL_TOKEN_ACCOUNT.as_ref(),
            )),
        ]),
        account_config: RpcAccountInfoConfig {
            encoding: Some(solana_account_decoder::UiAccountEncoding::Base64),
            ..Default::default()
        },
        with_context: None,
        sort_results: None,
    }
}
pub(super) async fn sol_hop_success() {
    use crate::adapters::rpc::sol_hop_reads::SolHopReadSession;
    let server = LocalServer::new(reply(Reply::Success)).await;
    let r = runtime(&[&server]);
    let s = SolHopReadSession::new(
        r.clone(),
        server.url.clone(),
        CommitmentConfig::processed(),
        Address::from_bytes([1; 32]),
        Instant::now() + Duration::from_secs(5),
    );
    for reverse in [false, true] {
        #[allow(deprecated)]
        let rows = s
            .rpc()
            .get_program_ui_accounts_with_config(
                &sol_trade_sdk::instruction::utils::raydium_cpmm::accounts::RAYDIUM_CPMM,
                hop_scan(reverse),
            )
            .await
            .unwrap();
        assert!(rows.is_empty());
    }
    assert_eq!(
        s.rpc()
            .get_token_account_balance(&key(4))
            .await
            .unwrap()
            .amount,
        "23"
    );
    s.finish(Ok(())).unwrap();
    assert_eq!(count(&server), 3);
    assert_eq!(method(&server, 2), "getTokenAccountBalance");
    assert!(r.sol_hop_slot().is_ok());
    server.finish().await;
}
pub(super) async fn sol_hop_errors_cancel() {
    use crate::adapters::rpc::sol_hop_reads::SolHopReadSession;
    let sink = LocalServer::new(reply(Reply::Success)).await;
    for balance in [false, true] {
        let max = if balance { 524_288 } else { 1_048_576 };
        for mode in [
            Reply::WrongId,
            Reply::RpcError,
            Reply::DeclaredOversize,
            Reply::Chunked(max + 1),
            Reply::RateLimited(Some("60")),
            Reply::Redirect(sink.url.clone()),
        ] {
            let server = LocalServer::new(reply(mode)).await;
            let r = runtime(&[&server, &sink]);
            let s = SolHopReadSession::new(
                r.clone(),
                server.url.clone(),
                CommitmentConfig::processed(),
                Address::from_bytes([1; 32]),
                Instant::now() + Duration::from_secs(5),
            );
            let result = if balance {
                s.rpc()
                    .get_token_account_balance(&key(4))
                    .await
                    .map(|_| ())
                    .map_err(anyhow::Error::from)
            } else {
                #[allow(deprecated)]
                let result = s
                    .rpc()
                    .get_program_ui_accounts_with_config(
                        &sol_trade_sdk::instruction::utils::raydium_cpmm::accounts::RAYDIUM_CPMM,
                        hop_scan(false),
                    )
                    .await
                    .map(|_| ())
                    .map_err(anyhow::Error::from);
                result
            };
            assert!(s.finish(result).is_err());
            assert_eq!((count(&server), count(&sink)), (1, 0));
            assert!(r.sol_hop_slot().is_ok());
            server.finish().await;
        }
    }
    sink.finish().await;
    let slow = LocalServer::new(reply(Reply::SlowHeaders)).await;
    let backup = LocalServer::new(Reply::Success).await;
    let r = runtime(&[&slow, &backup]);
    let s = SolHopReadSession::new(
        r.clone(),
        slow.url.clone(),
        CommitmentConfig::processed(),
        Address::from_bytes([1; 32]),
        Instant::now() + Duration::from_secs(5),
    );
    let task = tokio::spawn(async move {
        let result = s.rpc().get_token_account_balance(&key(4)).await;
        s.finish(result.map_err(anyhow::Error::from))
    });
    tokio::time::timeout(Duration::from_secs(1), slow.received.notified())
        .await
        .unwrap();
    assert!(r.sol_hop_slot().is_err());
    r.latest_blockhash().await.unwrap();
    assert!(!task.is_finished());
    task.abort();
    assert!(task.await.err().unwrap().is_cancelled());
    assert!(r.sol_hop_slot().is_ok());
    assert_eq!((count(&slow), count(&backup)), (2, 1));
    slow.finish().await;
    backup.finish().await;
}
