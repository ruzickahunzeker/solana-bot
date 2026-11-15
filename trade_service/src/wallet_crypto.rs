#![allow(dead_code)]

use anyhow::{anyhow, bail, Context};
use base64::Engine;
use ring::aead::{self, Aad, LessSafeKey, Nonce, UnboundKey};
use ring::pbkdf2;
use ring::rand::{SecureRandom, SystemRandom};
use serde::{Deserialize, Serialize};
use std::num::NonZeroU32;

const FORMAT_VERSION: u8 = 1;
const ALGORITHM: &str = "AES-256-GCM";
const KDF: &str = "PBKDF2-HMAC-SHA256";
const DEFAULT_ITERATIONS: u32 = 600_000;
const KEY_LEN: usize = 32;
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 12;
const AAD: &[u8] = b"pump-trade-service-wallets-v1";

#[derive(Debug, Serialize, Deserialize)]
struct EncryptedWalletsFile {
    version: u8,
    algorithm: String,
    kdf: String,
    iterations: u32,
    salt: String,
    nonce: String,
    ciphertext: String,
}

pub fn encrypt_json(plaintext: &[u8], passphrase: &str) -> anyhow::Result<String> {
    ensure_passphrase(passphrase)?;

    let rng = SystemRandom::new();
    let mut salt = [0_u8; SALT_LEN];
    let mut nonce = [0_u8; NONCE_LEN];
    rng.fill(&mut salt).map_err(|_| anyhow!("generate salt"))?;
    rng.fill(&mut nonce)
        .map_err(|_| anyhow!("generate nonce"))?;

    let key = derive_key(passphrase, &salt, DEFAULT_ITERATIONS)?;
    let less_safe_key = LessSafeKey::new(
        UnboundKey::new(&aead::AES_256_GCM, &key).map_err(|_| anyhow!("create AES key"))?,
    );
    let nonce_value = Nonce::assume_unique_for_key(nonce);
    let mut in_out = plaintext.to_vec();
    less_safe_key
        .seal_in_place_append_tag(nonce_value, Aad::from(AAD), &mut in_out)
        .map_err(|_| anyhow!("encrypt wallets"))?;

    let file = EncryptedWalletsFile {
        version: FORMAT_VERSION,
        algorithm: ALGORITHM.to_string(),
        kdf: KDF.to_string(),
        iterations: DEFAULT_ITERATIONS,
        salt: encode(&salt),
        nonce: encode(&nonce),
        ciphertext: encode(&in_out),
    };
    serde_json::to_string_pretty(&file).context("serialize encrypted wallets")
}

pub fn decrypt_json(contents: &str, passphrase: &str) -> anyhow::Result<Vec<u8>> {
    ensure_passphrase(passphrase)?;
    let file: EncryptedWalletsFile =
        serde_json::from_str(contents).context("parse encrypted wallets file")?;
    if file.version != FORMAT_VERSION {
        bail!("unsupported encrypted wallets version {}", file.version);
    }
    if file.algorithm != ALGORITHM {
        bail!("unsupported encrypted wallets algorithm {}", file.algorithm);
    }
    if file.kdf != KDF {
        bail!("unsupported encrypted wallets kdf {}", file.kdf);
    }

    let salt = decode(&file.salt, "salt")?;
    let nonce = decode(&file.nonce, "nonce")?;
    let mut ciphertext = decode(&file.ciphertext, "ciphertext")?;
    let nonce: [u8; NONCE_LEN] = nonce
        .try_into()
        .map_err(|_| anyhow!("encrypted wallets nonce must be {NONCE_LEN} bytes"))?;

    let key = derive_key(passphrase, &salt, file.iterations)?;
    let less_safe_key = LessSafeKey::new(
        UnboundKey::new(&aead::AES_256_GCM, &key).map_err(|_| anyhow!("create AES key"))?,
    );
    let plaintext = less_safe_key
        .open_in_place(
            Nonce::assume_unique_for_key(nonce),
            Aad::from(AAD),
            &mut ciphertext,
        )
        .map_err(|_| anyhow!("decrypt wallets failed: wrong passphrase or corrupted file"))?;
    Ok(plaintext.to_vec())
}

fn ensure_passphrase(passphrase: &str) -> anyhow::Result<()> {
    if passphrase.is_empty() {
        bail!("wallets passphrase is empty");
    }
    Ok(())
}

fn derive_key(passphrase: &str, salt: &[u8], iterations: u32) -> anyhow::Result<[u8; KEY_LEN]> {
    let iterations = NonZeroU32::new(iterations).context("PBKDF2 iterations must be non-zero")?;
    let mut key = [0_u8; KEY_LEN];
    pbkdf2::derive(
        pbkdf2::PBKDF2_HMAC_SHA256,
        iterations,
        salt,
        passphrase.as_bytes(),
        &mut key,
    );
    Ok(key)
}

fn encode(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn decode(value: &str, name: &str) -> anyhow::Result<Vec<u8>> {
    base64::engine::general_purpose::STANDARD
        .decode(value)
        .with_context(|| format!("decode encrypted wallets {name}"))
}
