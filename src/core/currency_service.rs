//! Fetches currency rates when the palette asks for them, at most every 12 hours.

use std::sync::mpsc::{self, Receiver};
use std::time::Instant;

use winisland_core::currency::{MAX_AGE, Rates, fetch_rates};

pub struct CurrencyService {
    fetched_at: Option<Instant>,
    pending: Option<Receiver<Result<Rates, String>>>,
}

impl CurrencyService {
    pub fn new() -> Self {
        Self {
            fetched_at: None,
            pending: None,
        }
    }

    /// Returns new rates when a fetch completes.
    pub fn poll(&mut self, requested: bool, now: Instant) -> Option<Rates> {
        if let Some(receiver) = &self.pending {
            match receiver.try_recv() {
                Ok(Ok(rates)) => {
                    self.pending = None;
                    self.fetched_at = Some(now);
                    return Some(rates);
                }
                Ok(Err(error)) => {
                    log::warn!("Currency rates unavailable: {error}");
                    self.pending = None;
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => self.pending = None,
            }
            return None;
        }
        let stale = self
            .fetched_at
            .is_none_or(|fetched| now.duration_since(fetched) >= MAX_AGE);
        if requested && stale && !super::safe_mode::active() {
            let (sender, receiver) = mpsc::channel();
            tokio::spawn(async move {
                let _ = sender.send(fetch_rates().await);
            });
            self.pending = Some(receiver);
        }
        None
    }
}
