use serde::{Deserialize, Serialize};

use crate::application::sol_transfer_policy::{TransferAmountInput, TransferAmountMode};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SolTransferAmountMode {
    Fixed,
    Percent,
}

#[cfg(test)]
impl SolTransferAmountMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Fixed => "fixed",
            Self::Percent => "percent",
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct SolTransferRequest {
    pub owner_type: String,
    pub owner_id: String,
    pub to_wallet_id: Option<String>,
    pub to_address: Option<String>,
    pub amount_mode: SolTransferAmountMode,
    pub amount_lamports: Option<u64>,
    pub percent: Option<u64>,
    pub idempotency_key: String,
}

impl SolTransferRequest {
    pub(crate) fn input(&self) -> crate::application::sol_transfers::SolTransferInput<'_> {
        crate::application::sol_transfers::SolTransferInput {
            owner_type: &self.owner_type,
            owner_id: &self.owner_id,
            to_wallet_id: self.to_wallet_id.as_deref(),
            to_address: self.to_address.as_deref(),
            amount: self.amount_input(),
            idempotency_key: &self.idempotency_key,
        }
    }
    pub(crate) fn amount_input(&self) -> TransferAmountInput {
        TransferAmountInput {
            mode: match self.amount_mode {
                SolTransferAmountMode::Fixed => TransferAmountMode::Fixed,
                SolTransferAmountMode::Percent => TransferAmountMode::Percent,
            },
            fixed_lamports: self.amount_lamports,
            percent: self.percent,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct SolTransferResponse {
    pub transfer_id: String,
    pub wallet_id: String,
    pub from_pubkey: String,
    pub to_pubkey: String,
    pub status: String,
    pub amount_lamports: Option<u64>,
    pub fee_lamports: Option<u64>,
    pub balance_before_lamports: Option<u64>,
    pub signature: Option<String>,
    pub error: Option<String>,
    pub duplicate: bool,
}

impl From<crate::application::sol_transfers::SolTransferOutcome> for SolTransferResponse {
    fn from(outcome: crate::application::sol_transfers::SolTransferOutcome) -> Self {
        let record = outcome.record;
        Self {
            transfer_id: record.transfer_id,
            wallet_id: record.wallet_id,
            from_pubkey: record.from_pubkey,
            to_pubkey: record.to_pubkey,
            status: record.status,
            amount_lamports: record.actual_amount_lamports,
            fee_lamports: record.fee_lamports,
            balance_before_lamports: record.balance_before_lamports,
            signature: record.signature,
            error: record.error,
            duplicate: outcome.duplicate,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    #[test]
    fn transfer_v1_optional_fields_literal_identity_and_integer_rules_are_unchanged() {
        for optional in [
            json!({}),
            json!({"to_wallet_id":null,"to_address":null,"amount_lamports":null,"percent":null}),
        ] {
            let mut value = json!({"owner_type":" telegram ","owner_id":" 42 ","amount_mode":"fixed","idempotency_key":" key ","simulate":true});
            value
                .as_object_mut()
                .unwrap()
                .extend(optional.as_object().unwrap().clone());
            let request: SolTransferRequest = serde_json::from_value(value).unwrap();
            assert_eq!(request.owner_type, " telegram ");
            assert_eq!(request.owner_id, " 42 ");
            assert_eq!(request.idempotency_key, " key ");
            assert!(request.to_wallet_id.is_none() && request.to_address.is_none());
            assert!(request.amount_lamports.is_none() && request.percent.is_none());
        }
        let mut value = json!({"owner_type":"telegram","owner_id":"42","amount_mode":"fixed","idempotency_key":"key","amount_lamports":u64::MAX});
        assert_eq!(
            serde_json::from_value::<SolTransferRequest>(value.clone())
                .unwrap()
                .amount_lamports,
            Some(u64::MAX)
        );
        value["amount_lamports"] = json!(1.5);
        assert!(serde_json::from_value::<SolTransferRequest>(value.clone()).is_err());
        value["amount_lamports"] = Value::Null;
        value["amount_mode"] = json!("Fixed");
        assert!(serde_json::from_value::<SolTransferRequest>(value).is_err());
    }

    #[test]
    fn transfer_amount_input_is_only_a_mapping_not_early_validation() {
        for mode in ["fixed", "percent"] {
            let request: SolTransferRequest = serde_json::from_value(json!({
                "owner_type":"telegram","owner_id":"42","amount_mode":mode,
                "idempotency_key":"key","amount_lamports":0,"percent":50,
            }))
            .unwrap();
            let input = request.amount_input();
            assert!(matches!(
                (mode, input.mode),
                ("fixed", TransferAmountMode::Fixed) | ("percent", TransferAmountMode::Percent)
            ));
            assert_eq!(input.fixed_lamports, Some(0));
            assert_eq!(input.percent, Some(50));
            assert!(crate::application::sol_transfer_policy::select_amount(input).is_err());
        }
    }
}
