//! Resolved message accounts only, not protocol, spend or signing authorization.
use anyhow::{bail, Context};
use base64::Engine;
use sha2::{Digest, Sha256};
use solana_account_decoder::{UiAccount, UiAccountData, UiAccountEncoding};
use solana_address_lookup_table_interface::{
    program,
    state::{AddressLookupTable, LOOKUP_TABLE_MAX_ADDRESSES, LOOKUP_TABLE_META_SIZE},
};
use solana_client::{nonblocking::rpc_client::RpcClient, rpc_config::RpcAccountInfoConfig};
use solana_commitment_config::CommitmentConfig;
use solana_sdk::{pubkey::Pubkey, signature::Signature, transaction::VersionedTransaction};
use std::{collections::HashSet, time::Duration};

use super::pumpapi_transaction::validate_packet_structure;
use crate::{domain::trade::Address, sanitize::redact_urls};

const MAX_LOOKUP_TABLES: usize = 4;
const MAX_ACCOUNT_KEYS: usize = 256;
const MAX_INSTRUCTIONS: usize = 32;
const MAX_LOOKUP_BYTES: usize = LOOKUP_TABLE_META_SIZE + LOOKUP_TABLE_MAX_ADDRESSES * 32;
const MAX_LOOKUP_BASE64_BYTES: usize = MAX_LOOKUP_BYTES.div_ceil(3) * 4;

pub(super) struct MessageAccount {
    pub(super) address: Address,
    pub(super) signer: bool,
    // Encoded requested privilege, not a claim about runtime reserved-key demotion.
    pub(super) writable: bool,
}

pub(crate) struct ResolvedPumpApiAccounts {
    wallet: Address,
    message_hash: [u8; 32],
    accounts: Vec<MessageAccount>,
    lookup_slot: Option<u64>,
}

impl ResolvedPumpApiAccounts {
    pub(super) fn account(&self, index: u8) -> anyhow::Result<&MessageAccount> {
        self.accounts
            .get(usize::from(index))
            .context("PumpAPI instruction account is unresolved")
    }
    pub(crate) fn lookup_slot(&self) -> Option<u64> {
        self.lookup_slot
    }

    pub(crate) fn ensure_bound(
        &self,
        wallet: Address,
        transaction: &VersionedTransaction,
    ) -> anyhow::Result<()> {
        validate_unsigned_wallet(wallet, transaction)?;
        if self.wallet != wallet || self.message_hash != message_hash(transaction)? {
            bail!("PumpAPI account view does not match unsigned message and wallet");
        }
        let payer = self
            .accounts
            .first()
            .context("PumpAPI account view has no payer")?;
        if payer.address != wallet || !payer.signer || !payer.writable {
            bail!("PumpAPI account view has invalid payer privileges");
        }
        let loaded_count = transaction
            .message
            .address_table_lookups()
            .unwrap_or(&[])
            .iter()
            .map(|lookup| lookup.writable_indexes.len() + lookup.readonly_indexes.len())
            .sum::<usize>();
        if self.accounts.len() != transaction.message.static_account_keys().len() + loaded_count {
            bail!("PumpAPI account view has inconsistent account count");
        }
        for instruction in transaction.message.instructions() {
            if usize::from(instruction.program_id_index) >= self.accounts.len()
                || instruction
                    .accounts
                    .iter()
                    .any(|index| usize::from(*index) >= self.accounts.len())
            {
                bail!("PumpAPI instruction references an unresolved account index");
            }
        }
        Ok(())
    }
}

pub(crate) async fn resolve_unsigned_accounts(
    rpc: &RpcClient,
    wallet: Address,
    transaction: &VersionedTransaction,
) -> anyhow::Result<ResolvedPumpApiAccounts> {
    validate_unsigned_wallet(wallet, transaction)?;
    let message = &transaction.message;
    let instruction_count = message.instructions().len();
    if instruction_count == 0 || instruction_count > MAX_INSTRUCTIONS {
        bail!("PumpAPI transaction has unsupported instruction count");
    }
    let lookups = message.address_table_lookups().unwrap_or(&[]);
    if lookups.len() > MAX_LOOKUP_TABLES {
        bail!("PumpAPI transaction has too many address lookup tables");
    }
    let mut unique_tables = HashSet::new();
    for lookup in lookups {
        if !unique_tables.insert(lookup.account_key) {
            bail!("PumpAPI transaction has duplicate address lookup tables");
        }
    }
    let static_keys = message.static_account_keys();
    let header = message.header();
    let required = usize::from(header.num_required_signatures);
    let mut accounts = static_keys
        .iter()
        .enumerate()
        .map(|(index, key)| MessageAccount {
            address: Address::from_bytes(key.to_bytes()),
            signer: index < required,
            writable: if index < required {
                index < required - usize::from(header.num_readonly_signed_accounts)
            } else {
                index < static_keys.len() - usize::from(header.num_readonly_unsigned_accounts)
            },
        })
        .collect::<Vec<_>>();
    ensure_unique_accounts(&accounts)?;
    let lookup_slot = if lookups.is_empty() {
        None
    } else {
        let keys = lookups
            .iter()
            .map(|lookup| lookup.account_key)
            .collect::<Vec<_>>();
        let response = tokio::time::timeout(
            Duration::from_secs(5),
            rpc.get_multiple_ui_accounts_with_config(
                &keys,
                RpcAccountInfoConfig {
                    encoding: Some(UiAccountEncoding::Base64),
                    commitment: Some(CommitmentConfig::processed()),
                    data_slice: None,
                    min_context_slot: None,
                },
            ),
        )
        .await
        .context("PumpAPI lookup accounts RPC timed out")?
        .map_err(|error| anyhow::anyhow!("{}", redact_urls(&error.to_string())))?;
        if response.value.len() != lookups.len() {
            bail!("PumpAPI lookup response has inconsistent account count");
        }
        let slot = response.context.slot;
        let mut writable = Vec::new();
        let mut readonly = Vec::new();
        for (lookup, account) in lookups.iter().zip(response.value) {
            let account = account.context("PumpAPI lookup table account is missing")?;
            let data = decode_lookup_data(&account)?;
            let table = AddressLookupTable::deserialize(&data)
                .map_err(|_| anyhow::anyhow!("PumpAPI lookup table data is invalid"))?;
            if table.meta.deactivation_slot != u64::MAX {
                bail!("PumpAPI lookup table is deactivating or deactivated");
            }
            if table.meta.last_extended_slot > slot {
                bail!("PumpAPI lookup table extension is newer than RPC context");
            }
            let old_len = usize::from(table.meta.last_extended_slot_start_index);
            if old_len > table.addresses.len() {
                bail!("PumpAPI lookup table extension index is invalid");
            }
            // Deactivating tables were rejected above, so active lookup needs no SlotHashes RPC.
            let slot_hashes = Default::default();
            for (indexes, output) in [
                (&lookup.writable_indexes, &mut writable),
                (&lookup.readonly_indexes, &mut readonly),
            ] {
                let addresses = table
                    .lookup_iter(slot, indexes, &slot_hashes)
                    .map_err(|_| {
                        anyhow::anyhow!(
                            "PumpAPI lookup index is out of range or not active at RPC context"
                        )
                    })?;
                for address in addresses {
                    output.push(address.context(
                        "PumpAPI lookup index is out of range or not active at RPC context",
                    )?);
                }
            }
        }
        // Runtime order: all static keys, then all loaded writable keys, then all loaded readonly keys.
        for (keys, is_writable) in [(writable, true), (readonly, false)] {
            accounts.extend(keys.into_iter().map(|key| MessageAccount {
                address: Address::from_bytes(key.to_bytes()),
                signer: false,
                writable: is_writable,
            }));
        }
        Some(slot)
    };
    if accounts.len() > MAX_ACCOUNT_KEYS {
        bail!("PumpAPI transaction has too many resolved accounts");
    }
    ensure_unique_accounts(&accounts)?;
    Ok(ResolvedPumpApiAccounts {
        wallet,
        message_hash: message_hash(transaction)?,
        accounts,
        lookup_slot,
    })
}

fn validate_unsigned_wallet(
    wallet: Address,
    transaction: &VersionedTransaction,
) -> anyhow::Result<()> {
    validate_packet_structure(transaction)?;
    if transaction.message.header().num_required_signatures != 1
        || transaction.signatures.len() != 1
        || transaction.signatures[0] != Signature::default()
    {
        bail!("PumpAPI transaction requires exactly one unsigned wallet signature slot");
    }
    if transaction
        .message
        .static_account_keys()
        .first()
        .map(Pubkey::to_bytes)
        != Some(wallet.to_bytes())
    {
        bail!("PumpAPI transaction payer does not match request wallet");
    }
    Ok(())
}

fn message_hash(transaction: &VersionedTransaction) -> anyhow::Result<[u8; 32]> {
    Ok(Sha256::digest(
        bincode::serialize(&transaction.message).context("encode PumpAPI account view message")?,
    )
    .into())
}

fn ensure_unique_accounts(accounts: &[MessageAccount]) -> anyhow::Result<()> {
    let mut unique = HashSet::new();
    if accounts
        .iter()
        .any(|account| !unique.insert(account.address.to_bytes()))
    {
        bail!("PumpAPI transaction has duplicate resolved accounts");
    }
    Ok(())
}

fn decode_lookup_data(account: &UiAccount) -> anyhow::Result<Vec<u8>> {
    if account.owner != program::id().to_string() || account.executable || account.lamports == 0 {
        bail!("PumpAPI lookup table account has invalid owner or account state");
    }
    let UiAccountData::Binary(encoded, UiAccountEncoding::Base64) = &account.data else {
        bail!("PumpAPI lookup table response must use base64 account data");
    };
    if encoded.len() > MAX_LOOKUP_BASE64_BYTES {
        bail!("PumpAPI lookup table data exceeds address limit");
    }
    let data = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .context("PumpAPI lookup table base64 is invalid")?;
    if data.len() < LOOKUP_TABLE_META_SIZE
        || data.len() > MAX_LOOKUP_BYTES
        || (data.len() - LOOKUP_TABLE_META_SIZE) % 32 != 0
        || account
            .space
            .is_some_and(|space| space != data.len() as u64)
    {
        bail!("PumpAPI lookup table data has invalid length or address count");
    }
    Ok(data)
}

#[cfg(test)]
mod tests;
