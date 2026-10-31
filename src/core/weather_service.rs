//! Keeps a forecast for the configured place and caches it as `weather.json`.
//!
//! Nothing is requested unless weather is enabled and a place has been chosen.

use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant, SystemTime};

use winisland_core::config::AppConfig;
use winisland_core::weather::{FRESH_FOR, Forecast, Freshness, Place, Units, fetch_forecast};

use super::json_store;

pub(crate) const FILE_NAME: &str = "weather.json";
const MAX_FILE_BYTES: u64 = 256 * 1024;
const RETRY_AFTER: Duration = Duration::from_secs(5 * 60);

/// What the Home card shows.
#[derive(Clone, Debug, PartialEq)]
pub struct WeatherSummary {
    pub temperature: String,
    pub condition_key: &'static str,
    pub place: String,
    pub stale: bool,
}

pub struct WeatherService {
    forecast: Option<Forecast>,
    pending: Option<Receiver<Result<Forecast, String>>>,
    next_fetch: Instant,
}

impl WeatherService {
    pub fn load() -> Self {
        let forecast = match json_store::read::<Forecast>(FILE_NAME, MAX_FILE_BYTES) {
            Ok(forecast) => forecast,
            Err(error) => {
                log::warn!("Ignoring cached weather: {error}");
                None
            }
        };
        Self {
            forecast,
            pending: None,
            next_fetch: Instant::now(),
        }
    }

    /// Call from the frame loop. Returns `true` when a new forecast arrived.
    pub fn poll(&mut self, config: &AppConfig, now: Instant) -> bool {
        let Some(place) = config
            .weather_place
            .as_ref()
            .filter(|_| config.weather_enabled && !super::safe_mode::active())
        else {
            self.pending = None;
            return false;
        };
        let units = if config.weather_imperial {
            Units::Imperial
        } else {
            Units::Metric
        };
        let matches = self
            .forecast
            .as_ref()
            .is_some_and(|forecast| forecast.place == *place && forecast.units == units);
        if !matches && self.pending.is_none() {
            // The place or units changed; the cached forecast no longer applies.
            if self.forecast.take().is_some() {
                self.next_fetch = now;
            }
        }
        if let Some(receiver) = &self.pending {
            match receiver.try_recv() {
                Ok(Ok(forecast)) => {
                    self.pending = None;
                    if let Err(error) = json_store::write(FILE_NAME, &forecast) {
                        log::warn!("Cannot cache weather: {error}");
                    }
                    self.forecast = Some(forecast);
                    self.next_fetch = now + FRESH_FOR;
                    return true;
                }
                Ok(Err(error)) => {
                    log::warn!("Weather refresh failed: {error}");
                    self.pending = None;
                    self.next_fetch = now + RETRY_AFTER;
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    self.pending = None;
                    self.next_fetch = now + RETRY_AFTER;
                }
            }
            return false;
        }
        if now >= self.next_fetch {
            self.start_fetch(place.clone(), units);
        }
        false
    }

    fn start_fetch(&mut self, place: Place, units: Units) {
        let (sender, receiver) = mpsc::channel();
        tokio::spawn(async move {
            let result = fetch_forecast(place, units)
                .await
                .map_err(|error| error.to_string());
            let _ = sender.send(result);
        });
        self.pending = Some(receiver);
    }

    /// The current forecast for display, or `None` when it is missing or too old.
    pub fn summary(&self, config: &AppConfig) -> Option<WeatherSummary> {
        if !config.weather_enabled {
            return None;
        }
        let forecast = self.forecast.as_ref()?;
        let freshness = forecast.freshness(SystemTime::now());
        if freshness == Freshness::Expired {
            return None;
        }
        Some(WeatherSummary {
            temperature: format!(
                "{:.0}{}",
                forecast.temperature,
                forecast.temperature_symbol()
            ),
            condition_key: forecast.condition.label_key(),
            place: forecast.place.name.clone(),
            stale: freshness == Freshness::Stale,
        })
    }
}

impl WeatherService {
    /// Deletes the cached forecast. A new one is fetched only if weather is still on.
    pub fn clear(&mut self) {
        self.forecast = None;
        self.next_fetch = Instant::now();
        if let Err(error) = json_store::remove(FILE_NAME) {
            log::warn!("Cannot delete cached weather: {error}");
        }
    }
}

impl WeatherService {
    /// `(title, detail)` rows for the Weather page: details, next hours, then 5 days.
    pub fn detail_rows(&self, config: &AppConfig) -> Vec<(String, String)> {
        use winisland_core::i18n::{tr, tr_args};
        let Some(forecast) = self.forecast.as_ref().filter(|_| config.weather_enabled) else {
            return Vec::new();
        };
        let symbol = forecast.temperature_symbol();
        let mut rows = vec![(
            format!(
                "{:.0}{symbol} · {}",
                forecast.temperature,
                tr(forecast.condition.label_key())
            ),
            tr_args(
                "weather_feels_like",
                &[&format!("{:.0}{symbol}", forecast.apparent_temperature)],
            ),
        )];
        let mut details = Vec::new();
        if let Some(humidity) = forecast.humidity_percent {
            details.push(tr_args("weather_humidity", &[&format!("{humidity:.0}")]));
        }
        if let Some(wind) = forecast.wind_speed {
            details.push(tr_args("weather_wind", &[&format!("{wind:.0}")]));
        }
        if let Some(uv) = forecast.uv_index {
            details.push(tr_args("weather_uv", &[&format!("{uv:.0}")]));
        }
        if !details.is_empty() {
            rows.push((details.join(" · "), forecast.place.name.clone()));
        }
        for hour in forecast.hourly.iter().take(6) {
            let time = hour.time.get(11..16).unwrap_or(&hour.time).to_string();
            let rain = hour
                .precipitation_probability
                .map(|chance| tr_args("weather_rain_chance", &[&chance.to_string()]))
                .unwrap_or_default();
            rows.push((format!("{time} · {:.0}{symbol}", hour.temperature), rain));
        }
        for day in &forecast.daily {
            rows.push((
                format!(
                    "{} · {:.0}{symbol} / {:.0}{symbol}",
                    day.date, day.max, day.min
                ),
                tr(day.condition.label_key()),
            ));
        }
        rows
    }
}
