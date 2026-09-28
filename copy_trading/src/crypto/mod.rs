use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use anyhow::{Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use rand::Rng;

#[derive(Debug, Clone)]
pub struct EncryptedSecret {
    pub ciphertext: String,
    pub nonce: String,
}

#[derive(Clone)]
pub struct SecretBox {
    key: [u8; 32],
}

impl SecretBox {
    pub fn from_hex(hex_key: &str) -> Result<Self> {
        let bytes = hex::decode(hex_key.trim()).context("MASTER_KEY_HEX must be hex")?;
        if bytes.len() != 32 {
            anyhow::bail!("MASTER_KEY_HEX must decode to 32 bytes");
        }
        let mut key = [0u8; 32];
        key.copy_from_slice(&bytes);
        Ok(Self { key })
    }

    #[allow(dead_code)]
    pub fn encrypt(&self, plaintext: &[u8]) -> Result<EncryptedSecret> {
        let cipher = Aes256Gcm::new_from_slice(&self.key).context("failed to create cipher")?;
        let mut nonce_bytes = [0u8; 12];
        rand::rng().fill(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);
        let ciphertext = cipher
            .encrypt(nonce, plaintext)
            .map_err(|err| anyhow::anyhow!("secret encryption failed: {err}"))?;
        Ok(EncryptedSecret {
            ciphertext: STANDARD.encode(ciphertext),
            nonce: STANDARD.encode(nonce_bytes),
        })
    }

    pub fn decrypt(&self, secret: &EncryptedSecret) -> Result<Vec<u8>> {
        let cipher = Aes256Gcm::new_from_slice(&self.key).context("failed to create cipher")?;
        let nonce_bytes = STANDARD.decode(&secret.nonce).context("invalid nonce")?;
        let ciphertext = STANDARD
            .decode(&secret.ciphertext)
            .context("invalid ciphertext")?;
        let nonce = Nonce::from_slice(&nonce_bytes);
        cipher
            .decrypt(nonce, ciphertext.as_ref())
            .map_err(|err| anyhow::anyhow!("secret decryption failed: {err}"))
    }
}
