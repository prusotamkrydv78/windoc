//! "What did I miss?": a short recap of the phone alerts in Notchy Memory.
//!
//! Always works with a built-in summary (alerts grouped by app and sender). With an AI key
//! set in the Memory tab (OpenRouter, OpenAI, or any OpenAI-compatible endpoint, including a
//! local Ollama at `http://localhost:11434/v1`), the robot asks the model for a friendlier
//! two or three sentences. Only the alerts being summarised are sent.

use std::collections::BTreeMap;

use parking_lot::Mutex;

use crate::core::memory::{self, Entry, Kind};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Reason {
    /// The user came back to the PC.
    Back,
    /// The phone asked.
    Phone,
    /// The Memory tab's button.
    Manual,
}

#[derive(Clone, Debug)]
pub struct Recap {
    pub text: String,
    /// The local model that wrote it, or `None` for the built-in summary.
    pub model: Option<String>,
    pub alerts: usize,
    pub at: u64,
}

struct State {
    latest: Option<Recap>,
    finished: Vec<(Recap, Reason)>,
    busy: bool,
}

static STATE: Mutex<State> = Mutex::new(State {
    latest: None,
    finished: Vec::new(),
    busy: false,
});

pub fn latest() -> Option<Recap> {
    STATE.lock().latest.clone()
}

pub fn busy() -> bool {
    STATE.lock().busy
}

/// The model the recap will use, when an AI key is set.
pub fn model() -> Option<String> {
    let settings = crate::core::brain::settings();
    (!settings.ai_key.is_empty()).then_some(settings.ai_model)
}

/// Recaps that finished since the last call, with why they were asked for.
pub fn take_finished() -> Vec<(Recap, Reason)> {
    std::mem::take(&mut STATE.lock().finished)
}

/// Starts a recap of the alerts since `since_ms`. Does nothing while one is running.
pub fn request(since_ms: u64, reason: Reason) {
    {
        let mut state = STATE.lock();
        if state.busy {
            return;
        }
        state.busy = true;
    }
    let model = model();
    let alerts: Vec<Entry> = memory::since(since_ms)
        .into_iter()
        .filter(|entry| entry.kind == Kind::PhoneAlert)
        .collect();
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        STATE.lock().busy = false;
        return;
    };
    handle.spawn(async move {
        let basic = summarise(&alerts);
        let ai = match (&model, alerts.len()) {
            (Some(model), 2..) => ask_model(model, &alerts).await,
            _ => None,
        };
        let recap = Recap {
            model: ai.as_ref().and(model),
            text: ai.unwrap_or(basic),
            alerts: alerts.len(),
            at: memory::now_ms(),
        };
        if recap.alerts > 0 {
            memory::record(Kind::Recap, recap.text.clone(), "", "");
        }
        let mut state = STATE.lock();
        state.latest = Some(recap.clone());
        state.finished.push((recap, reason));
        state.busy = false;
        drop(state);
        crate::platform::wake();
    });
}

/// The built-in summary: apps by count, with the people who wrote most.
pub fn summarise(alerts: &[Entry]) -> String {
    if alerts.is_empty() {
        return "Nothing new from your phone.".to_string();
    }
    let mut apps: BTreeMap<&str, (usize, BTreeMap<&str, usize>)> = BTreeMap::new();
    for alert in alerts {
        let app = if alert.app.is_empty() { "Phone" } else { alert.app.as_str() };
        let slot = apps.entry(app).or_default();
        slot.0 += 1;
        if !alert.title.is_empty() {
            *slot.1.entry(alert.title.as_str()).or_default() += 1;
        }
    }
    let mut apps: Vec<_> = apps.into_iter().collect();
    apps.sort_by(|a, b| b.1.0.cmp(&a.1.0));
    let parts: Vec<String> = apps
        .iter()
        .take(4)
        .map(|(app, (count, senders))| {
            let mut senders: Vec<_> = senders.iter().collect();
            senders.sort_by(|a, b| b.1.cmp(a.1));
            let names: Vec<String> = senders
                .iter()
                .take(3)
                .map(|(name, count)| {
                    if **count > 1 {
                        format!("{name} ×{count}")
                    } else {
                        (*name).to_string()
                    }
                })
                .collect();
            if names.is_empty() {
                format!("{app}: {count}")
            } else {
                format!("{app}: {count} ({})", names.join(", "))
            }
        })
        .collect();
    let more = apps.len().saturating_sub(4);
    let mut text = format!(
        "{} alert{} — {}",
        alerts.len(),
        if alerts.len() == 1 { "" } else { "s" },
        parts.join(" · ")
    );
    if more > 0 {
        text.push_str(&format!(" · +{more} more apps"));
    }
    text
}

async fn ask_model(model: &str, alerts: &[Entry]) -> Option<String> {
    let mut lines = String::new();
    for alert in alerts.iter().rev().take(60).rev() {
        let line = format!("- {} · {}: {}\n", alert.app, alert.title, alert.detail.replace('\n', " "));
        if lines.len() + line.len() > 5000 {
            break;
        }
        lines.push_str(&line);
    }
    let prompt = format!(
        "You are Notchy, a friendly little desk robot. The user was away from their laptop. \
         Summarise the phone notifications below in at most three short sentences. Say who \
         wants something from them and anything that looks urgent. No lists, no greetings.\n\n{lines}"
    );
    let text = crate::core::brain::chat(model, &prompt, 200).await.ok()?;
    let text = text.trim();
    // Some small models think out loud first; keep only the answer.
    let text = text.rsplit("</think>").next().unwrap_or(text).trim();
    (!text.is_empty()).then(|| text.chars().take(600).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alert(app: &str, title: &str) -> Entry {
        Entry {
            at: 0,
            kind: Kind::PhoneAlert,
            title: title.into(),
            detail: String::new(),
            app: app.into(),
        }
    }

    #[test]
    fn groups_by_app_and_sender() {
        let alerts = [
            alert("WhatsApp", "Mom"),
            alert("WhatsApp", "Mom"),
            alert("WhatsApp", "Ravi"),
            alert("Gmail", "Bank"),
        ];
        assert_eq!(
            summarise(&alerts),
            "4 alerts — WhatsApp: 3 (Mom ×2, Ravi) · Gmail: 1 (Bank)"
        );
        assert_eq!(summarise(&[]), "Nothing new from your phone.");
    }
}
