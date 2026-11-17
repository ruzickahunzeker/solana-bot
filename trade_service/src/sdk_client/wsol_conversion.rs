use self::authorization::WsolAuthorization;
use self::instructions::build_wsol_instructions;
use super::SdkTradeClient;
use crate::application::wsol_failure::WsolFailure;
use crate::application::wsol_submission_types::{WsolDirection as Direction, WsolQuoteSnapshot};
mod authorization;
mod instructions;
pub(crate) mod preparation;
#[cfg(test)]
pub(crate) mod queries;
pub(crate) mod simulation;
use anyhow::{bail, Context, Result};
use solana_client::nonblocking::rpc_client::RpcClient;
#[cfg(test)]
use solana_client::rpc_response::RpcSimulateTransactionResult;
#[cfg(test)]
use solana_commitment_config::CommitmentConfig;
#[cfg(test)]
use solana_program_pack::Pack;
use solana_sdk::signature::{Keypair, Signer};
use solana_sdk::{
    message::Message, pubkey::Pubkey, signature::Signature, transaction::Transaction,
};
#[cfg(test)]
use spl_associated_token_account_interface::address::get_associated_token_address_with_program_id;

pub(crate) fn quote_message(
    owner: &Pubkey,
    direction: Direction,
    amount: u64,
    balance: u64,
    exists: bool,
    rent: u64,
) -> Result<(Message, u64)> {
    let built = build_wsol_instructions(owner, direction, amount, balance, exists, rent)?;
    Ok((
        Message::new(&built.instructions, Some(owner)),
        built.temporary_rent_lamports,
    ))
}

/// Constructed only after quote, balance, fee and instruction validation.
pub(crate) struct UnsignedWsolTransaction {
    message: Message,
    owner: Pubkey,
    temporary_signer: Option<Keypair>,
    authorization: WsolAuthorization,
}

impl UnsignedWsolTransaction {
    fn validate(&self, wallet: &Pubkey, quote: &WsolQuoteSnapshot, now_ms: u64) -> Result<()> {
        if self.owner != *wallet || self.message.account_keys.first() != Some(wallet) {
            bail!(WsolFailure::integrity(
                "loaded wallet signer does not match quote"
            ));
        }
        let mut expected = vec![*wallet];
        if let Some(signer) = &self.temporary_signer {
            expected.push(signer.pubkey());
        }
        let count = usize::from(self.message.header.num_required_signatures);
        if count != expected.len()
            || self.message.account_keys.get(..count) != Some(expected.as_slice())
        {
            bail!(WsolFailure::integrity(
                "WSOL conversion message has unexpected signing accounts"
            ));
        }
        self.authorization.validate(
            wallet,
            quote,
            &self.message,
            self.temporary_signer.as_ref().map(Signer::pubkey),
            now_ms,
        )
    }

    fn into_signing_parts(
        self,
        wallet: &Pubkey,
        quote: &WsolQuoteSnapshot,
        now_ms: u64,
    ) -> Result<(Message, Option<Keypair>)> {
        self.validate(wallet, quote, now_ms)?;
        Ok((self.message, self.temporary_signer))
    }

    #[cfg(test)]
    pub(crate) async fn simulate(
        &self,
        rpc: &solana_client::nonblocking::rpc_client::RpcClient,
        quote: &WsolQuoteSnapshot,
    ) -> Result<solana_client::rpc_response::RpcSimulateTransactionResult> {
        self.validate(
            &self.owner,
            quote,
            crate::application::wsol_submission::current_time_ms()?,
        )?;
        let transaction = Transaction::new_unsigned(self.message.clone());
        let simulation = rpc
            .simulate_transaction_with_config(
                &transaction,
                unsigned_wsol_simulation_config(rpc.commitment()),
            )
            .await?;
        if let Some(err) = simulation.value.err.as_ref() {
            bail!(WsolFailure::rejected(format!(
                "WSOL conversion simulation failed: {err}"
            )));
        }
        Ok(simulation.value)
    }

    #[cfg(test)]
    pub(crate) fn unverified_for_policy_test() -> Self {
        Self {
            message: Message::default(),
            owner: Pubkey::default(),
            temporary_signer: None,
            authorization: WsolAuthorization {
                quote: WsolQuoteSnapshot::from(&crate::conversion::contract_tests::quote()),
                sol_balance_lamports: 0,
                wsol_balance_lamports: 0,
                account_rent_lamports: 0,
                rent_refund_lamports: 0,
                network_fee_lamports: 0,
                blockhash: Default::default(),
            },
        }
    }
}

#[cfg(test)]
fn unsigned_wsol_simulation_config(
    commitment: CommitmentConfig,
) -> solana_client::rpc_config::RpcSimulateTransactionConfig {
    solana_client::rpc_config::RpcSimulateTransactionConfig {
        sig_verify: false,
        commitment: Some(commitment),
        ..Default::default()
    }
}

#[cfg(test)]
pub(crate) async fn prepare_unsigned_wsol_transaction(
    owner: Pubkey,
    rpc: &RpcClient,
    quote: &WsolQuoteSnapshot,
) -> Result<UnsignedWsolTransaction> {
    if owner.to_string() != quote.wallet_pubkey {
        bail!(WsolFailure::integrity(
            "loaded wallet signer does not match quote"
        ));
    }
    let mint = crate::models::WSOL_MINT.parse::<Pubkey>()?;
    let token_program = spl_token_interface::ID;
    let ata = get_associated_token_address_with_program_id(&owner, &mint, &token_program);
    let sol_balance = rpc.get_balance(&owner).await?;
    let ata_account = rpc
        .get_account_with_commitment(&ata, CommitmentConfig::processed())
        .await?
        .value;
    if ata_account.is_some() != quote.ata_exists {
        bail!(WsolFailure::rejected(
            "WSOL account state changed; request a new quote"
        ));
    }
    let wsol_balance = match ata_account.as_ref() {
        Some(account) => {
            if account.owner != token_program {
                bail!(WsolFailure::integrity("WSOL ATA program owner changed"));
            }
            let token = spl_token_interface::state::Account::unpack(&account.data)
                .map_err(WsolFailure::integrity)?;
            if token.mint != mint || token.owner != owner || token.is_native.is_none() {
                bail!(WsolFailure::integrity("WSOL ATA mint or owner changed"));
            }
            token.amount
        }
        None => 0,
    };
    let current_refund = ata_account
        .as_ref()
        .map(|account| account.lamports.saturating_sub(wsol_balance))
        .unwrap_or(0);
    if quote.direction == Direction::Unwrap && quote.amount_lamports == wsol_balance {
        if current_refund != quote.rent_refund_lamports {
            bail!(WsolFailure::rejected(
                "WSOL account rent changed; request a new quote"
            ));
        }
    }
    let current_source_balance = match quote.direction {
        Direction::Wrap => sol_balance,
        Direction::Unwrap => wsol_balance,
    };
    quote
        .validate_current_balance(current_source_balance)
        .map_err(WsolFailure::rejected)?;
    let account_rent = rpc
        .get_minimum_balance_for_rent_exemption(spl_token_interface::state::Account::LEN)
        .await?;
    let built = build_wsol_instructions(
        &owner,
        quote.direction,
        quote.amount_lamports,
        wsol_balance,
        ata_account.is_some(),
        account_rent,
    )?;
    if built.temporary_rent_lamports != quote.temporary_rent_lamports {
        bail!(WsolFailure::rejected(
            "temporary rent changed; request a new quote"
        ));
    }
    let blockhash = rpc.get_latest_blockhash().await?;
    let mut message = Message::new(&built.instructions, Some(&owner));
    message.recent_blockhash = blockhash;
    let fee = rpc.get_fee_for_message(&message).await?;
    let authorization = WsolAuthorization {
        quote: quote.clone(),
        sol_balance_lamports: sol_balance,
        wsol_balance_lamports: wsol_balance,
        account_rent_lamports: account_rent,
        rent_refund_lamports: current_refund,
        network_fee_lamports: fee,
        blockhash,
    };
    authorization.validate_budget()?;
    Ok(UnsignedWsolTransaction {
        message,
        owner,
        temporary_signer: built.temporary_account,
        authorization,
    })
}

/// Only this SDK module constructs it; callers cannot edit the signed message or journal bytes.
pub(crate) struct SignedWsolTransaction {
    owner: Pubkey,
    authorized_message: Message,
    transaction: Transaction,
    signature: String,
    signed_bytes: Vec<u8>,
}

impl SignedWsolTransaction {
    pub(crate) fn signature(&self) -> &str {
        &self.signature
    }

    pub(crate) fn signed_bytes(&self) -> &[u8] {
        &self.signed_bytes
    }

    fn validate_snapshot(&self, wallet: &Pubkey) -> Result<()> {
        let count = usize::from(self.authorized_message.header.num_required_signatures);
        if self.owner != *wallet
            || self.authorized_message.account_keys.first() != Some(wallet)
            || self.transaction.message != self.authorized_message
            || count == 0
            || self.transaction.signatures.len() != count
            || self
                .transaction
                .signatures
                .first()
                .map(ToString::to_string)
                .as_deref()
                != Some(self.signature.as_str())
            || bincode::serialize(&self.transaction)? != self.signed_bytes
        {
            bail!("signed WSOL conversion does not match authorized transaction");
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) async fn simulate(&self, rpc: &RpcClient) -> Result<RpcSimulateTransactionResult> {
        self.validate_snapshot(&self.owner)?;
        Ok(rpc.simulate_transaction(&self.transaction).await?.value)
    }
}

impl SdkTradeClient {
    pub(crate) fn sign_prepared_wsol(
        &self,
        prepared: UnsignedWsolTransaction,
        quote: &WsolQuoteSnapshot,
    ) -> Result<SignedWsolTransaction> {
        self.ensure_broadcast_allowed()?;
        let owner = self.wallet();
        let (message, temporary_signer) = prepared.into_signing_parts(
            &owner,
            quote,
            crate::application::wsol_submission::current_time_ms()?,
        )?;
        let blockhash = message.recent_blockhash;
        let signer = self.wallet_signer();
        let mut transaction = Transaction::new_unsigned(message.clone());
        match temporary_signer.as_ref() {
            Some(temp) => transaction.try_sign(&[signer.as_ref(), temp], blockhash),
            None => transaction.try_sign(&[signer.as_ref()], blockhash),
        }
        .context("failed to sign WSOL conversion transaction")?;
        let signed = SignedWsolTransaction {
            owner,
            authorized_message: message,
            signature: transaction.signatures[0].to_string(),
            signed_bytes: bincode::serialize(&transaction)?,
            transaction,
        };
        signed.validate_snapshot(&owner)?;
        Ok(signed)
    }

    pub(crate) async fn send_signed_wsol(
        &self,
        signed: &SignedWsolTransaction,
        rpc: &RpcClient,
    ) -> Result<Signature> {
        self.ensure_broadcast_allowed()?;
        signed.validate_snapshot(&self.wallet())?;
        Ok(rpc.send_transaction(&signed.transaction).await?)
    }
}

#[cfg(test)]
mod authorization_tests;
#[cfg(test)]
mod tests {
    use super::*;

    fn preparation_rpc(sol: u64, token_balance: Option<u64>, fee: u64, corrupt: u8) -> RpcClient {
        use base64::Engine;
        use serde_json::json;
        use solana_client::rpc_request::RpcRequest;
        let owner = Pubkey::from([1; 32]);
        let account = token_balance.map(|amount| {
            let token = spl_token_interface::state::Account {
                mint: if corrupt == 2 { Pubkey::from([3; 32]) } else { crate::models::WSOL_MINT.parse().unwrap() },
                owner: if corrupt == 3 { Pubkey::from([3; 32]) } else { owner },
                amount,
                state: spl_token_interface::state::AccountState::Initialized,
                is_native: if corrupt == 4 { None } else { Some(2_039_280) }.into(),
                ..Default::default()
            };
            let mut data = vec![0; spl_token_interface::state::Account::LEN];
            spl_token_interface::state::Account::pack(token, &mut data).unwrap();
            json!({"lamports":amount+2_039_280,"owner":if corrupt == 1 { Pubkey::from([3;32]) } else { spl_token_interface::ID }.to_string(),
                "data":[base64::engine::general_purpose::STANDARD.encode(data),"base64"],"executable":false,"rentEpoch":0,"space":165})
        });
        let mut mocks = std::collections::HashMap::new();
        mocks.insert(
            RpcRequest::GetBalance,
            json!({"context":{"slot":42},"value":sol}),
        );
        mocks.insert(
            RpcRequest::GetAccountInfo,
            json!({"context":{"slot":42},"value":account}),
        );
        mocks.insert(
            RpcRequest::GetMinimumBalanceForRentExemption,
            json!(2_039_280),
        );
        mocks.insert(RpcRequest::GetLatestBlockhash, json!({"context":{"slot":42},"value":{"blockhash":solana_sdk::hash::Hash::new_from_array([8;32]).to_string(),"lastValidBlockHeight":100}}));
        mocks.insert(
            RpcRequest::GetFeeForMessage,
            json!({"context":{"slot":42},"value":fee}),
        );
        RpcClient::new_mock_with_mocks("succeeds".into(), mocks)
    }

    fn snapshot(direction: Direction, amount: u64, balance: Option<u64>) -> WsolQuoteSnapshot {
        let mut quote = WsolQuoteSnapshot::from(&crate::conversion::contract_tests::quote());
        quote.direction = direction;
        quote.amount_lamports = amount;
        quote.ata_exists = balance.is_some();
        quote.source_balance_lamports = if direction == Direction::Wrap {
            10_000_000
        } else {
            balance.unwrap_or(0)
        };
        quote.temporary_rent_lamports = if direction == Direction::Unwrap && balance != Some(amount)
        {
            2_039_280
        } else {
            0
        };
        quote.rent_refund_lamports = if direction == Direction::Unwrap && balance == Some(amount) {
            2_039_280
        } else {
            0
        };
        quote.expires_at_ms = 4_000_000_000_000;
        quote
    }

    fn quote_input(
        direction: Direction,
        amount: Option<u64>,
        all: bool,
    ) -> crate::application::wsol_queries::WsolQuoteInput {
        crate::application::wsol_queries::WsolQuoteInput {
            wallet_id: "fixture-wallet".into(),
            owner_type: "telegram".into(),
            owner_id: "42".into(),
            direction,
            amount_lamports: amount,
            all,
        }
    }

    #[tokio::test]
    async fn wsol_real_quote_estimation_covers_new_existing_full_and_partial_accounts_without_signing(
    ) {
        for (direction, balance, all, expected_rent, expected_refund) in [
            (Direction::Wrap, None, false, 0, 0),
            (Direction::Wrap, Some(2_000_000), false, 0, 0),
            (Direction::Unwrap, Some(2_000_000), true, 0, 2_039_280),
            (Direction::Unwrap, Some(2_000_000), false, 2_039_280, 0),
        ] {
            let input = quote_input(direction, if all { None } else { Some(1_000_000) }, all);
            let result = super::queries::estimate(
                Pubkey::from([1; 32]),
                &preparation_rpc(10_000_000, balance, 5_000, 0),
                &input,
            )
            .await
            .unwrap();
            assert_eq!(
                result.amount_lamports,
                if all { 2_000_000 } else { 1_000_000 }
            );
            assert_eq!(
                result.source_balance_lamports,
                if direction == Direction::Wrap {
                    10_000_000
                } else {
                    2_000_000
                }
            );
            assert_eq!(result.ata_exists, balance.is_some());
            assert_eq!(result.estimated_network_fee_lamports, 5_000);
            assert_eq!(result.temporary_rent_lamports, expected_rent);
            assert_eq!(result.rent_refund_lamports, expected_refund);
        }
    }

    #[tokio::test]
    async fn wsol_real_quote_estimation_preserves_identity_missing_amount_and_budget_rejections() {
        let owner = Pubkey::from([1; 32]);
        let input = quote_input(Direction::Unwrap, Some(1_000_000), false);
        for corrupt in 1..=4 {
            assert_eq!(
                super::queries::estimate(
                    owner,
                    &preparation_rpc(10_000_000, Some(2_000_000), 5_000, corrupt),
                    &input
                )
                .await
                .err()
                .unwrap()
                .to_string(),
                if corrupt == 1 {
                    "WSOL ATA has an unexpected program owner"
                } else {
                    "WSOL ATA mint or owner does not match wallet"
                }
            );
        }
        let missing = quote_input(Direction::Wrap, None, false);
        assert_eq!(
            super::queries::estimate(
                owner,
                &preparation_rpc(10_000_000, None, 5_000, 0),
                &missing
            )
            .await
            .err()
            .unwrap()
            .to_string(),
            "amount_lamports is required"
        );
        let input = quote_input(Direction::Wrap, Some(1_000_000), false);
        let minimum = 1_000_000 + 2_039_280 + 5_000 + 100_000;
        assert!(
            super::queries::estimate(owner, &preparation_rpc(minimum, None, 5_000, 0), &input)
                .await
                .is_ok()
        );
        assert_eq!(
            super::queries::estimate(owner, &preparation_rpc(minimum - 1, None, 5_000, 0), &input)
                .await
                .err()
                .unwrap()
                .to_string(),
            "SOL balance insufficient for amount, fee, rent and reserve"
        );
        assert_eq!(
            super::queries::estimate(owner, &preparation_rpc(u64::MAX, None, u64::MAX, 0), &input)
                .await
                .err()
                .unwrap()
                .to_string(),
            "conversion SOL budget overflow"
        );
    }

    #[tokio::test]
    async fn wsol_real_unsigned_preparation_uses_only_read_mock_rpc_for_four_account_shapes() {
        for (direction, amount, balance, instruction_count, signer_count) in [
            (Direction::Wrap, 1_000_000, None, 3, 1),
            (Direction::Wrap, 1_000_000, Some(2_000_000), 2, 1),
            (Direction::Unwrap, 2_000_000, Some(2_000_000), 1, 1),
            (Direction::Unwrap, 1_000_000, Some(2_000_000), 4, 2),
        ] {
            let quote = snapshot(direction, amount, balance);
            let prepared = prepare_unsigned_wsol_transaction(
                Pubkey::from([1; 32]),
                &preparation_rpc(10_000_000, balance, 5_000, 0),
                &quote,
            )
            .await
            .unwrap();
            prepared
                .validate(&Pubkey::from([1; 32]), &quote, 1)
                .unwrap();
            assert_eq!(prepared.message.instructions.len(), instruction_count);
            assert_eq!(
                prepared.message.header.num_required_signatures,
                signer_count
            );
            assert_eq!(
                prepared.message.recent_blockhash,
                solana_sdk::hash::Hash::new_from_array([8; 32])
            );
            assert!(Transaction::new_unsigned(prepared.message)
                .signatures
                .iter()
                .all(|signature| *signature == Signature::default()));
        }
    }

    #[tokio::test]
    async fn wsol_real_unsigned_preparation_rejects_ata_identity_and_changed_snapshot() {
        for corrupt in 1..=4 {
            let quote = snapshot(Direction::Unwrap, 1_000_000, Some(2_000_000));
            let error = prepare_unsigned_wsol_transaction(
                Pubkey::from([1; 32]),
                &preparation_rpc(10_000_000, Some(2_000_000), 5_000, corrupt),
                &quote,
            )
            .await
            .err()
            .unwrap();
            assert_eq!(
                error.to_string(),
                if corrupt == 1 {
                    "WSOL ATA program owner changed"
                } else {
                    "WSOL ATA mint or owner changed"
                }
            );
        }
        for case in 0..5 {
            let mut quote = snapshot(Direction::Unwrap, 1_000_000, Some(2_000_000));
            let message = match case {
                0 => {
                    quote.ata_exists = false;
                    "WSOL account state changed; request a new quote"
                }
                1 => {
                    quote.source_balance_lamports -= 1;
                    "conversion source balance changed; request a new quote"
                }
                2 => {
                    quote.temporary_rent_lamports -= 1;
                    "temporary rent changed; request a new quote"
                }
                3 => {
                    quote.estimated_network_fee_lamports = 4_999;
                    "network fee increased; request a new quote"
                }
                _ => {
                    quote.amount_lamports = 2_000_000;
                    quote.rent_refund_lamports = 1;
                    "WSOL account rent changed; request a new quote"
                }
            };
            assert_eq!(
                prepare_unsigned_wsol_transaction(
                    Pubkey::from([1; 32]),
                    &preparation_rpc(10_000_000, Some(2_000_000), 5_000, 0),
                    &quote
                )
                .await
                .err()
                .unwrap()
                .to_string(),
                message
            );
        }
    }

    pub(super) fn unsigned_case(
        direction: Direction,
        amount: u64,
        balance: u64,
        exists: bool,
    ) -> super::UnsignedWsolTransaction {
        let owner = solana_sdk::pubkey::Pubkey::new_from_array([7; 32]);
        let built =
            super::build_wsol_instructions(&owner, direction, amount, balance, exists, 2_039_280)
                .unwrap();
        let blockhash = solana_sdk::hash::Hash::new_from_array([8; 32]);
        let mut message = super::Message::new(&built.instructions, Some(&owner));
        message.recent_blockhash = blockhash;
        assert_eq!(
            message,
            super::Message::new_with_blockhash(&built.instructions, Some(&owner), &blockhash)
        );
        let mut quote = snapshot(direction, amount, exists.then_some(balance));
        quote.wallet_pubkey = owner.to_string();
        super::UnsignedWsolTransaction {
            message,
            owner,
            temporary_signer: built.temporary_account,
            authorization: WsolAuthorization {
                quote,
                sol_balance_lamports: 10_000_000,
                wsol_balance_lamports: balance,
                account_rent_lamports: 2_039_280,
                rent_refund_lamports: if exists { 2_039_280 } else { 0 },
                network_fee_lamports: 5_000,
                blockhash,
            },
        }
    }

    #[test]
    fn unsigned_wrap_and_unwrap_keep_original_messages_without_wallet_keys() {
        for (direction, amount, balance, exists, signers) in [
            (Direction::Wrap, 50, 0, false, 1),
            (Direction::Wrap, 50, 0, true, 1),
            (Direction::Unwrap, 100, 100, true, 1),
            (Direction::Unwrap, 50, 100, true, 2),
        ] {
            let prepared = unsigned_case(direction, amount, balance, exists);
            let transaction = super::Transaction::new_unsigned(prepared.message.clone());
            assert_eq!(transaction.signatures.len(), signers);
            assert!(transaction
                .signatures
                .iter()
                .all(|s| *s == solana_sdk::signature::Signature::default()));
            let owner = prepared.owner;
            let quote = prepared.authorization.quote.clone();
            let (message, temporary) = prepared.into_signing_parts(&owner, &quote, 1).unwrap();
            assert_eq!(message, transaction.message);
            assert_eq!(temporary.is_some(), signers == 2);
        }
    }

    #[test]
    fn unsigned_wsol_binding_rejects_wrong_wallet_or_signing_accounts() {
        let prepared = unsigned_case(Direction::Wrap, 50, 0, false);
        let quote = prepared.authorization.quote.clone();
        assert!(prepared
            .into_signing_parts(
                &solana_sdk::pubkey::Pubkey::new_from_array([9; 32]),
                &quote,
                1
            )
            .is_err());
        for variant in 0..3 {
            let mut prepared = unsigned_case(Direction::Unwrap, 50, 100, true);
            let owner = prepared.owner;
            let quote = prepared.authorization.quote.clone();
            match variant {
                0 => prepared.temporary_signer = None,
                1 => prepared.message.header.num_required_signatures = 1,
                2 => {
                    prepared.message.account_keys[1] =
                        solana_sdk::pubkey::Pubkey::new_from_array([9; 32])
                }
                _ => unreachable!(),
            }
            assert!(prepared.into_signing_parts(&owner, &quote, 1).is_err());
        }
    }

    fn simulation_rpc(
        error: serde_json::Value,
    ) -> solana_client::nonblocking::rpc_client::RpcClient {
        let mut mocks = std::collections::HashMap::new();
        mocks.insert(solana_client::rpc_request::RpcRequest::SimulateTransaction,serde_json::json!({
            "context":{"slot":42},"value":{"err":error,"logs":[],"accounts":null,"unitsConsumed":999,"returnData":null,"innerInstructions":null,"replacementBlockhash":null,"loadedAccountsDataSize":0}
        }));
        solana_client::nonblocking::rpc_client::RpcClient::new_mock_with_mocks(
            "fails".into(),
            mocks,
        )
    }

    #[tokio::test]
    async fn wsol_quote_unsigned_boundary_sdk_separates_parameter_account_and_rpc_failures_without_signing(
    ) {
        use crate::application::wsol_failure::WsolFailureKind;
        let owner = Pubkey::from([1; 32]);
        for corrupt in 1..=4 {
            let input = quote_input(Direction::Unwrap, Some(1_000_000), false);
            let error = super::queries::estimate(
                owner,
                &preparation_rpc(10_000_000, Some(2_000_000), 5_000, corrupt),
                &input,
            )
            .await
            .err()
            .unwrap();
            assert_eq!(
                error.downcast_ref::<WsolFailure>().unwrap().kind,
                WsolFailureKind::Integrity
            );
        }
        for input in [
            quote_input(Direction::Wrap, None, false),
            quote_input(Direction::Wrap, Some(0), false),
            quote_input(Direction::Unwrap, Some(3_000_000), false),
        ] {
            let error = super::queries::estimate(
                owner,
                &preparation_rpc(10_000_000, Some(2_000_000), 5_000, 0),
                &input,
            )
            .await
            .err()
            .unwrap();
            assert_eq!(
                error.downcast_ref::<WsolFailure>().unwrap().kind,
                WsolFailureKind::Rejected
            );
        }
        let prepared = unsigned_case(Direction::Wrap, 50, 0, false);
        let error = prepared
            .simulate(
                &simulation_rpc(serde_json::json!({"InstructionError":[0,{"Custom":42}]})),
                &prepared.authorization.quote,
            )
            .await
            .err()
            .unwrap();
        assert_eq!(
            error.downcast_ref::<WsolFailure>().unwrap().kind,
            WsolFailureKind::Rejected
        );
        let error = prepared
            .simulate(
                &RpcClient::new_mock("fails".into()),
                &prepared.authorization.quote,
            )
            .await
            .err()
            .unwrap();
        assert!(error.downcast_ref::<WsolFailure>().is_none());
        let mut wrong = prepared.authorization.quote.clone();
        wrong.amount_lamports += 1;
        let error = prepared
            .simulate(&simulation_rpc(serde_json::Value::Null), &wrong)
            .await
            .err()
            .unwrap();
        assert_eq!(
            error.downcast_ref::<WsolFailure>().unwrap().kind,
            WsolFailureKind::Integrity
        );
    }

    #[tokio::test]
    async fn unsigned_wsol_simulation_handles_all_modes_without_owner_signatures() {
        for commitment in [
            super::CommitmentConfig::processed(),
            super::CommitmentConfig::confirmed(),
            super::CommitmentConfig::finalized(),
        ] {
            let config = super::unsigned_wsol_simulation_config(commitment);
            assert!(!config.sig_verify);
            assert_eq!(config.commitment, Some(commitment));
        }
        for (direction, amount, balance, exists) in [
            (Direction::Wrap, 50, 0, false),
            (Direction::Wrap, 50, 0, true),
            (Direction::Unwrap, 100, 100, true),
            (Direction::Unwrap, 50, 100, true),
        ] {
            let prepared = unsigned_case(direction, amount, balance, exists);
            let result = prepared
                .simulate(
                    &simulation_rpc(serde_json::Value::Null),
                    &prepared.authorization.quote,
                )
                .await
                .unwrap();
            assert_eq!(result.units_consumed, Some(999));
        }
    }

    #[tokio::test]
    async fn unsigned_wsol_rpc_and_program_errors_do_not_become_successful_simulation() {
        let prepared = unsigned_case(Direction::Wrap, 50, 0, false);
        let rpc = simulation_rpc(serde_json::json!({"InstructionError":[0,{"Custom":42}]}));
        assert!(prepared
            .simulate(&rpc, &prepared.authorization.quote)
            .await
            .unwrap_err()
            .to_string()
            .starts_with("WSOL conversion simulation failed:"));
        let rpc = solana_client::nonblocking::rpc_client::RpcClient::new_mock("fails".into());
        assert!(prepared
            .simulate(&rpc, &prepared.authorization.quote)
            .await
            .is_err());
    }

    fn artifact(two_signers: bool) -> SignedWsolTransaction {
        let owner = Pubkey::from([1; 32]);
        let other = Pubkey::from([2; 32]);
        let mut instructions = vec![solana_system_interface::instruction::transfer(
            &owner, &other, 1,
        )];
        if two_signers {
            // Synthetic shape only, not a WSOL instruction-authorization fixture.
            instructions.push(solana_system_interface::instruction::transfer(
                &other, &owner, 1,
            ));
        }
        let message = Message::new(&instructions, Some(&owner));
        let mut transaction = Transaction::new_unsigned(message.clone());
        transaction.signatures.fill(Signature::from([7; 64]));
        SignedWsolTransaction {
            owner,
            authorized_message: message,
            signature: transaction.signatures[0].to_string(),
            signed_bytes: bincode::serialize(&transaction).unwrap(),
            transaction,
        }
    }

    #[test]
    fn wsol_signed_snapshot_and_journal_bytes_bind_one_or_two_signer_messages() {
        for two in [false, true] {
            let signed = artifact(two);
            signed.validate_snapshot(&signed.owner).unwrap();
            assert_eq!(signed.signature(), Signature::from([7; 64]).to_string());
            assert_eq!(
                signed.signed_bytes(),
                bincode::serialize(&signed.transaction).unwrap()
            );
            assert_eq!(signed.transaction.signatures.len(), if two { 2 } else { 1 });
        }
    }

    #[test]
    fn wsol_signed_snapshot_rejects_internal_tampering_before_rpc() {
        for case in 0..8 {
            let mut signed = artifact(false);
            let original_owner = signed.owner;
            match case {
                0 => signed.owner = Pubkey::from([3; 32]),
                1 => signed.authorized_message.account_keys[0] = Pubkey::from([3; 32]),
                2 => signed.transaction.message.instructions[0].data.push(0),
                3 => signed.transaction.signatures.clear(),
                4 => signed.transaction.signatures.push(Signature::default()),
                5 => signed.signature = "replacement".into(),
                6 => signed.signed_bytes.push(0),
                _ => signed.transaction.signatures[0] = Signature::from([8; 64]),
            }
            assert!(
                signed.validate_snapshot(&original_owner).is_err(),
                "case {case}"
            );
        }
        let signed = artifact(false);
        assert!(signed.validate_snapshot(&Pubkey::from([4; 32])).is_err());
    }

    #[tokio::test]
    async fn wsol_signed_simulation_keeps_program_errors_and_rejects_bad_snapshot_before_rpc() {
        for program_error in [
            serde_json::Value::Null,
            serde_json::json!({"InstructionError":[0,{"Custom":42}]}),
        ] {
            let mut mocks = std::collections::HashMap::new();
            mocks.insert(
                solana_client::rpc_request::RpcRequest::SimulateTransaction,
                serde_json::json!({
                    "context":{"slot":42},"value":{"err":program_error,"logs":[],"accounts":null,
                    "unitsConsumed":999,"returnData":null,"innerInstructions":null,
                    "replacementBlockhash":null,"loadedAccountsDataSize":0}
                }),
            );
            let rpc = RpcClient::new_mock_with_mocks("fails".into(), mocks);
            let result = artifact(false).simulate(&rpc).await.unwrap();
            assert_eq!(serde_json::to_value(result.err).unwrap(), program_error);
            assert_eq!(result.units_consumed, Some(999));
        }
        let rpc = RpcClient::new_mock("fails".into());
        assert!(artifact(false).simulate(&rpc).await.is_err());
        let mut changed = artifact(false);
        changed.signed_bytes.clear();
        assert_eq!(
            changed.simulate(&rpc).await.unwrap_err().to_string(),
            "signed WSOL conversion does not match authorized transaction"
        );
    }

    #[tokio::test]
    async fn wsol_signed_send_is_blocked_before_wallet_and_rpc_access() {
        let client = SdkTradeClient {
            lanes: vec![],
            sol_hop_cache: Default::default(),
            swqos_lanes: vec![],
            mev_protection: false,
            require_mev_submit: false,
            rpc_count: 0,
            broadcast_policy: crate::domain::execution_policy::BroadcastPolicy::Disabled,
        };
        let rpc = RpcClient::new_mock("fails".into());
        assert_eq!(
            client
                .send_signed_wsol(&artifact(false), &rpc)
                .await
                .unwrap_err()
                .to_string(),
            crate::domain::execution_policy::BROADCAST_DISABLED_ERROR
        );
    }
}
