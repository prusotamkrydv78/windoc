//! Refreshes subscribed calendar feeds every 30 minutes while any are configured.

use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use winisland_core::calendar_feed::{self, CalendarEvent};

const REFRESH: Duration = Duration::from_secs(30 * 60);
const RETRY: Duration = Duration::from_secs(5 * 60);

pub struct CalendarService {
    feeds: Vec<String>,
    events: Vec<CalendarEvent>,
    pending: Option<Receiver<Vec<CalendarEvent>>>,
    next_fetch: Instant,
    revision: u64,
}

impl CalendarService {
    pub fn new() -> Self {
        Self {
            feeds: Vec::new(),
            events: Vec::new(),
            pending: None,
            next_fetch: Instant::now(),
            revision: 0,
        }
    }

    pub fn poll(&mut self, feeds: &[String], now: Instant) {
        if self.feeds.as_slice() != feeds {
            self.feeds = feeds.to_vec();
            self.next_fetch = now;
            if feeds.is_empty() {
                self.events.clear();
                self.revision = self.revision.wrapping_add(1);
            }
        }
        if let Some(receiver) = &self.pending {
            match receiver.try_recv() {
                Ok(events) => {
                    self.pending = None;
                    self.events = events;
                    self.revision = self.revision.wrapping_add(1);
                    self.next_fetch = now + REFRESH;
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.pending = None;
                    self.next_fetch = now + RETRY;
                }
            }
            return;
        }
        if feeds.is_empty() || now < self.next_fetch || super::safe_mode::active() {
            return;
        }
        let feeds = feeds.to_vec();
        let (sender, receiver) = mpsc::channel();
        tokio::spawn(async move {
            let mut events = Vec::new();
            for feed in feeds {
                match calendar_feed::fetch(&feed).await {
                    Ok(mut feed_events) => events.append(&mut feed_events),
                    Err(error) => log::warn!("Calendar feed failed: {error}"),
                }
            }
            events.sort();
            let _ = sender.send(events);
        });
        self.pending = Some(receiver);
        self.next_fetch = now + RETRY;
    }

    /// Agenda rows for the next seven days as `(title, detail)`.
    pub fn agenda_rows(&self) -> Vec<(String, String)> {
        let today = crate::platform::shell().local_datetime();
        let from = format!("{:04}{:02}{:02}", today.year, today.month, today.day);
        let start = winisland_core::screen_time::LocalDate::new(
            i32::from(today.year),
            u8::try_from(today.month).unwrap_or(1),
            u8::try_from(today.day).unwrap_or(1),
        )
        .day_number();
        let mut rows = Vec::new();
        for event in calendar_feed::upcoming(&self.events, &from, "99999999")
            .into_iter()
            .filter(|event| {
                let year: i32 = event.start[0..4].parse().unwrap_or(0);
                let month: u8 = event.start[4..6].parse().unwrap_or(0);
                let day: u8 = event.start[6..8].parse().unwrap_or(0);
                winisland_core::screen_time::LocalDate::new(year, month, day).day_number()
                    < start + 7
            })
            .take(50)
        {
            let when = match event.time() {
                Some(time) if event.utc => format!("{} {time} UTC", event.date()),
                Some(time) => format!("{} {time}", event.date()),
                None => event.date(),
            };
            let detail = if event.location.is_empty() {
                when
            } else {
                format!("{when} · {}", event.location)
            };
            rows.push((event.title.clone(), detail));
        }
        rows
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }
}
