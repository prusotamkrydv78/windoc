//! The robot's AI jobs, all through the key in the Memory tab (`core::brain`):
//! - Ask: a question answered from Notchy Memory (the phone can ask too).
//! - Diary: once a day after 9 pm, a short "your day" written into Memory.
//! - Clipboard magic: translate, summarise or fix the copied text, pasted back.
//! - Smart replies: three short replies for a phone message.
//!
//! Without a key: Ask searches Memory and lists matches, the diary is a plain tally, and
//! magic and replies say a key is needed. Results reach the UI thread through [`take`].

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::core::brain;
use crate::core::memory::{self, Entry, Kind};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Magic {
    Translate,
    Summarise,
    Fix,
}

impl Magic {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Translate => "Translate",
            Self::Summarise => "Summarise",
            Self::Fix => "Fix grammar",
        }
    }
}

/// Who asked.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Asker {
    Tab,
    Phone,
}

#[derive(Clone, Debug)]
pub enum Done {
    Answer { question: String, answer: String, from: Asker },
    /// Text for the clipboard, and what was done.
    Magic { text: String, what: &'static str },
    Replies { key: String, replies: Vec<String> },
    Diary(String),
    Failed(String),
}

static DONE: Mutex<Vec<Done>> = Mutex::new(Vec::new());
static BUSY: Mutex<bool> = Mutex::new(false);
static LAST_ANSWER: Mutex<Option<(String, String)>> = Mutex::new(None);

pub fn take() -> Vec<Done> {
    std::mem::take(&mut DONE.lock())
}

pub fn busy() -> bool {
    *BUSY.lock()
}

/// The last question and answer, for the Memory tab.
pub fn last_answer() -> Option<(String, String)> {
    LAST_ANSWER.lock().clone()
}

fn finish(done: Done) {
    if let Done::Answer { question, answer, .. } = &done {
        *LAST_ANSWER.lock() = Some((question.clone(), answer.clone()));
    }
    if let Done::Replies { key, replies } = &done {
        *REPLIES.lock() = Some((key.clone(), replies.clone()));
    }
    DONE.lock().push(done);
    *BUSY.lock() = false;
    crate::platform::wake();
}

fn spawn(job: impl std::future::Future<Output = Done> + Send + 'static) {
    {
        let mut busy = BUSY.lock();
        if *busy {
            return;
        }
        *busy = true;
    }
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => {
            handle.spawn(async move { finish(job.await) });
        }
        Err(_) => *BUSY.lock() = false,
    }
}

fn has_key() -> bool {
    !brain::settings().ai_key.is_empty()
}

fn when(entry: &Entry) -> String {
    let minutes = memory::now_ms().saturating_sub(entry.at) / 60_000;
    match minutes {
        0..60 => format!("{minutes} min ago"),
        60..1440 => format!("{} h ago", minutes / 60),
        _ => format!("{} days ago", minutes / 1440),
    }
}

fn lines(entries: &[Entry], budget: usize) -> String {
    let mut out = String::new();
    for entry in entries {
        let line = format!(
            "- [{}] {}{}: {} {}\n",
            when(entry),
            entry.kind.label(),
            if entry.app.is_empty() { String::new() } else { format!(" · {}", entry.app) },
            entry.title,
            entry.detail.replace('\n', " ")
        );
        if out.len() + line.len() > budget {
            break;
        }
        out.push_str(&line);
    }
    out
}

/// Answers `question` from Memory.
pub fn ask(question: String, from: Asker) {
    spawn(async move {
        // Matches for each word, plus the newest entries for "what happened" questions.
        let mut found: Vec<Entry> = Vec::new();
        for word in question.split_whitespace().filter(|word| word.len() > 3) {
            for entry in memory::search(word, 15) {
                if !found.iter().any(|known| known.at == entry.at && known.title == entry.title) {
                    found.push(entry);
                }
            }
        }
        for entry in memory::search("", 25) {
            if !found.iter().any(|known| known.at == entry.at && known.title == entry.title) {
                found.push(entry);
            }
        }
        found.sort_by(|a, b| b.at.cmp(&a.at));
        if !has_key() {
            let matches: Vec<Entry> = question
                .split_whitespace()
                .filter(|word| word.len() > 3)
                .flat_map(|word| memory::search(word, 5))
                .take(5)
                .collect();
            let answer = if matches.is_empty() {
                "I couldn't find that in Memory. Add an AI key in the Memory tab and I can answer properly.".to_string()
            } else {
                matches
                    .iter()
                    .map(|entry| format!("{} ({}): {}", entry.title, when(entry), entry.detail))
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            return Done::Answer { question, answer, from };
        }
        let prompt = format!(
            "Answer the user's question using only this log of what happened between their \
             phone and PC. If the log does not say, say you don't know. One or two sentences.\n\n\
             Log (newest first):\n{}\nQuestion: {question}",
            lines(&found, 6000)
        );
        match brain::chat(&brain::settings().ai_model, &prompt, 220).await {
            Ok(answer) => Done::Answer { question, answer: answer.trim().to_string(), from },
            Err(error) => Done::Failed(error),
        }
    });
}

/// Translate, summarise or fix `text`; the result goes on the clipboard.
pub fn magic(what: Magic, text: String) {
    if text.trim().is_empty() {
        DONE.lock().push(Done::Failed("Copy some text first".to_string()));
        return;
    }
    if !has_key() {
        DONE.lock().push(Done::Failed("Clipboard magic needs an AI key (Memory → Settings)".to_string()));
        return;
    }
    spawn(async move {
        let instruction = match what {
            Magic::Translate => "Translate this into English (or, if it is already English, into Hindi). Reply with the translation only.",
            Magic::Summarise => "Summarise this in two or three short sentences. Reply with the summary only.",
            Magic::Fix => "Fix spelling and grammar, keeping the meaning and tone. Reply with the corrected text only.",
        };
        let text: String = text.chars().take(6000).collect();
        match brain::chat(&brain::settings().ai_model, &format!("{instruction}\n\n{text}"), 800).await {
            Ok(result) => Done::Magic { text: result.trim().to_string(), what: what.label() },
            Err(error) => Done::Failed(error),
        }
    });
}

/// Three short replies to a phone message.
pub fn suggest_replies(key: String, app: String, title: String, text: String) {
    if !has_key() {
        let replies = vec!["Okay 👍".to_string(), "On it".to_string(), "Call you soon".to_string()];
        *REPLIES.lock() = Some((key, replies));
        crate::platform::wake();
        return;
    }
    spawn(async move {
        let prompt = format!(
            "Write three different short replies (under 12 words each) the user could send to \
             this {app} message from {title}: \"{text}\". Match the language of the message. \
             One reply per line, no numbering, no quotes."
        );
        match brain::chat(&brain::settings().ai_model, &prompt, 120).await {
            Ok(reply) => Done::Replies {
                key,
                replies: reply
                    .lines()
                    .map(|line| line.trim().trim_start_matches(['-', '•', '*', '1', '2', '3', '.', ')']).trim().to_string())
                    .filter(|line| !line.is_empty())
                    .take(3)
                    .collect(),
            },
            Err(error) => Done::Failed(error),
        }
    });
}

static DIARY_CHECKED: Mutex<Option<std::time::Instant>> = Mutex::new(None);

/// [`diary_poll`] at most once a minute.
pub fn diary_poll_throttled(now: std::time::Instant) {
    {
        let mut checked = DIARY_CHECKED.lock();
        if checked.is_some_and(|at| now.saturating_duration_since(at) < std::time::Duration::from_secs(60)) {
            return;
        }
        *checked = Some(now);
    }
    diary_poll();
}

/// Replies the assistant suggested, by notification key, for the Phone tab.
static REPLIES: Mutex<Option<(String, Vec<String>)>> = Mutex::new(None);

pub fn replies_for(key: &str) -> Option<Vec<String>> {
    REPLIES.lock().as_ref().filter(|(known, _)| known == key).map(|(_, replies)| replies.clone())
}

#[derive(Serialize, Deserialize, Default)]
struct DiaryState {
    last_day: String,
}

/// Writes the day's diary once, after 9 pm. Call now and then from the UI thread.
pub fn diary_poll() {
    let local = crate::platform::shell().local_datetime();
    if local.hour < 21 {
        return;
    }
    let today = format!("{:04}-{:02}-{:02}", local.year, local.month, local.day);
    let mut state: DiaryState = crate::core::json_store::read("diary.json", 4096).ok().flatten().unwrap_or_default();
    if state.last_day == today {
        return;
    }
    state.last_day = today;
    let _ = crate::core::json_store::write("diary.json", &state);
    let since = memory::now_ms().saturating_sub(u64::from(local.hour) * 3_600_000 + u64::from(local.minute) * 60_000);
    let entries: Vec<Entry> = memory::since(since).into_iter().filter(|entry| entry.kind != Kind::Recap).collect();
    if entries.is_empty() {
        return;
    }
    let count = |kind: Kind| entries.iter().filter(|entry| entry.kind == kind).count();
    let tally = format!(
        "Today: {} phone alerts, {} Focus phases, {} meetings, {} files moved, {} downloads, {} notes.",
        count(Kind::PhoneAlert),
        count(Kind::Focus),
        count(Kind::Meeting) / 2,
        count(Kind::FileIn) + count(Kind::FileOut),
        count(Kind::Download),
        count(Kind::Note),
    );
    if !has_key() {
        memory::record(Kind::Recap, "Your day", tally.clone(), "");
        DONE.lock().push(Done::Diary(tally));
        return;
    }
    let mut oldest_first = entries;
    oldest_first.reverse();
    spawn(async move {
        let prompt = format!(
            "Write the user's short, warm diary entry for today (three or four sentences) from \
             this log: what they focused on, meetings, who messaged, anything left to do.\n\n{}\n{tally}",
            lines(&oldest_first, 6000)
        );
        let diary = brain::chat(&brain::settings().ai_model, &prompt, 260)
            .await
            .map(|text| text.trim().to_string())
            .unwrap_or(tally);
        memory::record(Kind::Recap, "Your day", diary.clone(), "");
        Done::Diary(diary)
    });
}
