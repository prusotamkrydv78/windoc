//! The robot learns the user's routine from Notchy Memory (last 14 days):
//! - the hour Focus sessions usually finish in → a nudge at that hour when none is running;
//! - the hour activity usually stops at night → "winding down?" when still busy an hour later;
//! - every Sunday after 6 pm, a weekly spending report (AI-written with a key).
//!
//! Each nudge happens at most once a day; the state lives in `routine.json`.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::core::memory::{self, Kind};

const DAYS: u64 = 14;
const MIN_DAYS: usize = 4;

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
struct State {
    focus_nudged: String,
    bedtime_nudged: String,
    spending_week: String,
}

pub enum Nudge {
    Focus(u16),
    Bedtime(u16),
    Spending(String),
}

/// Local hour of a Unix time, using the PC's current UTC offset (close enough for habits).
fn local_hour(ms: u64, offset_minutes: i64) -> u16 {
    let minutes = (ms / 60_000) as i64 + offset_minutes;
    (minutes.rem_euclid(1440) / 60) as u16
}

fn local_day(ms: u64, offset_minutes: i64) -> i64 {
    ((ms / 60_000) as i64 + offset_minutes).div_euclid(1440)
}

/// The hour in which something happened on the most distinct days, if on `MIN_DAYS`+ days.
fn habit_hour(times: &[u64], offset: i64) -> Option<u16> {
    let mut days_by_hour: HashMap<u16, std::collections::HashSet<i64>> = HashMap::new();
    for &at in times {
        days_by_hour
            .entry(local_hour(at, offset))
            .or_default()
            .insert(local_day(at, offset));
    }
    days_by_hour
        .into_iter()
        .filter(|(_, days)| days.len() >= MIN_DAYS)
        .max_by_key(|(_, days)| days.len())
        .map(|(hour, _)| hour)
}

/// Checks the routine; call about once a minute from the UI thread.
pub fn poll(focusing: bool) -> Option<Nudge> {
    let local = crate::platform::shell().local_datetime();
    let now = memory::now_ms();
    // The PC's UTC offset, from its local clock.
    let utc_minutes = (now / 60_000) as i64;
    let local_minutes = utc_minutes.div_euclid(1440) * 1440 + i64::from(local.hour) * 60 + i64::from(local.minute);
    let mut offset = local_minutes - utc_minutes;
    if offset > 720 {
        offset -= 1440;
    } else if offset < -720 {
        offset += 1440;
    }
    let today = format!("{:04}-{:02}-{:02}", local.year, local.month, local.day);
    let mut state: State = crate::core::json_store::read("routine.json", 4096).ok().flatten().unwrap_or_default();
    let history = memory::since(now.saturating_sub(DAYS * 86_400_000));
    let save = |state: &State| {
        let _ = crate::core::json_store::write("routine.json", state);
    };

    // Usual Focus hour.
    if !focusing && state.focus_nudged != today && local.minute < 15 {
        let focus: Vec<u64> = history.iter().filter(|entry| entry.kind == Kind::Focus).map(|entry| entry.at).collect();
        if habit_hour(&focus, offset).is_some_and(|hour| hour == local.hour) {
            state.focus_nudged = today.clone();
            save(&state);
            return Some(Nudge::Focus(local.hour));
        }
    }

    // Usual bedtime: the last activity hour of each evening.
    if state.bedtime_nudged != today && (local.hour >= 21 || local.hour < 3) {
        let mut last_by_day: HashMap<i64, u64> = HashMap::new();
        for entry in &history {
            let hour = local_hour(entry.at, offset);
            if hour >= 20 || hour < 3 {
                // After midnight counts for the evening before.
                let day = local_day(entry.at.saturating_sub(3 * 3_600_000), offset);
                let slot = last_by_day.entry(day).or_default();
                *slot = (*slot).max(entry.at);
            }
        }
        let lasts: Vec<u64> = last_by_day.into_values().collect();
        if let Some(bedtime) = habit_hour(&lasts, offset) {
            let late = (local.hour + 24 - bedtime) % 24;
            if (1..=2).contains(&late) && state.bedtime_nudged != today {
                state.bedtime_nudged = today.clone();
                save(&state);
                return Some(Nudge::Bedtime(bedtime));
            }
        }
    }

    // Sunday evening spending report (day_of_week: 0 = Sunday).
    if local.day_of_week == 0 && local.hour >= 18 && state.spending_week != today {
        state.spending_week = today;
        save(&state);
        let week: Vec<_> = memory::since(now.saturating_sub(7 * 86_400_000))
            .into_iter()
            .filter(|entry| entry.kind == Kind::Expense)
            .collect();
        if !week.is_empty() {
            return Some(Nudge::Spending(spending_summary(&week)));
        }
    }
    None
}

/// "₹4,320 across 12 payments · Swiggy ₹1,250 · Amazon ₹980 · …" from expense titles
/// (`₹450.00 · Swiggy`).
fn spending_summary(week: &[memory::Entry]) -> String {
    let mut total = 0.0f64;
    let mut by_merchant: HashMap<String, f64> = HashMap::new();
    for entry in week {
        let Some((amount, merchant)) = entry.title.trim_start_matches('₹').split_once(" · ") else {
            continue;
        };
        let amount: f64 = amount.replace(',', "").parse().unwrap_or(0.0);
        total += amount;
        *by_merchant.entry(merchant.to_string()).or_default() += amount;
    }
    let mut merchants: Vec<_> = by_merchant.into_iter().collect();
    merchants.sort_by(|a, b| b.1.total_cmp(&a.1));
    let top: Vec<String> = merchants
        .iter()
        .take(4)
        .map(|(name, amount)| format!("{name} ₹{amount:.0}"))
        .collect();
    format!("₹{total:.0} across {} payments · {}", week.len(), top.join(" · "))
}

/// The weekly report in friendlier words when an AI key is set; otherwise the plain summary.
pub async fn spending_report(summary: String) -> String {
    if crate::core::brain::settings().ai_key.is_empty() {
        return summary;
    }
    let prompt = format!(
        "Here is the user's spending this week: {summary}. In two short, kind sentences, say \
         where most money went and one practical tip. No lists."
    );
    crate::core::brain::chat(&crate::core::brain::settings().ai_model, &prompt, 140)
        .await
        .map(|text| text.trim().to_string())
        .unwrap_or(summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn habit_needs_enough_days() {
        let day = 86_400_000u64;
        let at_ten = 10 * 3_600_000u64;
        let times: Vec<u64> = (0..5).map(|index| index * day + at_ten).collect();
        assert_eq!(habit_hour(&times, 0), Some(10));
        assert_eq!(habit_hour(&times[..3], 0), None);
    }
}
