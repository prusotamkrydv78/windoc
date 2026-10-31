//! Read-only ICS calendar feeds (workflow §9).
//!
//! Parsing is defensive: input size, line count and event count are capped. Recurring
//! events (RRULE) show their first occurrence only until bounded expansion is added.

use std::time::Duration;

pub const MAX_FEED_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_EVENTS: usize = 5_000;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct CalendarEvent {
    /// Sortable start `YYYYMMDDTHHMMSS` (or `YYYYMMDD` for all-day events).
    pub start: String,
    pub title: String,
    pub all_day: bool,
    /// Whether the start time is UTC (`Z` suffix) rather than floating local time.
    pub utc: bool,
    pub location: String,
}

impl CalendarEvent {
    /// `YYYY-MM-DD`.
    pub fn date(&self) -> String {
        format!(
            "{}-{}-{}",
            &self.start[0..4],
            &self.start[4..6],
            &self.start[6..8]
        )
    }

    /// `HH:MM`, or `None` for all-day events.
    pub fn time(&self) -> Option<String> {
        (!self.all_day && self.start.len() >= 13)
            .then(|| format!("{}:{}", &self.start[9..11], &self.start[11..13]))
    }
}

/// Undoes RFC 5545 line folding (continuation lines start with a space or tab).
fn unfold(text: &str) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for line in text.lines() {
        if let Some(continuation) = line.strip_prefix([' ', '\t'])
            && let Some(last) = lines.last_mut()
        {
            last.push_str(continuation);
        } else {
            lines.push(line.to_string());
        }
    }
    lines
}

fn unescape(value: &str) -> String {
    value
        .replace("\\n", " ")
        .replace("\\N", " ")
        .replace("\\,", ",")
        .replace("\\;", ";")
        .replace("\\\\", "\\")
}

fn valid_start(value: &str) -> bool {
    let digits = value
        .chars()
        .take(8)
        .all(|character| character.is_ascii_digit());
    digits && (value.len() == 8 || (value.len() >= 15 && value.as_bytes()[8] == b'T'))
}

/// Parses VEVENTs from an ICS document. Malformed events are skipped.
pub fn parse(text: &str) -> Vec<CalendarEvent> {
    let text = &text[..text.len().min(MAX_FEED_BYTES)];
    let mut events = Vec::new();
    let mut current: Option<CalendarEvent> = None;
    for line in unfold(text) {
        if line == "BEGIN:VEVENT" {
            current = Some(CalendarEvent {
                start: String::new(),
                title: String::new(),
                all_day: false,
                utc: false,
                location: String::new(),
            });
            continue;
        }
        if line == "END:VEVENT" {
            if let Some(event) = current.take()
                && valid_start(&event.start)
            {
                events.push(event);
                if events.len() >= MAX_EVENTS {
                    break;
                }
            }
            continue;
        }
        let Some(event) = current.as_mut() else {
            continue;
        };
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let (key, params) = name.split_once(';').unwrap_or((name, ""));
        match key {
            "DTSTART" => {
                event.utc = value.ends_with('Z');
                event.start = value.trim_end_matches('Z').to_string();
                event.all_day = params.contains("VALUE=DATE") || value.len() == 8;
            }
            "SUMMARY" => event.title = unescape(value).chars().take(200).collect(),
            "LOCATION" => event.location = unescape(value).chars().take(200).collect(),
            _ => {}
        }
    }
    events.sort();
    events
}

/// Events starting on or after `from` (`YYYYMMDD`) and before `until` (`YYYYMMDD`).
pub fn upcoming<'a>(
    events: &'a [CalendarEvent],
    from: &str,
    until: &str,
) -> Vec<&'a CalendarEvent> {
    events
        .iter()
        .filter(|event| {
            let day = &event.start[..8];
            day >= from && day < until
        })
        .collect()
}

/// Fetches one feed. `webcal://` links are fetched over HTTPS.
pub async fn fetch(url: &str) -> Result<Vec<CalendarEvent>, String> {
    let url = url
        .strip_prefix("webcal://")
        .map_or_else(|| url.to_string(), |rest| format!("https://{rest}"));
    if !url.starts_with("https://") && !url.starts_with("http://") {
        return Err("unsupported link".into());
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|error| error.to_string())?;
    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|error| error.to_string())?;
    if !response.status().is_success() {
        return Err(format!("HTTP {}", response.status().as_u16()));
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_FEED_BYTES as u64)
    {
        return Err("feed is too large".into());
    }
    let body = response.bytes().await.map_err(|error| error.to_string())?;
    if body.len() > MAX_FEED_BYTES {
        return Err("feed is too large".into());
    }
    Ok(parse(&String::from_utf8_lossy(&body)))
}
