//! Command palette parsing (workflow §7): local action syntax and module search.
//!
//! Everything here is local. A query is never sent anywhere; online search providers are
//! a separate, explicit action.

use std::time::Duration;

use crate::units;

/// Folders `open` understands without a full path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KnownFolder {
    Downloads,
    Documents,
    Desktop,
    Pictures,
    Music,
    Videos,
}

impl KnownFolder {
    fn parse(name: &str) -> Option<Self> {
        match name {
            "downloads" | "download" => Some(Self::Downloads),
            "documents" | "docs" => Some(Self::Documents),
            "desktop" => Some(Self::Desktop),
            "pictures" | "photos" => Some(Self::Pictures),
            "music" => Some(Self::Music),
            "videos" => Some(Self::Videos),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    /// `timer 25`, `timer 1h30m`, `timer 90s`
    StartTimer(Duration),
    /// `pomodoro`
    StartPomodoro,
    /// `stopwatch`
    StartStopwatch,
    /// `10 km to mi`
    Convert {
        value: f64,
        from: &'static str,
        to: &'static str,
        result: f64,
    },
    /// `10 usd to inr`; converted with cached rates once available.
    ConvertCurrency {
        value: f64,
        from: String,
        to: String,
    },
    /// `convert png`: convert images on the active shelf.
    ConvertImages(&'static str),
    /// `compress 200kb`: re-encode images on the active shelf as JPEG within a size.
    CompressImages(u64),
    /// `note buy milk`
    QuickNote(String),
    /// `open downloads`
    OpenFolder(KnownFolder),
    /// `clip invoice`: search clipboard history.
    SearchClipboard(String),
    /// A module name such as `weather` or `shelf`.
    OpenModule(&'static str),
    /// `g rust lifetimes`: search a provider. Carries the full URL to open.
    WebSearch { provider: &'static str, url: String },
}

const MODULES: &[(&str, &[&str])] = &[
    ("home", &["home", "dashboard"]),
    ("media", &["media", "music"]),
    ("clipboard", &["clipboard", "clips"]),
    ("shelf", &["shelf", "files"]),
    ("focus", &["focus", "timer"]),
    ("notes", &["notes"]),
    ("clock", &["clock", "world clock", "time zones"]),
    ("weather", &["weather", "forecast"]),
    ("settings", &["settings", "preferences"]),
];

/// Parses `25`, `25m`, `90s`, `1h`, `1h30m`, `1:30` (h:mm). Bare numbers are minutes.
pub fn parse_duration(text: &str) -> Option<Duration> {
    let text = text.trim().to_lowercase();
    if text.is_empty() {
        return None;
    }
    if let Some((hours, minutes)) = text.split_once(':') {
        let hours: u64 = hours.parse().ok()?;
        let minutes: u64 = minutes.parse().ok()?;
        return (minutes < 60).then(|| Duration::from_secs(hours * 3600 + minutes * 60));
    }
    if let Ok(minutes) = text.parse::<u64>() {
        return (minutes > 0).then(|| Duration::from_secs(minutes * 60));
    }
    let mut total = 0u64;
    let mut number = String::new();
    for character in text.chars() {
        if character.is_ascii_digit() {
            number.push(character);
            continue;
        }
        let value: u64 = number.parse().ok()?;
        number.clear();
        total += match character {
            'h' => value * 3600,
            'm' => value * 60,
            's' => value,
            _ => return None,
        };
    }
    if !number.is_empty() {
        return None;
    }
    (total > 0 && total <= 24 * 3600).then(|| Duration::from_secs(total))
}

/// Interprets a palette query. Returns `None` when nothing matches.
pub fn parse(query: &str) -> Option<Command> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return None;
    }
    let lower = trimmed.to_lowercase();
    let (word, rest) = match lower.split_once(char::is_whitespace) {
        Some((word, rest)) => (word, rest.trim()),
        None => (lower.as_str(), ""),
    };
    // Keep the original casing of free text.
    let original_rest = trimmed
        .split_once(char::is_whitespace)
        .map_or("", |(_, rest)| rest.trim());

    match word {
        "timer" if !rest.is_empty() => {
            return parse_duration(rest).map(Command::StartTimer);
        }
        "pomodoro" if rest.is_empty() => return Some(Command::StartPomodoro),
        "stopwatch" if rest.is_empty() => return Some(Command::StartStopwatch),
        "note" if !original_rest.is_empty() => {
            return Some(Command::QuickNote(original_rest.to_string()));
        }
        "open" => {
            if let Some(folder) = KnownFolder::parse(rest) {
                return Some(Command::OpenFolder(folder));
            }
        }
        "convert" => {
            if let Some(format) = ["png", "jpg", "ico", "bmp"]
                .into_iter()
                .find(|format| *format == rest || (rest == "jpeg" && *format == "jpg"))
            {
                return Some(Command::ConvertImages(format));
            }
        }
        "compress" => {
            if let Some(bytes) = parse_size(rest) {
                return Some(Command::CompressImages(bytes));
            }
        }
        "g" | "yt" | "gh" | "so" if !original_rest.is_empty() => {
            return search_url(word, original_rest);
        }
        "clip" | "clips" if !original_rest.is_empty() => {
            return Some(Command::SearchClipboard(original_rest.to_string()));
        }
        _ => {}
    }
    if let Some(query) = units::parse(trimmed) {
        if let Ok(result) = units::convert(query.value, query.from, query.to) {
            return Some(Command::Convert {
                value: query.value,
                from: query.from.symbol,
                to: query.to.symbol,
                result,
            });
        }
    }
    if let Some((value, from, to)) = crate::currency::parse_query(trimmed) {
        return Some(Command::ConvertCurrency { value, from, to });
    }
    let target = if word == "open" { rest } else { lower.as_str() };
    MODULES
        .iter()
        .find(|(_, names)| names.iter().any(|name| *name == target))
        .map(|(module, _)| Command::OpenModule(module))
}

/// Module names starting with `prefix`, for suggestions while typing.
pub fn suggest_modules(prefix: &str) -> Vec<&'static str> {
    let prefix = prefix.trim().to_lowercase();
    if prefix.is_empty() {
        return Vec::new();
    }
    MODULES
        .iter()
        .filter(|(_, names)| names.iter().any(|name| name.starts_with(&prefix)))
        .map(|(module, _)| *module)
        .collect()
}

/// Search providers: palette prefix, display name and URL template (`{}` is the query).
pub const SEARCH_PROVIDERS: [(&str, &str, &str); 4] = [
    ("g", "Google", "https://www.google.com/search?q={}"),
    (
        "yt",
        "YouTube",
        "https://www.youtube.com/results?search_query={}",
    ),
    ("gh", "GitHub", "https://github.com/search?q={}"),
    (
        "so",
        "Stack Overflow",
        "https://stackoverflow.com/search?q={}",
    ),
];

fn search_url(prefix: &str, query: &str) -> Option<Command> {
    let (_, name, template) = SEARCH_PROVIDERS
        .iter()
        .find(|(candidate, _, _)| *candidate == prefix)?;
    let encoded: String = query
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (byte as char).to_string()
            }
            b' ' => "+".to_string(),
            _ => format!("%{byte:02X}"),
        })
        .collect();
    Some(Command::WebSearch {
        provider: name,
        url: template.replace("{}", &encoded),
    })
}

/// Parses `200kb`, `1.5 mb` or `500k` into bytes.
fn parse_size(text: &str) -> Option<u64> {
    let text = text.replace(' ', "");
    let split = text
        .find(|character: char| character.is_ascii_alphabetic())
        .unwrap_or(text.len());
    let value: f64 = text[..split].parse().ok()?;
    let factor = match &text[split..] {
        "b" => 1.0,
        "k" | "kb" => 1024.0,
        "m" | "mb" => 1024.0 * 1024.0,
        _ => return None,
    };
    let bytes = value * factor;
    (bytes >= 1024.0).then_some(bytes as u64)
}
