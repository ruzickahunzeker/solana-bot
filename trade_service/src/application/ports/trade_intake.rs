use crate::application::{
    ports::trade_admission::{RejectedAdmissionPayload, ValidatedAdmissionPayload},
    trade_command::TradeCommand,
    trade_input::TradeInput,
    validated_trade::ValidatedTrade,
};

/// The codec stays lazy and outside Application; binding only resolves wallet metadata.
pub(crate) trait TradeIntakeRequest: Send + Sync {
    type Validated<'a>: ValidatedAdmissionPayload + Send
    where
        Self: 'a;
    type Rejected<'a>: RejectedAdmissionPayload + Send
    where
        Self: 'a;

    fn requested_wallet_id(&self) -> Option<&str>;
    fn bind_wallet(&mut self, wallet_id: String);
    fn validation_input(&self) -> TradeInput<'_>;
    fn validated_payload<'a>(
        &'a self,
        wallet_id: &'a str,
        wallet_pubkey: &'a str,
        normalized: &'a ValidatedTrade,
        command: &'a TradeCommand,
    ) -> Self::Validated<'a>;
    fn rejected_payload<'a>(
        &'a self,
        wallet_id: &'a str,
        wallet_pubkey: &'a str,
    ) -> Self::Rejected<'a>;
}
