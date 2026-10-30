//! Small JSON documents in the app data directory, read with a size cap and written
//! atomically (temporary file, then rename) so a crash never leaves a half-written file.

use std::fs;
use std::io::Read;
use std::path::PathBuf;

use serde::Serialize;
use serde::de::DeserializeOwned;

fn path(file_name: &str) -> PathBuf {
    crate::platform::shell().data_dir().join(file_name)
}

/// Reads `file_name`. Returns `Ok(None)` when the file does not exist.
pub fn read<T: DeserializeOwned>(file_name: &str, max_bytes: u64) -> Result<Option<T>, String> {
    let path = path(file_name);
    let file = match fs::File::open(&path) {
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
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|error| error.to_string())
}

pub fn write<T: Serialize>(file_name: &str, value: &T) -> Result<(), String> {
    let path = path(file_name);
    if let Some(directory) = path.parent() {
        fs::create_dir_all(directory).map_err(|error| error.to_string())?;
    }
    let bytes = serde_json::to_vec(value).map_err(|error| error.to_string())?;
    let temp = path.with_extension(format!("json.{}.tmp", std::process::id()));
    fs::write(&temp, bytes).map_err(|error| error.to_string())?;
    fs::rename(&temp, &path).map_err(|error| {
        let _ = fs::remove_file(&temp);
        error.to_string()
    })
}
