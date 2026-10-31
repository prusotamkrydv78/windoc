//! Collects local data into an encrypted backup file and restores it.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use winisland_core::backup::{self, BackupFiles};
use winisland_platform_windows::dpapi;

use super::json_store;

const CONFIG_NAME: &str = "config.toml";
/// Clipboard history is stored DPAPI-encrypted for this account; backups carry it as
/// plain JSON inside the passphrase-encrypted file so it can move to another PC.
const CLIPBOARD_PORTABLE_NAME: &str = "clipboard.json";
const CLIPBOARD_ENTROPY: &[u8] = b"winisland-next/clipboard/v1";
const DATA_FILES: [&str; 5] = [
    super::focus_store::FILE_NAME,
    super::screen_time::FILE_NAME,
    super::weather_service::FILE_NAME,
    super::shelf_store::FILE_NAME,
    "notes.json",
];
const MAX_FILE_BYTES: u64 = 96 * 1024 * 1024;

fn collect() -> Result<BackupFiles, String> {
    let mut files = BackupFiles::new();
    let config =
        std::fs::read(super::persistence::get_config_path()).map_err(|error| error.to_string())?;
    files.insert(CONFIG_NAME.to_string(), config);
    for name in DATA_FILES {
        if let Some(bytes) = json_store::read_bytes(name, MAX_FILE_BYTES)? {
            files.insert(name.to_string(), bytes);
        }
    }
    if let Some(encrypted) =
        json_store::read_bytes(super::clipboard_history::FILE_NAME, MAX_FILE_BYTES)?
    {
        let plain = dpapi::unprotect(&encrypted, Some(CLIPBOARD_ENTROPY))
            .map_err(|error| error.to_string())?;
        files.insert(CLIPBOARD_PORTABLE_NAME.to_string(), plain);
    }
    Ok(files)
}

pub fn export(path: &Path, passphrase: &str) -> Result<(), String> {
    let created = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    let data =
        backup::encrypt(&collect()?, passphrase, created).map_err(|error| error.to_string())?;
    std::fs::write(path, data).map_err(|error| error.to_string())
}

/// Replaces local data with the backup's contents. The app must restart afterwards so
/// running modules reload it.
pub fn restore(path: &Path, passphrase: &str) -> Result<(), String> {
    let data = std::fs::read(path).map_err(|error| error.to_string())?;
    let (files, _created) =
        backup::decrypt(&data, passphrase).map_err(|error| error.to_string())?;
    for (name, bytes) in files {
        match name.as_str() {
            CONFIG_NAME => std::fs::write(super::persistence::get_config_path(), bytes)
                .map_err(|error| error.to_string())?,
            CLIPBOARD_PORTABLE_NAME => {
                let encrypted = dpapi::protect(&bytes, Some(CLIPBOARD_ENTROPY))
                    .map_err(|error| error.to_string())?;
                json_store::write_bytes(super::clipboard_history::FILE_NAME, &encrypted)?;
            }
            name if DATA_FILES.contains(&name) => json_store::write_bytes(name, &bytes)?,
            other => log::warn!("Skipping unknown file in backup: {other}"),
        }
    }
    Ok(())
}
