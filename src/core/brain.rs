//! Settings for Notchy's "brain": the AI endpoint and the SQL Server database, edited in the
//! Memory tab and saved in `brain.json`. The API key and the connection string (which can hold
//! a password) are encrypted with Windows DPAPI for this user before they touch the disk.
//!
//! The AI side speaks the OpenAI chat-completions API, so OpenRouter, OpenAI, Groq, Together
//! or a local Ollama (`http://localhost:11434/v1`) all work by changing the base URL.

use std::time::Duration;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const FILE: &str = "brain.json";
pub const OPENROUTER: &str = "https://openrouter.ai/api/v1";
pub const OPENAI: &str = "https://api.openai.com/v1";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    pub ai_base_url: String,
    pub ai_key: String,
    pub ai_model: String,
    /// ADO.NET style, e.g. `Server=tcp:localhost,1433;Database=Notchy;User Id=sa;Password=…;TrustServerCertificate=true`.
    pub sql_connection: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            ai_base_url: OPENROUTER.to_string(),
            ai_key: String::new(),
            ai_model: "openai/gpt-4o-mini".to_string(),
            sql_connection: String::new(),
        }
    }
}

/// On disk: secrets as base64 DPAPI blobs.
#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
struct Stored {
    ai_base_url: String,
    ai_key: String,
    ai_model: String,
    sql_connection: String,
}

static SETTINGS: Mutex<Option<Settings>> = Mutex::new(None);

pub fn settings() -> Settings {
    let mut cached = SETTINGS.lock();
    if let Some(settings) = cached.as_ref() {
        return settings.clone();
    }
    let settings = match crate::core::json_store::read::<Stored>(FILE, 64 * 1024) {
        Ok(Some(stored)) => {
            let defaults = Settings::default();
            Settings {
                ai_base_url: if stored.ai_base_url.is_empty() { defaults.ai_base_url } else { stored.ai_base_url },
                ai_key: unprotect(&stored.ai_key).unwrap_or_default(),
                ai_model: if stored.ai_model.is_empty() { defaults.ai_model } else { stored.ai_model },
                sql_connection: unprotect(&stored.sql_connection).unwrap_or_default(),
            }
        }
        Ok(None) => Settings::default(),
        Err(error) => {
            log::warn!("Cannot read {FILE}: {error}");
            Settings::default()
        }
    };
    *cached = Some(settings.clone());
    settings
}

pub fn save(settings: Settings) {
    let stored = Stored {
        ai_base_url: settings.ai_base_url.trim().trim_end_matches('/').to_string(),
        ai_key: protect(settings.ai_key.trim()),
        ai_model: settings.ai_model.trim().to_string(),
        sql_connection: protect(settings.sql_connection.trim()),
    };
    if let Err(error) = crate::core::json_store::write(FILE, &stored) {
        log::warn!("Cannot save {FILE}: {error}");
    }
    let changed_sql = SETTINGS
        .lock()
        .as_ref()
        .is_none_or(|old| old.sql_connection != settings.sql_connection);
    *SETTINGS.lock() = Some(Settings {
        ai_base_url: stored.ai_base_url,
        ai_key: settings.ai_key.trim().to_string(),
        ai_model: stored.ai_model,
        sql_connection: settings.sql_connection.trim().to_string(),
    });
    if changed_sql {
        crate::core::memory_sql::reconnect();
    }
}

fn protect(plain: &str) -> String {
    use base64::Engine;
    use windows::Win32::Foundation::{HLOCAL, LocalFree};
    use windows::Win32::Security::Cryptography::{CRYPT_INTEGER_BLOB, CryptProtectData};
    if plain.is_empty() {
        return String::new();
    }
    let input = CRYPT_INTEGER_BLOB {
        cbData: plain.len() as u32,
        pbData: plain.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB::default();
    // SAFETY: `input` points at `plain`; `output` is freed below.
    if unsafe { CryptProtectData(&input, None, None, None, None, 0, &mut output) }.is_err() {
        return String::new();
    }
    // SAFETY: DPAPI returned `cbData` bytes.
    let bytes = unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize) }.to_vec();
    // SAFETY: allocated by CryptProtectData.
    unsafe {
        let _ = LocalFree(Some(HLOCAL(output.pbData.cast())));
    }
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn unprotect(encoded: &str) -> Option<String> {
    use base64::Engine;
    use windows::Win32::Foundation::{HLOCAL, LocalFree};
    use windows::Win32::Security::Cryptography::{CRYPT_INTEGER_BLOB, CryptUnprotectData};
    if encoded.is_empty() {
        return None;
    }
    let blob = base64::engine::general_purpose::STANDARD.decode(encoded).ok()?;
    let input = CRYPT_INTEGER_BLOB {
        cbData: blob.len() as u32,
        pbData: blob.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB::default();
    // SAFETY: as above.
    unsafe { CryptUnprotectData(&input, None, None, None, None, 0, &mut output) }.ok()?;
    // SAFETY: DPAPI returned `cbData` bytes.
    let bytes = unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize) }.to_vec();
    // SAFETY: allocated by CryptUnprotectData.
    unsafe {
        let _ = LocalFree(Some(HLOCAL(output.pbData.cast())));
    }
    String::from_utf8(bytes).ok()
}

/// One chat completion. Errors are short, user-facing sentences.
pub async fn chat(model: &str, prompt: &str, max_tokens: u32) -> Result<String, String> {
    let settings = settings();
    if settings.ai_key.is_empty() {
        return Err("No AI key set".to_string());
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(45))
        .build()
        .map_err(|error| error.to_string())?;
    let response = client
        .post(format!("{}/chat/completions", settings.ai_base_url.trim_end_matches('/')))
        .bearer_auth(&settings.ai_key)
        // OpenRouter shows these on its dashboard; other APIs ignore them.
        .header("HTTP-Referer", "https://notchy.local")
        .header("X-Title", "Notchy")
        .json(&json!({
            "model": model,
            "messages": [
                {"role": "system", "content": "You are Notchy, a friendly little desk robot living in a laptop's notch. Be brief and warm."},
                {"role": "user", "content": prompt},
            ],
            "max_tokens": max_tokens,
            "temperature": 0.3,
        }))
        .send()
        .await
        .map_err(|_| "Cannot reach the AI service".to_string())?;
    let status = response.status();
    let body: Value = response.json().await.map_err(|_| "The AI sent a reply Notchy cannot read".to_string())?;
    if !status.is_success() {
        let message = body["error"]["message"].as_str().unwrap_or("request refused");
        return Err(format!("AI error {}: {message}", status.as_u16()));
    }
    body["choices"][0]["message"]["content"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| "The AI sent an empty reply".to_string())
}

/// A tiny request to check the key and model work.
pub async fn test_ai() -> Result<String, String> {
    let model = settings().ai_model;
    chat(&model, "Say hi in five words.", 20).await
}
