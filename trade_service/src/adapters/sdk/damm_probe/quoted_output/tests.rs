use super::*;
use crate::adapters::sdk::damm_probe::evidence::{check_snapshot, tests::Fixture};
use crate::domain::{
    preparation_balance::PreparationBalanceFailure, sdk_candidate::DammCandidateDetails,
};
use solana_account_decoder::UiAccount;
use std::time::Duration;

#[tokio::test]
async fn damm_output_evidence_rejects_a_bank_before_the_balance_before_simulating() {
    let source = Fixture::default();
    let prepared = source.prepare();
    let rpc = source.rpc.client();
    let error = super::super::inspect_prepared_damm_probe_at_slot(&rpc, &prepared, 43)
        .await
        .err()
        .unwrap();
    assert!(error.to_string().contains("precedes"));
    assert!(!error
        .downcast_ref::<PreparationBalanceFailure>()
        .unwrap()
        .allows_lane_fallback());
    assert_eq!(
        source.rpc.methods(),
        [solana_client::rpc_request::RpcRequest::GetMultipleAccounts]
    );
    assert_eq!(
        source.rpc.calls.lock().unwrap()[0].1[1]["minContextSlot"],
        43
    );
}

#[tokio::test]
async fn damm_output_evidence_missing_balance_slot_never_enters_rpc() {
    let source = Fixture::default();
    let prepared = source.prepare();
    let rpc = source.rpc.client();
    let error = super::super::inspect_prepared_damm_probe_at_slot(&rpc, &prepared, 0)
        .await
        .err()
        .unwrap();
    assert!(!error
        .downcast_ref::<PreparationBalanceFailure>()
        .unwrap()
        .allows_lane_fallback());
    assert!(source.rpc.methods().is_empty());
}

fn fixture() -> (DammOutputEvidence, DammCandidateDetails) {
    let source = Fixture::default();
    let prepared = source.prepare();
    let values: Vec<Option<UiAccount>> =
        serde_json::from_value(serde_json::json!(source.rpc.values)).unwrap();
    let snapshot = check_snapshot(&prepared, 42, &values, false).unwrap();
    let evidence =
        DammOutputEvidence::checked(&prepared, &snapshot, 915, 42, 43, Instant::now()).unwrap();
    let intent = prepared.accounts();
    let details = DammCandidateDetails {
        wallet: intent.wallet.to_bytes(),
        mint: intent.input_mint.to_bytes(),
        quote_mint: intent.output_mint.to_bytes(),
        pool_hint: Some(intent.pool.pool.to_bytes()),
        pool: pool_snapshot(&intent.pool),
        balance_units: 1000,
        balance_slot: 42,
        input_account: intent.input_ata.to_bytes(),
        input_program: snapshot.input.program.to_bytes(),
        input_decimals: snapshot.input.decimals,
        input_units: 100,
        sell_percent: 10,
        minimum_output_units: 777,
        slippage_bps: 1500,
        tip_sol_bits: 0.0_f64.to_bits(),
        relay: "default".into(),
        tip_destination: None,
        recent_blockhash: prepared.transaction().message.recent_blockhash.to_bytes(),
        simulate: false,
        use_mev: Some(false),
        requested_mev_provider: None,
        wait_tx_confirmed: false,
        mev_submit_verify_ms: 0,
    };
    (evidence, details)
}

#[test]
fn damm_output_evidence_binds_context_without_restarting_its_age() {
    let (evidence, details) = fixture();
    let started = evidence.started;
    for _ in 0..3 {
        evidence.validate_details(&details).unwrap();
        assert_eq!(evidence.started, started);
        assert_eq!(evidence.output_units(), 915);
    }
}

#[test]
fn damm_output_evidence_rejects_all_identity_amount_slot_and_minimum_mutations() {
    let (evidence, details) = fixture();
    for change in 0..22 {
        let mut d = details.clone();
        match change {
            0 => d.wallet = [30; 32],
            1 => d.mint = [30; 32],
            2 => d.quote_mint = [30; 32],
            3 => d.pool.pool = [30; 32],
            4 => d.pool.token_a_vault = [30; 32],
            5 => d.pool.token_b_vault = [30; 32],
            6 => d.pool.token_a_mint = [30; 32],
            7 => d.pool.token_b_mint = [30; 32],
            8 => d.pool.token_a_program = [30; 32],
            9 => d.pool.token_b_program = [30; 32],
            10 => d.pool.referral = Some([30; 32]),
            11 => d.pool.rate_limiter_sysvar = !d.pool.rate_limiter_sysvar,
            12 => d.input_account = [30; 32],
            13 => d.input_program = [30; 32],
            14 => d.input_decimals += 1,
            15 => d.input_units += 1,
            16 => d.recent_blockhash = [30; 32],
            17 => d.pool_hint = Some([30; 32]),
            18 => d.balance_slot = 43,
            19 => d.balance_units += 100,
            20 => d.sell_percent = 0,
            21 => d.minimum_output_units = 1,
            _ => unreachable!(),
        }
        let error = evidence.validate_details(&d).unwrap_err();
        let kind = error.downcast_ref::<PreparationBalanceFailure>().unwrap();
        assert!(!kind.allows_lane_fallback(), "change {change}");
    }
}

#[test]
fn damm_output_evidence_expired_or_future_start_is_not_renewed_or_allowed_to_fallback() {
    let (mut evidence, details) = fixture();
    for started in [
        Instant::now() - Duration::from_secs(4),
        Instant::now() + Duration::from_secs(4),
    ] {
        evidence.started = started;
        let error = evidence.validate_details(&details).unwrap_err();
        assert!(error.to_string().contains("expired"));
        assert!(!error
            .downcast_ref::<PreparationBalanceFailure>()
            .unwrap()
            .allows_lane_fallback());
        assert_eq!(evidence.started, started);
    }
}

#[test]
fn damm_output_evidence_recomputes_each_valid_slippage_minimum_from_the_same_net_quote() {
    let (evidence, mut details) = fixture();
    for bps in [0, 1, 1500, 5000, 9980] {
        details.slippage_bps = bps;
        details.minimum_output_units = minimum_output(915, bps).unwrap();
        evidence.validate_details(&details).unwrap();
        details.minimum_output_units += 1;
        assert!(evidence.validate_details(&details).is_err());
    }
    for bps in [9990, 9999, 10_000, u64::MAX] {
        details.slippage_bps = bps;
        assert!(evidence.validate_details(&details).is_err());
    }
}

#[test]
fn damm_output_evidence_checked_constructor_requires_ordered_slots_positive_output_and_original_age(
) {
    let source = Fixture::default();
    let prepared = source.prepare();
    let values: Vec<Option<UiAccount>> =
        serde_json::from_value(serde_json::json!(source.rpc.values)).unwrap();
    let snapshot = check_snapshot(&prepared, 42, &values, false).unwrap();
    for (amount, account, simulated, started) in [
        (0, 42, 43, Instant::now()),
        (1, 0, 43, Instant::now()),
        (1, 42, 41, Instant::now()),
        (1, 42, 43, Instant::now() - Duration::from_secs(4)),
    ] {
        assert!(DammOutputEvidence::checked(
            &prepared, &snapshot, amount, account, simulated, started
        )
        .is_err());
    }
}
