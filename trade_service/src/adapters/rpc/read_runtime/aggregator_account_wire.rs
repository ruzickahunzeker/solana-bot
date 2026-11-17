use super::*;
use crate::{
    adapters::{rpc::read_runtime::aggregator_accounts::tests::ui, sdk::aggregator_accounts},
    domain::trade::Address,
};
use solana_address_lookup_table_interface::{
    program,
    state::{AddressLookupTable, LookupTableMeta},
};
use solana_commitment_config::CommitmentConfig;
use solana_sdk::pubkey::Pubkey;
use std::{borrow::Cow, collections::HashMap};

fn key(n: u8) -> Pubkey {
    Pubkey::new_from_array([n; 32])
}
fn reply(other: Reply) -> Reply {
    let table = AddressLookupTable {
        meta: LookupTableMeta::default(),
        addresses: Cow::Owned(vec![key(4), key(5)]),
    }
    .serialize_for_tests()
    .unwrap();
    Reply::Parameters {
        accounts: HashMap::from([
            (key(1).to_string(), ui(spl_token_interface::ID, &[7, 8, 9])),
            (key(2).to_string(), ui(program::id(), &table)),
        ]),
        epoch: None,
        other: Box::new(other),
    }
}
pub(super) async fn success() {
    for mode in [Reply::Success, Reply::Chunked(65_536)] {
        let server = LocalServer::new(reply(mode)).await;
        let r = runtime(&[&server]);
        let (ata, token_program) = aggregator_accounts::target_ata(
            &r,
            &server.url,
            CommitmentConfig::confirmed(),
            key(9),
            key(1),
        )
        .await
        .unwrap();
        assert_eq!(token_program, spl_token_interface::ID);
        assert_eq!(
            ata,
            sol_trade_sdk::common::fast_fn::get_associated_token_address_with_program_id_fast(
                &key(9),
                &key(1),
                &token_program
            )
        );
        assert_eq!(
            aggregator_accounts::lookup_table(
                &r,
                &server.url,
                CommitmentConfig::finalized(),
                key(2)
            )
            .await
            .unwrap(),
            vec![key(4), key(5)]
        );
        let calls = server.requests.lock().unwrap();
        assert_eq!(calls.len(), 2);
        for (index, commitment) in [(0, "confirmed"), (1, "finalized")] {
            assert_eq!(calls[index]["method"], "getAccountInfo");
            assert_eq!(calls[index]["params"][0], key(index as u8 + 1).to_string());
            assert_eq!(
                calls[index]["params"][1],
                serde_json::json!({"encoding":"base64","commitment":commitment})
            );
        }
        drop(calls);
        assert_eq!(r.slots[9].available_permits(), 1);
        server.finish().await;
    }
}
pub(super) async fn errors() {
    let sink = LocalServer::new(reply(Reply::Success)).await;
    for mode in [
        Reply::WrongId,
        Reply::InvalidJson,
        Reply::RpcError,
        Reply::ServerError,
        Reply::Disconnect,
        Reply::DeclaredOversize,
        Reply::Chunked(65_537),
        Reply::AtLimit,
        Reply::Redirect(sink.url.clone()),
    ] {
        let server = LocalServer::new(reply(mode)).await;
        let r = runtime(&[&server, &sink]);
        let error = aggregator_accounts::target_ata(
            &r,
            &server.url,
            CommitmentConfig::processed(),
            key(9),
            key(1),
        )
        .await
        .unwrap_err();
        assert!(!format!("{error:?}").contains("fixture-secret"));
        assert_eq!((count(&server), count(&sink)), (1, 0));
        assert_eq!(r.slots[9].available_permits(), 1);
        server.finish().await;
    }
    let limited = LocalServer::new(reply(Reply::RateLimited(Some("60")))).await;
    let r = runtime(&[&limited, &sink]);
    assert!(r
        .aggregator_account(
            &limited.url,
            Address::from_bytes([1; 32]),
            CommitmentConfig::processed()
        )
        .await
        .is_err());
    assert!(r
        .aggregator_account(
            &limited.url,
            Address::from_bytes([2; 32]),
            CommitmentConfig::processed()
        )
        .await
        .is_err());
    r.latest_blockhash().await.unwrap();
    assert_eq!((count(&limited), count(&sink)), (1, 1));
    assert_eq!(method(&sink, 0), "getLatestBlockhash");
    limited.finish().await;
    sink.finish().await;
}
pub(super) async fn timeout_cancel() {
    for mode in [Reply::SlowHeaders, Reply::SlowBody] {
        let server = LocalServer::new(reply(mode)).await;
        let r = runtime(&[&server]);
        let started = Instant::now();
        assert!(r
            .aggregator_account(
                &server.url,
                Address::from_bytes([1; 32]),
                CommitmentConfig::processed()
            )
            .await
            .is_err());
        assert!(started.elapsed() < Duration::from_secs(3));
        assert_eq!(count(&server), 1);
        assert_eq!(r.slots[9].available_permits(), 1);
        server.finish().await;
    }
    let slow = LocalServer::new(reply(Reply::SlowHeaders)).await;
    let backup = LocalServer::new(Reply::Success).await;
    let r = runtime(&[&slow, &backup]);
    let task = {
        let r = r.clone();
        let url = slow.url.clone();
        tokio::spawn(async move {
            r.aggregator_account(
                &url,
                Address::from_bytes([1; 32]),
                CommitmentConfig::processed(),
            )
            .await
        })
    };
    tokio::time::timeout(Duration::from_secs(1), slow.received.notified())
        .await
        .unwrap();
    assert_eq!(r.slots[9].available_permits(), 0);
    r.latest_blockhash().await.unwrap();
    assert!(!task.is_finished());
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert_eq!(r.slots[9].available_permits(), 1);
    assert_eq!((count(&slow), count(&backup)), (2, 1));
    slow.finish().await;
    backup.finish().await;
}
