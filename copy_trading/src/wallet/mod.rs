use crate::{
    crypto::{EncryptedSecret, SecretBox},
    db::{models::WalletRecord, Database},
    trade_client::{
        ManagedWalletResponse, SolTransferResponse, TradeServiceClient, VanityJobResponse,
        WalletBalancesResponse,
    },
};
use anyhow::{Context, Result};
use solana_sdk::signature::Keypair;

#[derive(Clone)]
pub struct WalletService {
    db: Database,
    secret_box: SecretBox,
    trade_client: TradeServiceClient,
}

impl WalletService {
    pub fn new(db: Database, secret_box: SecretBox, trade_client: TradeServiceClient) -> Self {
        Self {
            db,
            secret_box,
            trade_client,
        }
    }

    pub async fn create_wallet(&self, user_id: i64, label: Option<&str>) -> Result<WalletRecord> {
        let label = label.unwrap_or("Wallet");
        let remote = self.trade_client.create_wallet(user_id, label).await?;
        let wallet = match self.persist_remote_wallet(user_id, label, &remote).await {
            Ok(wallet) => wallet,
            Err(err) => {
                let _ = self.trade_client.delete_wallet(&remote.wallet_id).await;
                return Err(err);
            }
        };
        self.db
            .audit(
                Some(user_id),
                "wallet.create",
                Some("wallet"),
                Some(&wallet.id.to_string()),
                Some(&format!(
                    "{{\"external_wallet_id\":\"{}\"}}",
                    remote.wallet_id
                )),
            )
            .await?;
        Ok(wallet)
    }

    pub async fn import_wallet(
        &self,
        user_id: i64,
        private_key_text: &str,
        label: Option<&str>,
    ) -> Result<WalletRecord> {
        let keypair = parse_keypair(private_key_text)?;
        let private_key_base58 = bs58::encode(keypair.to_bytes()).into_string();
        let label = label.unwrap_or("Imported");
        let remote = self
            .trade_client
            .import_wallet(user_id, label, &private_key_base58)
            .await?;
        let wallet = self.persist_remote_wallet(user_id, label, &remote).await?;
        self.db
            .audit(
                Some(user_id),
                "wallet.import",
                Some("wallet"),
                Some(&wallet.id.to_string()),
                Some(&format!(
                    "{{\"external_wallet_id\":\"{}\"}}",
                    remote.wallet_id
                )),
            )
            .await?;
        Ok(wallet)
    }

    pub async fn export_private_key_base58(&self, user_id: i64, wallet_id: i64) -> Result<String> {
        let wallet = self.db.get_wallet(user_id, wallet_id).await?;
        let private_key = match wallet.external_wallet_id.as_deref() {
            Some(external_wallet_id) => self.trade_client.export_wallet(external_wallet_id).await?,
            None => self.export_legacy_private_key(wallet_id).await?,
        };
        self.db
            .audit(
                Some(user_id),
                "wallet.export_private_key",
                Some("wallet"),
                Some(&wallet_id.to_string()),
                Some("{\"channel\":\"telegram\"}"),
            )
            .await?;
        Ok(private_key)
    }

    pub async fn rename_wallet(&self, user_id: i64, wallet_id: i64, label: &str) -> Result<()> {
        let wallet = self.db.get_wallet(user_id, wallet_id).await?;
        if let Some(external_wallet_id) = wallet.external_wallet_id.as_deref() {
            self.trade_client
                .rename_wallet(external_wallet_id, label)
                .await?;
        }
        self.db.rename_wallet(user_id, wallet_id, label).await
    }

    pub async fn delete_wallet(&self, user_id: i64, wallet_id: i64) -> Result<()> {
        let wallet = self.db.get_wallet(user_id, wallet_id).await?;
        if let Some(external_wallet_id) = wallet.external_wallet_id.as_deref() {
            self.trade_client.delete_wallet(external_wallet_id).await?;
        }
        self.db.soft_delete_wallet(user_id, wallet_id).await
    }

    pub async fn balances(&self, user_id: i64) -> Result<WalletBalancesResponse> {
        self.trade_client.wallet_balances(user_id).await
    }

    pub async fn create_vanity_job(
        &self,
        user_id: i64,
        side: &str,
        pattern: &str,
    ) -> Result<VanityJobResponse> {
        self.trade_client
            .create_vanity_job(user_id, side, pattern)
            .await
    }

    pub async fn vanity_job(&self, user_id: i64, job_id: &str) -> Result<VanityJobResponse> {
        self.trade_client.vanity_job(user_id, job_id).await
    }

    pub async fn cancel_vanity_job(&self, user_id: i64, job_id: &str) -> Result<VanityJobResponse> {
        self.trade_client.cancel_vanity_job(user_id, job_id).await
    }

    pub async fn transfer_sol(
        &self,
        user_id: i64,
        wallet_id: i64,
        to_wallet_id: Option<i64>,
        to_address: Option<&str>,
        amount_lamports: Option<u64>,
        percent: Option<u64>,
    ) -> Result<SolTransferResponse> {
        let source = self.db.get_wallet(user_id, wallet_id).await?;
        let source_external = source
            .external_wallet_id
            .as_deref()
            .context("source wallet is not bound to trade_service")?;
        let target = match to_wallet_id {
            Some(id) => Some(self.db.get_wallet(user_id, id).await?),
            None => None,
        };
        let target_external = target
            .as_ref()
            .and_then(|wallet| wallet.external_wallet_id.as_deref());
        self.trade_client
            .transfer_sol(
                user_id,
                source_external,
                target_external,
                to_address,
                amount_lamports,
                percent,
                format!("tg:{}", uuid::Uuid::new_v4()),
            )
            .await
    }

    pub async fn migrate_legacy_wallets(&self) -> Result<usize> {
        let wallets = self.db.list_unbound_wallets().await?;
        let mut migrated = 0;
        for wallet in wallets {
            let private_key = self.export_legacy_private_key(wallet.id).await?;
            let remote = self
                .trade_client
                .import_wallet(wallet.user_id, &wallet.label, &private_key)
                .await?;
            if remote.pubkey != wallet.public_key {
                anyhow::bail!(
                    "migrated wallet {} pubkey mismatch: expected {}, got {}",
                    wallet.id,
                    wallet.public_key,
                    remote.pubkey
                );
            }
            self.db
                .bind_external_wallet(wallet.user_id, wallet.id, &remote.wallet_id)
                .await?;
            self.db
                .audit(
                    Some(wallet.user_id),
                    "wallet.migrate_to_trade_service",
                    Some("wallet"),
                    Some(&wallet.id.to_string()),
                    Some(&format!(
                        "{{\"external_wallet_id\":\"{}\"}}",
                        remote.wallet_id
                    )),
                )
                .await?;
            migrated += 1;
        }
        Ok(migrated)
    }

    pub async fn sync_trade_service_wallets(&self, user_id: i64) -> Result<usize> {
        let owner_id = user_id.to_string();
        let wallets = self.trade_client.managed_wallets().await?;
        let mut visible = 0;
        for remote in wallets.into_iter().filter(|wallet| {
            wallet.enabled
                && ((wallet.owner_type == "system" && wallet.owner_id == "global")
                    || (wallet.owner_type == "copy_trading" && wallet.owner_id == owner_id))
        }) {
            self.persist_remote_wallet(user_id, &remote.label, &remote)
                .await?;
            visible += 1;
        }
        Ok(visible)
    }

    async fn persist_remote_wallet(
        &self,
        user_id: i64,
        label: &str,
        remote: &ManagedWalletResponse,
    ) -> Result<WalletRecord> {
        if let Some(existing) = self
            .db
            .get_wallet_by_public_key(user_id, &remote.pubkey, true)
            .await?
        {
            if existing.is_deleted {
                self.db
                    .restore_external_wallet(user_id, existing.id, &remote.wallet_id, label)
                    .await?;
            } else if existing.external_wallet_id.as_deref() != Some(&remote.wallet_id)
                || existing.label != label
            {
                self.db
                    .sync_external_wallet_metadata(user_id, existing.id, &remote.wallet_id, label)
                    .await?;
            }
            return self.db.get_wallet(user_id, existing.id).await;
        }

        let wallet_id = self
            .db
            .insert_external_wallet(user_id, &remote.wallet_id, label, &remote.pubkey)
            .await?;
        self.db.get_wallet(user_id, wallet_id).await
    }

    async fn export_legacy_private_key(&self, wallet_id: i64) -> Result<String> {
        let (ciphertext, nonce) = self.db.get_wallet_secret(wallet_id).await?;
        let bytes = self
            .secret_box
            .decrypt(&EncryptedSecret { ciphertext, nonce })?;
        Ok(bs58::encode(bytes).into_string())
    }
}

fn parse_keypair(input: &str) -> Result<Keypair> {
    let trimmed = input.trim();
    let bytes = if trimmed.starts_with('[') {
        serde_json::from_str::<Vec<u8>>(trimmed).context("invalid JSON private key array")?
    } else {
        bs58::decode(trimmed)
            .into_vec()
            .context("invalid base58 private key")?
    };

    let secret_bytes: [u8; 32] = match bytes.len() {
        64 => bytes[..32].try_into().context("invalid 64 byte keypair")?,
        32 => bytes
            .try_into()
            .map_err(|_| anyhow::anyhow!("invalid 32 byte secret key"))?,
        len => anyhow::bail!("invalid private key length: expected 32 or 64 bytes, got {len}"),
    };

    Ok(Keypair::new_from_array(secret_bytes))
}
