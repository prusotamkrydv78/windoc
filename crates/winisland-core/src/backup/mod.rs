//! Portable, passphrase-encrypted backups (workflow §22, Phase 3).
//!
//! Format: `WIBK` magic, version byte, 16-byte Argon2id salt, 12-byte nonce, then the
//! ChaCha20-Poly1305 ciphertext of a JSON document mapping file names to their bytes.
//! Without the passphrase the file reveals nothing but its size.

use std::collections::BTreeMap;

use argon2::Argon2;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use serde::{Deserialize, Serialize};

const MAGIC: &[u8; 4] = b"WIBK";
const VERSION: u8 = 1;
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 12;
const HEADER_LEN: usize = MAGIC.len() + 1 + SALT_LEN + NONCE_LEN;
pub const MIN_PASSPHRASE_CHARS: usize = 8;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BackupError {
    WeakPassphrase,
    NotABackup,
    UnsupportedVersion(u8),
    /// Wrong passphrase or a damaged file; AEAD cannot tell them apart.
    Decrypt,
    Format(String),
    Random,
}

impl std::fmt::Display for BackupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WeakPassphrase => write!(
                f,
                "the passphrase needs at least {MIN_PASSPHRASE_CHARS} characters"
            ),
            Self::NotABackup => write!(f, "this file is not a backup"),
            Self::UnsupportedVersion(version) => write!(f, "unsupported backup version {version}"),
            Self::Decrypt => write!(f, "wrong passphrase or damaged backup"),
            Self::Format(error) => write!(f, "backup contents are invalid: {error}"),
            Self::Random => write!(f, "secure random numbers are unavailable"),
        }
    }
}

impl std::error::Error for BackupError {}

#[derive(Serialize, Deserialize)]
struct Payload {
    created_unix: u64,
    files: BTreeMap<String, String>,
}

/// Backed-up files by name.
pub type BackupFiles = BTreeMap<String, Vec<u8>>;

fn derive_key(passphrase: &str, salt: &[u8]) -> Result<Key, BackupError> {
    let mut key = [0u8; 32];
    Argon2::default()
        .hash_password_into(passphrase.as_bytes(), salt, &mut key)
        .map_err(|error| BackupError::Format(error.to_string()))?;
    Ok(Key::from(key))
}

fn random<const N: usize>() -> Result<[u8; N], BackupError> {
    let mut bytes = [0u8; N];
    getrandom::fill(&mut bytes).map_err(|_| BackupError::Random)?;
    Ok(bytes)
}

pub fn encrypt(
    files: &BackupFiles,
    passphrase: &str,
    created_unix: u64,
) -> Result<Vec<u8>, BackupError> {
    if passphrase.chars().count() < MIN_PASSPHRASE_CHARS {
        return Err(BackupError::WeakPassphrase);
    }
    let payload = Payload {
        created_unix,
        files: files
            .iter()
            .map(|(name, bytes)| (name.clone(), STANDARD.encode(bytes)))
            .collect(),
    };
    let plain =
        serde_json::to_vec(&payload).map_err(|error| BackupError::Format(error.to_string()))?;
    let salt: [u8; SALT_LEN] = random()?;
    let nonce: [u8; NONCE_LEN] = random()?;
    let cipher = ChaCha20Poly1305::new(&derive_key(passphrase, &salt)?);
    let ciphertext = cipher
        .encrypt(Nonce::from_slice(&nonce), plain.as_slice())
        .map_err(|_| BackupError::Decrypt)?;
    let mut output = Vec::with_capacity(HEADER_LEN + ciphertext.len());
    output.extend_from_slice(MAGIC);
    output.push(VERSION);
    output.extend_from_slice(&salt);
    output.extend_from_slice(&nonce);
    output.extend_from_slice(&ciphertext);
    Ok(output)
}

pub fn decrypt(data: &[u8], passphrase: &str) -> Result<(BackupFiles, u64), BackupError> {
    if data.len() < HEADER_LEN || &data[..MAGIC.len()] != MAGIC {
        return Err(BackupError::NotABackup);
    }
    let version = data[MAGIC.len()];
    if version != VERSION {
        return Err(BackupError::UnsupportedVersion(version));
    }
    let salt = &data[MAGIC.len() + 1..MAGIC.len() + 1 + SALT_LEN];
    let nonce = &data[MAGIC.len() + 1 + SALT_LEN..HEADER_LEN];
    let cipher = ChaCha20Poly1305::new(&derive_key(passphrase, salt)?);
    let plain = cipher
        .decrypt(Nonce::from_slice(nonce), &data[HEADER_LEN..])
        .map_err(|_| BackupError::Decrypt)?;
    let payload: Payload =
        serde_json::from_slice(&plain).map_err(|error| BackupError::Format(error.to_string()))?;
    let files = payload
        .files
        .into_iter()
        .map(|(name, encoded)| {
            STANDARD
                .decode(encoded)
                .map(|bytes| (name, bytes))
                .map_err(|error| BackupError::Format(error.to_string()))
        })
        .collect::<Result<BackupFiles, _>>()?;
    Ok((files, payload.created_unix))
}
