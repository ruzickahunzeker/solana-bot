use super::{lane_preparation::LanePreparation, ExecuteTrade, SdkLane, SdkTradeClient};
use crate::{
    adapters::{
        rpc::{
            damm_sell_simulation::shared::DammSimulationReader, read_runtime::SharedReadRuntime,
            sdk_signature::observe_sdk_signature_shared,
        },
        sdk::{
            damm_sell_message::{DammSellContext, DammSellMessageProfile},
            ordinary_lanes::{
                decode_sdk_report, ensure_sender_layout, ensure_single_fee_strategy,
                execution_infrastructure, OrdinarySdkLane, SdkInvocationFailure,
                SdkLaneObservation, SdkLaneReport,
            },
            ordinary_params::simulation_fee_strategy,
            ordinary_preparation,
            preparation_input::{self, SdkInput},
        },
    },
    domain::{
        preparation_balance::{reject_balance, PreparationBalanceSnapshot},
        trade::{Address, Pool, Side},
    },
};
use sol_trade_sdk::{swqos::SwqosType, TradeBuyParams, TradeSellParams, TradingClient};
use solana_sdk::signature::Signer;
use std::sync::Arc;

pub(super) enum PreparedLaneRequest {
    Buy {
        client: TradingClient,
        request: TradeBuyParams,
        holding: Option<PreparationBalanceSnapshot>,
        message: Option<Arc<crate::adapters::sdk::damm_probe::DammBuyMessage>>,
    },
    Sell {
        client: TradingClient,
        request: TradeSellParams,
        preview: Option<TradeSellParams>,
        balance: u64,
        percent: u64,
        holding: Option<Arc<PreparationBalanceSnapshot>>,
        message: Option<Arc<DammSellMessageProfile>>,
        preview_message: Option<Arc<DammSellMessageProfile>>,
    },
}

pub(super) struct LaneExecution<'a> {
    wallet: &'a SdkTradeClient,
    lane: &'a SdkLane,
    side: Side,
    read_rpc: &'a std::sync::Arc<crate::adapters::rpc::read_runtime::SharedReadRuntime>,
}

fn recheck_after_entry(
    holding: Option<&PreparationBalanceSnapshot>,
    wallet: Address,
    signatures: Vec<String>,
) -> Result<(), SdkInvocationFailure> {
    if let Some(holding) = holding {
        holding
            .recheck(wallet)
            .map_err(|error| SdkInvocationFailure {
                signatures,
                message: error.to_string(),
            })?;
    }
    Ok(())
}

fn damm_context(client: &TradingClient, provider: SwqosType) -> DammSellContext {
    DammSellContext {
        wallet: client.payer.pubkey(),
        provider,
        version: client.transaction_version,
        seed_optimized: client.use_seed_optimize,
    }
}

fn buy_context(
    client: &TradingClient,
    provider: SwqosType,
) -> crate::adapters::sdk::damm_probe::DammBuyContext {
    crate::adapters::sdk::damm_probe::DammBuyContext {
        wallet: client.payer.pubkey(),
        provider,
        version: client.transaction_version,
        seed_optimized: client.use_seed_optimize,
    }
}

async fn invoke_buy(
    runtime: &SharedReadRuntime,
    client: TradingClient,
    request: TradeBuyParams,
    provider: SwqosType,
    message: Option<Arc<crate::adapters::sdk::damm_probe::DammBuyMessage>>,
) -> Result<SdkLaneReport, SdkInvocationFailure> {
    if let Some(message) = message {
        message
            .validate_request(buy_context(&client, provider), &request)
            .map_err(|error| SdkInvocationFailure {
                signatures: vec![],
                message: error.to_string(),
            })?;
        ensure_sender_layout(
            provider,
            &client
                .infrastructure
                .swqos_clients
                .iter()
                .map(|sender| sender.get_swqos_type())
                .collect::<Vec<_>>(),
            client.middleware_manager.is_some(),
        )
        .map_err(|error| SdkInvocationFailure {
            signatures: vec![],
            message: error.to_string(),
        })?;
        let rpc = DammSimulationReader::new(runtime, client.get_rpc().url());
        return message
            .execute(&rpc, |middleware| async move {
                let client = client.with_middleware_manager(middleware);
                decode_sdk_report(client.buy(request).await)
            })
            .await;
    }
    decode_sdk_report(client.buy(request).await)
}

async fn invoke_sell(
    runtime: &SharedReadRuntime,
    client: TradingClient,
    request: TradeSellParams,
    balance: u64,
    percent: u64,
    provider: SwqosType,
    message: Option<Arc<DammSellMessageProfile>>,
) -> Result<SdkLaneReport, SdkInvocationFailure> {
    if let Some(profile) = message {
        let checked = (|| -> anyhow::Result<()> {
            ensure_sender_layout(
                provider,
                &client
                    .infrastructure
                    .swqos_clients
                    .iter()
                    .map(|sender| sender.get_swqos_type())
                    .collect::<Vec<_>>(),
                client.middleware_manager.is_some(),
            )?;
            profile.validate_request(damm_context(&client, provider), &request, balance, percent)
        })();
        checked.map_err(|error| SdkInvocationFailure {
            signatures: vec![],
            message: error.to_string(),
        })?;
        let rpc = DammSimulationReader::new(runtime, client.get_rpc().url());
        profile
            .execute(&rpc, |middleware| async move {
                let client = client.with_middleware_manager(middleware);
                decode_sdk_report(client.sell_by_percent(request, balance, percent).await)
            })
            .await
    } else {
        decode_sdk_report(client.sell_by_percent(request, balance, percent).await)
    }
}

impl<'a> LaneExecution<'a> {
    pub(super) fn new(
        wallet: &'a SdkTradeClient,
        lane: &'a SdkLane,
        side: Side,
        read_rpc: &'a std::sync::Arc<crate::adapters::rpc::read_runtime::SharedReadRuntime>,
    ) -> Self {
        Self {
            wallet,
            lane,
            side,
            read_rpc,
        }
    }

    fn execution_client(&self) -> anyhow::Result<TradingClient> {
        let original = &self.lane.client;
        // A per-invocation view avoids changing shared clients used by other backends.
        let mut client = original.clone();
        client.infrastructure = execution_infrastructure(
            &original.infrastructure,
            self.lane.swqos_type,
            original.middleware_manager.is_some(),
        )?;
        Ok(client)
    }
}

impl OrdinarySdkLane for LaneExecution<'_> {
    type Prepared = PreparedLaneRequest;
    fn provider(&self) -> SwqosType {
        self.lane.swqos_type
    }

    async fn prepare(&self, trade: &ExecuteTrade) -> anyhow::Result<Self::Prepared> {
        let client = self.execution_client()?;
        let source = LanePreparation::new(self.wallet, self.lane, self.read_rpc);
        match self.side {
            Side::Buy => {
                let request = ordinary_preparation::buy(&source, trade, self.provider()).await?;
                ensure_single_fee_strategy(
                    &request.gas_fee_strategy,
                    self.provider(),
                    self.side,
                    request.simulate,
                )?;
                let message = source
                    .take_buy_output()?
                    .map(|quote| {
                        crate::adapters::sdk::damm_probe::DammBuyMessage::new(
                            quote,
                            buy_context(&client, self.provider()),
                            trade,
                            &request,
                        )
                    })
                    .transpose()?;
                Ok(PreparedLaneRequest::Buy {
                    client,
                    request,
                    holding: source.take_balance()?,
                    message,
                })
            }
            Side::Sell => {
                let (request, balance, percent) =
                    ordinary_preparation::sell(&source, trade, self.provider()).await?;
                ensure_single_fee_strategy(
                    &request.gas_fee_strategy,
                    self.provider(),
                    self.side,
                    request.simulate,
                )?;
                let preview = if trade.pool == Pool::MeteoraDammV2 && !trade.simulate {
                    let mut preview = request.clone();
                    preview.simulate = true;
                    preview.gas_fee_strategy = simulation_fee_strategy();
                    ensure_single_fee_strategy(
                        &preview.gas_fee_strategy,
                        self.provider(),
                        self.side,
                        true,
                    )?;
                    Some(preview)
                } else {
                    None
                };
                let holding = source.take_balance()?.map(Arc::new);
                let quote = source.take_output()?;
                let (message, preview_message) = if trade.pool == Pool::MeteoraDammV2 {
                    let holding = holding
                        .as_ref()
                        .ok_or_else(|| reject_balance("DAMM final balance evidence is missing"))?;
                    let context = damm_context(&client, self.provider());
                    let quote = quote
                        .as_ref()
                        .ok_or_else(|| reject_balance("DAMM final output evidence is missing"))?;
                    let prepare_message = |intent: &ExecuteTrade, request: &TradeSellParams| {
                        DammSellMessageProfile::new(
                            context,
                            intent,
                            request,
                            balance,
                            percent,
                            holding.clone(),
                            quote.clone(),
                        )
                        .map_err(|error| {
                            reject_balance("DAMM final message profile rejected")
                                .context(error.to_string())
                        })
                    };
                    let message = prepare_message(trade, &request)?;
                    let preview_message = match &preview {
                        Some(request) => {
                            let mut intent = trade.clone();
                            intent.simulate = true;
                            Some(prepare_message(&intent, request)?)
                        }
                        None => None,
                    };
                    (Some(message), preview_message)
                } else {
                    if quote.is_some() {
                        return Err(reject_balance(
                            "non-DAMM preparation has unexpected output evidence",
                        ));
                    }
                    (None, None)
                };
                Ok(PreparedLaneRequest::Sell {
                    client,
                    request,
                    preview,
                    balance,
                    percent,
                    holding,
                    message,
                    preview_message,
                })
            }
        }
    }

    fn validate_prepared(
        &self,
        prepared: &Self::Prepared,
        trade: &ExecuteTrade,
    ) -> anyhow::Result<()> {
        let wallet = Address::from_bytes(self.wallet.wallet().to_bytes());
        let (holding, input) = match prepared {
            PreparedLaneRequest::Buy {
                client,
                request,
                holding,
                message,
            } => {
                if self.side != Side::Buy
                    || request.mint != trade.mint
                    || client.payer.pubkey() != self.wallet.wallet()
                    || Some(request.input_token_amount) != trade.amount_lamports
                {
                    return Err(reject_balance(
                        "prepared buy does not match its wallet or input intent",
                    ));
                }
                if message.is_some() != (trade.pool == Pool::MeteoraDammV2) {
                    return Err(reject_balance("DAMM buy quote is missing or unexpected"));
                }
                if let Some(message) = message {
                    message.validate(buy_context(client, self.provider()), trade, request)?;
                }
                (holding.as_ref(), SdkInput::Buy(request))
            }
            PreparedLaneRequest::Sell {
                client,
                request,
                holding,
                balance,
                percent,
                preview,
                message,
                preview_message,
            } => {
                if self.side != Side::Sell
                    || request.mint != trade.mint
                    || client.payer.pubkey() != self.wallet.wallet()
                    || Some(*percent) != trade.sell_percent
                {
                    return Err(reject_balance(
                        "prepared sell does not match its wallet or input intent",
                    ));
                }
                if holding
                    .as_ref()
                    .ok_or_else(|| reject_balance("sell preparation balance is missing"))?
                    .checked_units(wallet, Address::from_bytes(trade.mint.to_bytes()))?
                    != *balance
                {
                    return Err(reject_balance(
                        "prepared sell balance differs from captured metadata",
                    ));
                }
                if preview.is_some() != (trade.pool == Pool::MeteoraDammV2 && !trade.simulate) {
                    return Err(reject_balance(
                        "prepared SDK preview does not match its execution mode",
                    ));
                }
                if let Some(preview) = preview {
                    preparation_input::validate_preview(
                        holding.as_deref(),
                        wallet,
                        trade,
                        preview,
                    )?;
                }
                if message.is_some() != (trade.pool == Pool::MeteoraDammV2)
                    || preview_message.is_some() != preview.is_some()
                {
                    return Err(reject_balance(
                        "DAMM final message profiles are missing or unexpected",
                    ));
                }
                let context = damm_context(client, self.provider());
                if let Some(message) = message {
                    message.validate_intent(trade)?;
                    message.validate_request(context, request, *balance, *percent)?;
                }
                if let (Some(message), Some(preview)) = (preview_message, preview) {
                    let mut intent = trade.clone();
                    intent.simulate = true;
                    message.validate_intent(&intent)?;
                    message.validate_request(context, preview, *balance, *percent)?;
                }
                (holding.as_deref(), SdkInput::Sell(request))
            }
        };
        if holding.is_some() {
            let client = match prepared {
                PreparedLaneRequest::Buy { client, .. }
                | PreparedLaneRequest::Sell { client, .. } => client,
            };
            crate::adapters::sdk::preparation_balance::require_canonical_mode(
                client.use_seed_optimize,
            )?;
        }
        preparation_input::validate(holding, wallet, trade, self.side, input)
    }

    async fn invoke(
        &self,
        prepared: Self::Prepared,
    ) -> Result<SdkLaneReport, SdkInvocationFailure> {
        match prepared {
            PreparedLaneRequest::Buy {
                client,
                request,
                holding,
                message,
            } => {
                recheck_after_entry(
                    holding.as_ref(),
                    Address::from_bytes(self.wallet.wallet().to_bytes()),
                    vec![],
                )?;
                if message.is_some()
                    != (request.dex_type == sol_trade_sdk::trading::factory::DexType::MeteoraDammV2)
                {
                    return Err(SdkInvocationFailure {
                        signatures: vec![],
                        message: "DAMM buy quote is missing or unexpected at SDK entry".into(),
                    });
                }
                invoke_buy(
                    self.read_rpc.as_ref(),
                    client,
                    request,
                    self.provider(),
                    message,
                )
                .await
            }
            PreparedLaneRequest::Sell {
                client,
                request,
                preview,
                balance,
                percent,
                holding,
                message,
                preview_message,
            } => {
                recheck_after_entry(
                    holding.as_deref(),
                    Address::from_bytes(self.wallet.wallet().to_bytes()),
                    vec![],
                )?;
                if let Some(preview) = preview {
                    let final_message =
                        message
                            .as_ref()
                            .cloned()
                            .ok_or_else(|| SdkInvocationFailure {
                                signatures: vec![],
                                message: "DAMM final candidate is missing".into(),
                            })?;
                    let preview_message = preview_message.ok_or_else(|| SdkInvocationFailure {
                        signatures: vec![],
                        message: "DAMM unsigned preview is missing".into(),
                    })?;
                    preview_message
                        .validate_request(
                            damm_context(&client, self.provider()),
                            &preview,
                            balance,
                            percent,
                        )
                        .map_err(|error| SdkInvocationFailure {
                            signatures: vec![],
                            message: error.to_string(),
                        })?;
                    let rpc =
                        DammSimulationReader::new(self.read_rpc.as_ref(), client.get_rpc().url());
                    return final_message
                        .after_unsigned_preview(&preview_message, &rpc, || async move {
                            invoke_sell(
                                self.read_rpc.as_ref(),
                                client,
                                request,
                                balance,
                                percent,
                                self.provider(),
                                message,
                            )
                            .await
                        })
                        .await;
                }
                invoke_sell(
                    self.read_rpc.as_ref(),
                    client,
                    request,
                    balance,
                    percent,
                    self.provider(),
                    message,
                )
                .await
            }
        }
    }

    async fn observe(
        &self,
        signature: &str,
        trade: &ExecuteTrade,
    ) -> anyhow::Result<SdkLaneObservation> {
        if self.provider() == SwqosType::Default
            || trade.wait_tx_confirmed
            || trade.simulate
            || trade.mev_submit_verify_ms == 0
        {
            return Ok(SdkLaneObservation::Skipped);
        }
        let url = self.lane.client.get_rpc().url();
        match observe_sdk_signature_shared(
            self.read_rpc.as_ref(),
            &url,
            signature,
            trade.mev_submit_verify_ms,
        )
        .await?
        {
            Some(observation) => Ok(SdkLaneObservation::Observed(observation)),
            None => Ok(SdkLaneObservation::NotVisible),
        }
    }
}

#[cfg(test)]
mod balance_tests {
    use super::*;
    use std::time::{Duration, Instant};
    #[test]
    fn preparation_balance_recheck_after_sdk_entry_preserves_all_existing_signature_evidence() {
        let wallet = Address::from_bytes([1; 32]);
        let snapshot = PreparationBalanceSnapshot::fixture(
            wallet,
            Address::from_bytes([7; 32]),
            1000,
            Instant::now() - Duration::from_secs(4),
        );
        let signatures = vec![
            "fixture-preview-signature-1".to_string(),
            "fixture-preview-signature-2".to_string(),
        ];
        let failure = recheck_after_entry(Some(&snapshot), wallet, signatures.clone())
            .err()
            .unwrap();
        assert_eq!(failure.signatures, signatures);
        assert!(failure.message.contains("expired"));
    }
    #[test]
    fn preparation_balance_recheck_is_read_only_and_does_not_renew_snapshot() {
        let wallet = Address::from_bytes([1; 32]);
        let snapshot = PreparationBalanceSnapshot::fixture(
            wallet,
            Address::from_bytes([7; 32]),
            1000,
            Instant::now(),
        );
        assert!(recheck_after_entry(Some(&snapshot), wallet, vec![]).is_ok());
        assert!(recheck_after_entry(None, wallet, vec![]).is_ok());
        assert!(
            recheck_after_entry(Some(&snapshot), Address::from_bytes([2; 32]), vec![]).is_err()
        );
    }
}
