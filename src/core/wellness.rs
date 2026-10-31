//! Water and stretch reminders shown as Windows notifications.

use std::time::{Duration, Instant};

use winisland_core::config::AppConfig;
use winisland_core::i18n::tr;

struct Reminder {
    title_key: &'static str,
    body_key: &'static str,
    interval: Option<Duration>,
    due: Option<Instant>,
}

impl Reminder {
    const fn new(title_key: &'static str, body_key: &'static str) -> Self {
        Self {
            title_key,
            body_key,
            interval: None,
            due: None,
        }
    }

    fn poll(&mut self, minutes: f32, now: Instant) {
        let interval = (minutes >= 1.0).then(|| Duration::from_secs_f32(minutes * 60.0));
        if interval != self.interval {
            self.interval = interval;
            self.due = interval.map(|interval| now + interval);
        }
        if let (Some(interval), Some(due)) = (self.interval, self.due)
            && now >= due
        {
            crate::platform::notify().show_toast(&tr(self.title_key), &tr(self.body_key));
            self.due = Some(now + interval);
        }
    }
}

pub struct WellnessReminders {
    water: Reminder,
    stretch: Reminder,
}

impl WellnessReminders {
    pub fn new() -> Self {
        Self {
            water: Reminder::new("water_reminder_title", "water_reminder_body"),
            stretch: Reminder::new("stretch_reminder_title", "stretch_reminder_body"),
        }
    }

    pub fn poll(&mut self, config: &AppConfig, now: Instant) {
        self.water.poll(config.water_reminder_minutes, now);
        self.stretch.poll(config.stretch_reminder_minutes, now);
    }
}
