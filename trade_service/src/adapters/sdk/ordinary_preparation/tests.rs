use super::*;
use crate::domain::trade::{USD1_MINT, USDC_MINT, WSOL_MINT};
use sol_trade_sdk::trading::core::params::BonkParams;
use solana_sdk::hash::Hash;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    Load,
    Balance,
    Exists,
    Probe,
}

#[derive(Debug, PartialEq, Eq)]
enum Event {
    Load(Pool, Pubkey, Pubkey),
    Balance(Pubkey),
    Exists(Pubkey),
    Probe(Pubkey, Pubkey, u64, u8, Hash),
    BuyProbe(Pubkey, Pubkey, u64, u8, Hash),
}

struct Source {
    events: Mutex<Vec<Event>>,
    balance: u64,
    exists: bool,
    quoted: u64,
    fail: Option<Stage>,
    block: Option<Stage>,
    dropped: Arc<AtomicBool>,
    wrong_variant: bool,
}

struct DropFlag(Arc<AtomicBool>);
impl Drop for DropFlag {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

impl Source {
    fn new() -> Self {
        Self {
            events: Mutex::new(vec![]),
            balance: 1_000_000,
            exists: false,
            quoted: 1_000,
            fail: None,
            block: None,
            dropped: Arc::default(),
            wrong_variant: false,
        }
    }

    async fn checkpoint(&self, stage: Stage) -> anyhow::Result<()> {
        if self.block == Some(stage) {
            let _drop = DropFlag(self.dropped.clone());
            std::future::pending::<()>().await;
        }
        if self.fail == Some(stage) {
            bail!("synthetic {stage:?} failure");
        }
        Ok(())
    }
}

fn key(n: u8) -> Pubkey {
    Pubkey::new_from_array([n; 32])
}

fn trade(pool: Pool, quote: Pubkey) -> ExecuteTrade {
    ExecuteTrade {
        pool,
        mint: key(7),
        quote_mint: quote,
        pool_id: Some(key(8)),
        sol_hop_pool: None,
        sol_hop_pool_id: None,
        amount_lamports: Some(1_000),
        sell_percent: Some(100),
        slippage_bps: 1_500,
        tip_sol: 0.0001,
        recent_blockhash: Hash::new_from_array([9; 32]),
        wait_tx_confirmed: false,
        mev_submit_verify_ms: 17,
        simulate: false,
        use_mev: None,
        mev_provider: None,
    }
}

impl OrdinaryPreparationSource for Source {
    async fn load_params(&self, trade: &ExecuteTrade) -> anyhow::Result<DexParamEnum> {
        self.events
            .lock()
            .unwrap()
            .push(Event::Load(trade.pool, trade.mint, trade.quote_mint));
        self.checkpoint(Stage::Load).await?;
        if trade.pool == Pool::MeteoraDammV2 && !self.wrong_variant {
            Ok(DexParamEnum::MeteoraDammV2(
                MeteoraDammV2Params::new(
                    key(10),
                    key(11),
                    key(12),
                    trade.mint,
                    trade.quote_mint,
                    key(13),
                    key(14),
                )
                .with_swap_mode(2)
                .with_referral_token_account(key(15))
                .with_rate_limiter_sysvar(true),
            ))
        } else {
            Ok(DexParamEnum::Bonk(BonkParams::default()))
        }
    }

    async fn token_balance(&self, mint: Pubkey) -> anyhow::Result<u64> {
        self.events.lock().unwrap().push(Event::Balance(mint));
        self.checkpoint(Stage::Balance).await?;
        Ok(self.balance)
    }

    async fn mint_account_exists(&self, mint: Pubkey) -> anyhow::Result<bool> {
        self.events.lock().unwrap().push(Event::Exists(mint));
        self.checkpoint(Stage::Exists).await?;
        Ok(self.exists)
    }

    async fn damm_sell_output(
        &self,
        trade: &ExecuteTrade,
        params: &MeteoraDammV2Params,
        amount: u64,
    ) -> anyhow::Result<u64> {
        assert_eq!(params.referral_token_account, Some(key(15)));
        assert!(params.include_rate_limiter_sysvar);
        assert_eq!(params.token_a_mint, trade.mint);
        assert_eq!(params.token_b_mint, trade.quote_mint);
        self.events.lock().unwrap().push(Event::Probe(
            trade.mint,
            trade.quote_mint,
            amount,
            params.swap_mode,
            trade.recent_blockhash,
        ));
        self.checkpoint(Stage::Probe).await?;
        Ok(self.quoted)
    }
    async fn damm_buy_output(
        &self,
        trade: &ExecuteTrade,
        params: &MeteoraDammV2Params,
        amount: u64,
    ) -> anyhow::Result<u64> {
        self.events.lock().unwrap().push(Event::BuyProbe(
            trade.mint,
            trade.quote_mint,
            amount,
            params.swap_mode,
            trade.recent_blockhash,
        ));
        self.checkpoint(Stage::Probe).await?;
        Ok(self.quoted)
    }
}

fn load(t: &ExecuteTrade) -> Event {
    Event::Load(t.pool, t.mint, t.quote_mint)
}

#[tokio::test]
async fn buy_preparation_keeps_protocol_quote_balance_ata_order_and_hop_units() {
    for pool in [
        Pool::Pump,
        Pool::PumpAmm,
        Pool::Bonk,
        Pool::StonkFun,
        Pool::StonkFunSwap,
        Pool::MeteoraDammV2,
        Pool::MeteoraDbc,
        Pool::RaydiumAmmV4,
        Pool::RaydiumCpmm,
    ] {
        for quote in [
            WSOL_MINT.parse().unwrap(),
            USDC_MINT.parse().unwrap(),
            USD1_MINT.parse().unwrap(),
            key(90),
        ] {
            for hop in [false, true] {
                for simulate in [false, true] {
                    let source = Source::new();
                    let mut t = trade(pool, quote);
                    if pool == Pool::MeteoraDammV2 {
                        t.sell_percent = None;
                    }
                    t.simulate = simulate;
                    if hop {
                        t.sol_hop_pool = Some(Pool::RaydiumCpmm);
                        t.sol_hop_pool_id = Some(key(91));
                    }
                    let token = if hop {
                        Ok(TradeTokenType::SOL)
                    } else {
                        trade_token_type_for_buy(pool, &quote)
                    };
                    let result = buy(&source, &t, SwqosType::Temporal).await;
                    let mut expected = vec![load(&t)];
                    match token {
                        Err(error) => {
                            assert_eq!(result.err().unwrap().to_string(), error.to_string())
                        }
                        Ok(token) => {
                            if matches!(
                                pool,
                                Pool::PumpAmm
                                    | Pool::StonkFun
                                    | Pool::StonkFunSwap
                                    | Pool::RaydiumCpmm
                            ) && quote != WSOL_MINT.parse::<Pubkey>().unwrap()
                                && !hop
                            {
                                expected.push(Event::Balance(quote));
                            }
                            expected.push(Event::Exists(t.mint));
                            if pool == Pool::MeteoraDammV2 && hop {
                                assert_eq!(
                                    result.err().unwrap().to_string(),
                                    "DAMM buy requires a direct buy-only intent"
                                );
                                assert_eq!(*source.events.lock().unwrap(), expected);
                                continue;
                            }
                            if pool == Pool::MeteoraDammV2 {
                                expected.push(Event::BuyProbe(
                                    t.mint,
                                    t.quote_mint,
                                    1000,
                                    0,
                                    t.recent_blockhash,
                                ));
                            }
                            let params = result.unwrap();
                            assert_eq!(params.input_token_type, token);
                            assert_eq!(params.input_token_amount, 1_000);
                            assert_eq!(params.simulate, simulate);
                            assert!(params.create_mint_ata);
                            assert_eq!(
                                params.fixed_output_token_amount,
                                (pool == Pool::MeteoraDammV2).then_some(850)
                            );
                        }
                    }
                    assert_eq!(*source.events.lock().unwrap(), expected);
                }
            }
        }
    }
}

#[tokio::test]
async fn buy_errors_preserve_load_then_quote_then_amount_precedence() {
    let mut t = trade(Pool::Bonk, USDC_MINT.parse().unwrap());
    t.amount_lamports = None;
    let mut source = Source::new();
    source.fail = Some(Stage::Load);
    assert_eq!(
        buy(&source, &t, SwqosType::Default)
            .await
            .err()
            .unwrap()
            .to_string(),
        "synthetic Load failure"
    );
    assert_eq!(*source.events.lock().unwrap(), [load(&t)]);
    let source = Source::new();
    assert_eq!(
        buy(&source, &t, SwqosType::Default)
            .await
            .err()
            .unwrap()
            .to_string(),
        "bonk does not support USDC quote in this SDK; use WSOL or USD1"
    );
    assert_eq!(*source.events.lock().unwrap(), [load(&t)]);
    t.quote_mint = WSOL_MINT.parse().unwrap();
    let source = Source::new();
    assert_eq!(
        buy(&source, &t, SwqosType::Default)
            .await
            .err()
            .unwrap()
            .to_string(),
        "amount_lamports missing"
    );
    assert_eq!(*source.events.lock().unwrap(), [load(&t)]);
}

#[tokio::test]
async fn buy_balance_and_ata_failures_stop_without_additional_preparation() {
    let t = trade(Pool::RaydiumCpmm, key(90));
    for stage in [Some(Stage::Balance), Some(Stage::Exists), None] {
        let mut source = Source::new();
        source.fail = stage;
        if stage.is_none() {
            source.balance = 999;
        }
        let message = buy(&source, &t, SwqosType::Jito)
            .await
            .err()
            .unwrap()
            .to_string();
        let mut expected = vec![load(&t), Event::Balance(t.quote_mint)];
        match stage {
            Some(Stage::Exists) => {
                expected.push(Event::Exists(t.mint));
                assert_eq!(message, "synthetic Exists failure");
            }
            Some(Stage::Balance) => assert_eq!(message, "synthetic Balance failure"),
            None => assert_eq!(
                message,
                "quote token balance insufficient: available 999 units, requested 1000 units"
            ),
            _ => unreachable!(),
        }
        assert_eq!(*source.events.lock().unwrap(), expected);
    }
}

#[tokio::test]
async fn sell_missing_zero_failed_balance_and_wrong_variant_keep_their_legacy_order() {
    let mut t = trade(Pool::MeteoraDammV2, WSOL_MINT.parse().unwrap());
    t.sell_percent = None;
    let source = Source::new();
    assert_eq!(
        sell(&source, &t, SwqosType::Default)
            .await
            .err()
            .unwrap()
            .to_string(),
        "sell_percent missing"
    );
    assert!(source.events.lock().unwrap().is_empty());
    t.sell_percent = Some(100);
    for (balance, fail, wrong, expected) in [
        (0, None, false, "token balance is zero"),
        (
            1_000,
            Some(Stage::Balance),
            false,
            "synthetic Balance failure",
        ),
        (1_000, Some(Stage::Load), false, "synthetic Load failure"),
        (1_000, None, true, "DAMM v2 protocol params missing"),
    ] {
        let mut source = Source::new();
        source.balance = balance;
        source.fail = fail;
        source.wrong_variant = wrong;
        assert_eq!(
            sell(&source, &t, SwqosType::Default)
                .await
                .err()
                .unwrap()
                .to_string(),
            expected
        );
        let mut events = vec![Event::Balance(t.mint)];
        if balance != 0 && fail != Some(Stage::Balance) {
            events.push(load(&t));
        }
        assert_eq!(*source.events.lock().unwrap(), events);
    }
}

#[tokio::test]
async fn damm_sell_checks_exact_integer_input_before_probe_and_keeps_slippage_and_flags() {
    for simulate in [false, true] {
        for percent in [25, 99, 100] {
            let mut t = trade(Pool::MeteoraDammV2, WSOL_MINT.parse().unwrap());
            t.sell_percent = Some(percent);
            t.simulate = simulate;
            let source = Source::new();
            let (params, balance, actual_percent) =
                sell(&source, &t, SwqosType::Temporal).await.unwrap();
            assert_eq!((balance, actual_percent), (1_000_000, percent));
            assert_eq!(params.input_token_amount, 0);
            assert_eq!(params.fixed_output_token_amount, Some(850));
            assert_eq!(params.close_mint_token_ata, percent == 100);
            assert_eq!(params.simulate, simulate);
            let DexParamEnum::MeteoraDammV2(damm) = params.extension_params else {
                panic!("variant changed")
            };
            assert_eq!(damm.swap_mode, 0);
            assert_eq!(damm.referral_token_account, Some(key(15)));
            assert!(damm.include_rate_limiter_sysvar);
            assert_eq!(
                *source.events.lock().unwrap(),
                [
                    Event::Balance(t.mint),
                    load(&t),
                    Event::Probe(
                        t.mint,
                        t.quote_mint,
                        1_000_000 * percent / 100,
                        0,
                        t.recent_blockhash
                    )
                ]
            );
        }
    }
    for (balance, percent, message) in [
        (u64::MAX, 2, "sell amount overflow"),
        (1, 25, "sell input amount rounds to zero"),
    ] {
        let mut source = Source::new();
        source.balance = balance;
        let mut t = trade(Pool::MeteoraDammV2, WSOL_MINT.parse().unwrap());
        t.sell_percent = Some(percent);
        assert_eq!(
            sell(&source, &t, SwqosType::Default)
                .await
                .err()
                .unwrap()
                .to_string(),
            message
        );
        assert_eq!(
            *source.events.lock().unwrap(),
            [Event::Balance(t.mint), load(&t)]
        );
    }
}

#[tokio::test]
async fn non_damm_sell_does_not_add_probe_or_damm_overflow_rules() {
    for pool in [
        Pool::Pump,
        Pool::PumpAmm,
        Pool::Bonk,
        Pool::StonkFun,
        Pool::StonkFunSwap,
        Pool::MeteoraDbc,
        Pool::RaydiumAmmV4,
        Pool::RaydiumCpmm,
    ] {
        let mut source = Source::new();
        source.balance = u64::MAX;
        let t = trade(pool, WSOL_MINT.parse().unwrap());
        let (params, balance, percent) = sell(&source, &t, SwqosType::Default).await.unwrap();
        assert_eq!((balance, percent), (u64::MAX, 100));
        assert_eq!(params.fixed_output_token_amount, None);
        assert_eq!(
            *source.events.lock().unwrap(),
            [Event::Balance(t.mint), load(&t)]
        );
    }
}

#[tokio::test]
async fn damm_probe_failure_and_bad_quote_stop_before_final_sdk_request() {
    let mut t = trade(Pool::MeteoraDammV2, WSOL_MINT.parse().unwrap());
    for (quoted, slippage, fail, message) in [
        (1_000, 1_500, Some(Stage::Probe), "synthetic Probe failure"),
        (0, 1_500, None, "invalid DAMM v2 quote or slippage"),
        (1, 1_500, None, "DAMM v2 minimum output rounds to zero"),
        (1_000, 10_000, None, "invalid DAMM v2 quote or slippage"),
    ] {
        let mut source = Source::new();
        source.quoted = quoted;
        source.fail = fail;
        t.slippage_bps = slippage;
        assert_eq!(
            sell(&source, &t, SwqosType::Temporal)
                .await
                .err()
                .unwrap()
                .to_string(),
            message
        );
        assert_eq!(source.events.lock().unwrap().len(), 3);
    }
    t.quote_mint = USD1_MINT.parse().unwrap();
    t.slippage_bps = 1_500;
    let source = Source::new();
    assert_eq!(
        sell(&source, &t, SwqosType::Temporal)
            .await
            .err()
            .unwrap()
            .to_string(),
        "USD1 quote is only supported for bonk or stonkfun in this SDK"
    );
    assert_eq!(source.events.lock().unwrap().len(), 3);
}

#[tokio::test]
async fn cancellation_drops_each_preparation_await_without_retry_or_detached_work() {
    for (is_sell, stage) in [
        (false, Stage::Load),
        (false, Stage::Balance),
        (false, Stage::Exists),
        (true, Stage::Balance),
        (true, Stage::Load),
        (true, Stage::Probe),
    ] {
        let mut source = Source::new();
        source.block = Some(stage);
        let t = trade(
            if is_sell {
                Pool::MeteoraDammV2
            } else {
                Pool::RaydiumCpmm
            },
            if is_sell {
                WSOL_MINT.parse().unwrap()
            } else {
                key(90)
            },
        );
        let result = tokio::time::timeout(std::time::Duration::from_millis(20), async {
            if is_sell {
                sell(&source, &t, SwqosType::Default).await.map(|_| ())
            } else {
                buy(&source, &t, SwqosType::Default).await.map(|_| ())
            }
        })
        .await;
        assert!(result.is_err());
        assert!(source.dropped.load(Ordering::SeqCst));
        let expected = match (is_sell, stage) {
            (false, Stage::Load) | (true, Stage::Balance) => 1,
            (false, Stage::Balance) | (true, Stage::Load) => 2,
            _ => 3,
        };
        assert_eq!(source.events.lock().unwrap().len(), expected);
    }
}

#[tokio::test]
async fn damm_buy_preparation_uses_a_net_quote_minimum_instead_of_a_constant_floor() {
    for quote in [WSOL_MINT.parse().unwrap(), USDC_MINT.parse().unwrap()] {
        let source = Source::new();
        let mut command = trade(Pool::MeteoraDammV2, quote);
        command.sell_percent = None;
        let request = buy(&source, &command, SwqosType::Temporal).await.unwrap();
        assert_eq!(request.fixed_output_token_amount, Some(850));
        assert_eq!(
            *source.events.lock().unwrap(),
            [
                load(&command),
                Event::Exists(command.mint),
                Event::BuyProbe(
                    command.mint,
                    command.quote_mint,
                    1000,
                    0,
                    command.recent_blockhash
                )
            ]
        );
    }
}

#[tokio::test]
async fn damm_buy_preparation_quote_failure_zero_rounding_and_bad_slippage_never_fall_back_to_one()
{
    for (units, slippage, fail) in [
        (1000, 1500, Some(Stage::Probe)),
        (0, 1500, None),
        (1, 1500, None),
        (1000, 10000, None),
    ] {
        let mut source = Source::new();
        source.quoted = units;
        source.fail = fail;
        let mut command = trade(Pool::MeteoraDammV2, WSOL_MINT.parse().unwrap());
        command.sell_percent = None;
        command.slippage_bps = slippage;
        assert!(buy(&source, &command, SwqosType::Default).await.is_err());
        assert_eq!(source.events.lock().unwrap().len(), 3);
    }
}

#[tokio::test]
async fn damm_buy_preparation_cancellation_drops_the_quote_without_detached_retry() {
    let mut source = Source::new();
    source.block = Some(Stage::Probe);
    let mut command = trade(Pool::MeteoraDammV2, WSOL_MINT.parse().unwrap());
    command.sell_percent = None;
    assert!(tokio::time::timeout(
        std::time::Duration::from_millis(20),
        buy(&source, &command, SwqosType::Default)
    )
    .await
    .is_err());
    assert!(source.dropped.load(Ordering::SeqCst));
    assert_eq!(source.events.lock().unwrap().len(), 3);
}

#[test]
fn damm_buy_projection_requires_an_explicit_positive_minimum_and_keeps_other_protocols_unset() {
    let command = trade(Pool::MeteoraDammV2, WSOL_MINT.parse().unwrap());
    for minimum in [None, Some(0)] {
        assert!(ordinary_buy_params(
            &command,
            DexParamEnum::Bonk(Default::default()),
            TradeTokenType::SOL,
            1000,
            false,
            SwqosType::Default,
            minimum
        )
        .is_err());
    }
    let command = trade(Pool::Pump, WSOL_MINT.parse().unwrap());
    assert!(ordinary_buy_params(
        &command,
        DexParamEnum::Bonk(Default::default()),
        TradeTokenType::SOL,
        1000,
        false,
        SwqosType::Default,
        Some(1)
    )
    .is_err());
}
