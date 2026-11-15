use crate::{
    aggregator_quote,
    api::dto::sol_buy::SolBuyQuoteRequest,
    application::ports::sol_buy_quotes::{SolBuyQuoteInput, SolBuyQuoteSource},
    domain::quote::QuoteEstimate,
};

pub(crate) struct AggregatorQuoteSource;

impl SolBuyQuoteSource for AggregatorQuoteSource {
    type Request = SolBuyQuoteRequest;

    fn map_request(&self, input: SolBuyQuoteInput<'_>) -> Self::Request {
        SolBuyQuoteRequest {
            wallet_id: input.wallet_id.into(),
            mint: input.mint.into(),
            amount_lamports: input.amount_lamports,
            slippage_bps: input.slippage_bps,
        }
    }
    fn validate(&self, request: &Self::Request) -> anyhow::Result<()> {
        aggregator_quote::validate_request(request)
    }
    async fn fetch(&self, request: &Self::Request) -> anyhow::Result<QuoteEstimate> {
        Ok(aggregator_quote::quote(request).await?.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::trade::WSOL_MINT;

    #[test]
    fn quote_source_mapping_keeps_literal_fields_and_full_integer_width_without_fetch() {
        let input = SolBuyQuoteInput {
            wallet_id: " wallet ",
            mint: " literal mint ",
            amount_lamports: u64::MAX,
            slippage_bps: u16::MAX,
        };
        let request = AggregatorQuoteSource.map_request(input);
        assert_eq!(request.wallet_id, input.wallet_id);
        assert_eq!(request.mint, input.mint);
        assert_eq!(request.amount_lamports, u64::MAX);
        assert_eq!(request.slippage_bps, u16::MAX);
    }

    #[test]
    fn quote_source_reuses_original_parameter_limits_and_error_priority_without_fetch() {
        let original = SolBuyQuoteInput {
            wallet_id: "fixture-wallet",
            mint: "11111111111111111111111111111111",
            amount_lamports: 1_000_000,
            slippage_bps: 1500,
        };
        for (input, expected) in [
            (
                SolBuyQuoteInput {
                    wallet_id: " ",
                    mint: "invalid",
                    amount_lamports: 0,
                    slippage_bps: 0,
                },
                Some("wallet_id is required and must be at most 128 characters"),
            ),
            (
                SolBuyQuoteInput {
                    mint: "invalid",
                    amount_lamports: 0,
                    slippage_bps: 0,
                    ..original
                },
                Some("mint must be a valid pubkey"),
            ),
            (
                SolBuyQuoteInput {
                    mint: WSOL_MINT,
                    amount_lamports: 0,
                    slippage_bps: 0,
                    ..original
                },
                Some("output mint must differ from WSOL"),
            ),
            (
                SolBuyQuoteInput {
                    amount_lamports: 99_999,
                    slippage_bps: 0,
                    ..original
                },
                Some("amount_lamports must be between 0.0001 and 0.1 SOL"),
            ),
            (
                SolBuyQuoteInput {
                    amount_lamports: 100_000_001,
                    ..original
                },
                Some("amount_lamports must be between 0.0001 and 0.1 SOL"),
            ),
            (
                SolBuyQuoteInput {
                    slippage_bps: 0,
                    ..original
                },
                Some("slippage_bps must be between 1 and 3000"),
            ),
            (
                SolBuyQuoteInput {
                    slippage_bps: 3001,
                    ..original
                },
                Some("slippage_bps must be between 1 and 3000"),
            ),
            (
                SolBuyQuoteInput {
                    amount_lamports: 100_000,
                    slippage_bps: 1,
                    ..original
                },
                None,
            ),
            (
                SolBuyQuoteInput {
                    amount_lamports: 100_000_000,
                    slippage_bps: 3000,
                    ..original
                },
                None,
            ),
        ] {
            let request = AggregatorQuoteSource.map_request(input);
            assert_eq!(
                AggregatorQuoteSource
                    .validate(&request)
                    .err()
                    .as_ref()
                    .map(ToString::to_string)
                    .as_deref(),
                expected
            );
        }
    }
}
