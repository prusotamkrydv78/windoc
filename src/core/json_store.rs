//! Small documents in the app data directory, read with a size cap and written atomically
//! (temporary file, then rename) so a crash never leaves a half-written file.

use std::fs;
use std::io::Read;
use std::path::PathBuf;

use serde::Serialize;
use serde::de::DeserializeOwned;

fn path(file_name: &str) -> PathBuf {
    crate::platform::shell().data_dir().join(file_name)
}

/// Reads raw bytes. Returns `Ok(None)` when the file does not exist.
pub fn read_bytes(file_name: &str, max_bytes: u64) -> Result<Option<Vec<u8>>, String> {
    let file = match fs::File::open(path(file_name)) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    let mut bytes = Vec::new();
    file.take(max_bytes + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > max_bytes {
        return Err("file exceeds size limit".into());
    }
    Ok(Some(bytes))
}

pub fn write_bytes(file_name: &str, bytes: &[u8]) -> Result<(), String> {
    let path = path(file_name);
    if let Some(directory) = path.parent() {
        fs::create_dir_all(directory).map_err(|error| error.to_string())?;
    }
    let temp = path.with_extension(format!("{}.tmp", std::process::id()));
    fs::write(&temp, bytes).map_err(|error| error.to_string())?;
    fs::rename(&temp, &path).map_err(|error| {
        let _ = fs::remove_file(&temp);
        error.to_string()
    })
}

/// Reads a JSON document. Returns `Ok(None)` when the file does not exist.
pub fn read<T: DeserializeOwned>(file_name: &str, max_bytes: u64) -> Result<Option<T>, String> {
    match read_bytes(file_name, max_bytes)? {
        Some(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|error| error.to_string()),
        None => Ok(None),
    }
}

pub fn write<T: Serialize>(file_name: &str, value: &T) -> Result<(), String> {
    let bytes = serde_json::to_vec(value).map_err(|error| error.to_string())?;
    write_bytes(file_name, &bytes)
}

/// Deletes `file_name` if it exists.
pub fn remove(file_name: &str) -> Result<(), String> {
    match fs::remove_file(path(file_name)) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

/// Size of `file_name` in bytes, or `None` when it does not exist.
pub fn file_size(file_name: &str) -> Option<u64> {
    fs::metadata(path(file_name))
        .ok()
        .map(|metadata| metadata.len())
}
