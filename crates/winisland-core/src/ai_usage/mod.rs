//! AI coding usage from local Claude Code and Codex logs (workflow §18).
//!
//! Only metadata is read: timestamps, model names, token counts and session ids. Prompt
//! and response text is never parsed into memory beyond the JSON line being inspected,
//! never stored and never sent anywhere. Source logs are opened read-only.

use std::collections::BTreeMap;

use serde_json::Value;

/// Which tool wrote a log line.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Tool {
    ClaudeCode,
    Codex,
}

impl Tool {
    pub const fn label(self) -> &'static str {
        match self {
            Self::ClaudeCode => "Claude Code",
            Self::Codex => "Codex",
        }
    }
}

/// Token usage from one response.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UsageEvent {
    pub tool: Tool,
    /// UTC date `YYYY-MM-DD` taken from the event timestamp.
    pub date: String,
    pub session: String,
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_tokens: u64,
}

impl UsageEvent {
    pub fn total_tokens(&self) -> u64 {
        self.input_tokens + self.output_tokens + self.cache_tokens
    }
}

fn number(value: &Value, key: &str) -> u64 {
    value.get(key).and_then(Value::as_u64).unwrap_or(0)
}

fn date_of(timestamp: &str) -> Option<String> {
    let date = timestamp.get(..10)?;
    (date.len() == 10 && date.as_bytes()[4] == b'-' && date.as_bytes()[7] == b'-')
        .then(|| date.to_string())
}

/// Parses one Claude Code JSONL line: assistant messages carry `message.model` and
/// `message.usage`. Any other line yields `None`.
pub fn parse_claude_line(line: &str) -> Option<UsageEvent> {
    let value: Value = serde_json::from_str(line).ok()?;
    if value.get("type")?.as_str()? != "assistant" {
        return None;
    }
    let message = value.get("message")?;
    let usage = message.get("usage")?;
    Some(UsageEvent {
        tool: Tool::ClaudeCode,
        date: date_of(value.get("timestamp")?.as_str()?)?,
        session: value
            .get("sessionId")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        model: message
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string(),
        input_tokens: number(usage, "input_tokens"),
        output_tokens: number(usage, "output_tokens"),
        cache_tokens: number(usage, "cache_read_input_tokens")
            + number(usage, "cache_creation_input_tokens"),
    })
}

/// Parses one Codex rollout JSONL line: `token_count` events carry the last turn's usage.
pub fn parse_codex_line(line: &str, session: &str, model: &str) -> Option<UsageEvent> {
    let value: Value = serde_json::from_str(line).ok()?;
    let payload = value.get("payload")?;
    if payload.get("type")?.as_str()? != "token_count" {
        return None;
    }
    let usage = payload.get("info")?.get("last_token_usage")?;
    Some(UsageEvent {
        tool: Tool::Codex,
        date: date_of(value.get("timestamp")?.as_str()?)?,
        session: session.to_string(),
        model: model.to_string(),
        input_tokens: number(usage, "input_tokens"),
        output_tokens: number(usage, "output_tokens") + number(usage, "reasoning_output_tokens"),
        cache_tokens: number(usage, "cached_input_tokens"),
    })
}

/// Reads the model from a Codex `turn_context` line, if this is one.
pub fn codex_model(line: &str) -> Option<String> {
    let value: Value = serde_json::from_str(line).ok()?;
    if value.get("type")?.as_str()? != "turn_context" {
        return None;
    }
    Some(value.get("payload")?.get("model")?.as_str()?.to_string())
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DayReport {
    pub tokens: u64,
    pub responses: u64,
    pub sessions: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UsageReport {
    /// Per UTC date, oldest first.
    pub days: BTreeMap<String, DayReport>,
    /// Total tokens per model across all days, largest first.
    pub models: Vec<(String, u64)>,
    /// Total tokens per tool, Claude Code first.
    pub tools: Vec<(Tool, u64)>,
}

impl UsageReport {
    pub fn from_events<'a>(events: impl IntoIterator<Item = &'a UsageEvent>) -> Self {
        let mut days: BTreeMap<String, DayReport> = BTreeMap::new();
        let mut sessions: BTreeMap<String, std::collections::BTreeSet<String>> = BTreeMap::new();
        let mut models: BTreeMap<String, u64> = BTreeMap::new();
        let mut tools: BTreeMap<Tool, u64> = BTreeMap::new();
        for event in events {
            let tokens = event.total_tokens();
            let day = days.entry(event.date.clone()).or_default();
            day.tokens += tokens;
            day.responses += 1;
            sessions
                .entry(event.date.clone())
                .or_default()
                .insert(event.session.clone());
            *models.entry(event.model.clone()).or_default() += tokens;
            *tools.entry(event.tool).or_default() += tokens;
        }
        for (date, ids) in sessions {
            if let Some(day) = days.get_mut(&date) {
                day.sessions = ids.len();
            }
        }
        let mut models: Vec<(String, u64)> = models.into_iter().collect();
        models.sort_by(|a, b| b.1.cmp(&a.1));
        // Claude Code is the primary tool, so it always comes first.
        let tools: Vec<(Tool, u64)> = tools.into_iter().collect();
        Self {
            days,
            models,
            tools,
        }
    }
}

/// Compact token count such as `1.2M` or `850K`.
pub fn format_tokens(tokens: u64) -> String {
    if tokens >= 1_000_000 {
        format!("{:.1}M", tokens as f64 / 1_000_000.0)
    } else if tokens >= 1_000 {
        format!("{:.0}K", tokens as f64 / 1_000.0)
    } else {
        tokens.to_string()
    }
}
