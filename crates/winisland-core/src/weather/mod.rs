//! Weather through a provider abstraction, starting with Open-Meteo (workflow §11).
//!
//! Privacy: nothing is requested until the user enables weather and chooses a place. A
//! manual city is the default; coordinates come from Open-Meteo's geocoding search and are
//! rounded to two decimals (about 1 km) before they are stored or sent.

use std::sync::LazyLock;
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};

pub const FORECAST_ENDPOINT: &str = "https://api.open-meteo.com/v1/forecast";
pub const GEOCODING_ENDPOINT: &str = "https://geocoding-api.open-meteo.com/v1/search";
/// Forecasts younger than this are shown without a refresh.
pub const FRESH_FOR: Duration = Duration::from_secs(30 * 60);
/// Older forecasts are hidden rather than shown as current.
pub const DISCARD_AFTER: Duration = Duration::from_secs(12 * 60 * 60);
const MAX_RESPONSE_BYTES: usize = 512 * 1024;
const HOURS_KEPT: usize = 24;

static HTTP_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap_or_default()
});

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Place {
    pub name: String,
    /// Region and country for disambiguation, such as "Bavaria, Germany".
    pub detail: String,
    pub latitude: f64,
    pub longitude: f64,
}

impl Place {
    /// Rounds coordinates to two decimals so a precise location is never kept.
    pub fn coarse(mut self) -> Self {
        self.latitude = (self.latitude * 100.0).round() / 100.0;
        self.longitude = (self.longitude * 100.0).round() / 100.0;
        self
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Units {
    #[default]
    Metric,
    Imperial,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Condition {
    Clear,
    PartlyCloudy,
    Cloudy,
    Fog,
    Drizzle,
    Rain,
    Snow,
    Showers,
    Thunderstorm,
    Unknown,
}

impl Condition {
    /// Maps a WMO weather interpretation code.
    pub fn from_wmo(code: u32) -> Self {
        match code {
            0 | 1 => Self::Clear,
            2 => Self::PartlyCloudy,
            3 => Self::Cloudy,
            45 | 48 => Self::Fog,
            51..=57 => Self::Drizzle,
            61..=67 => Self::Rain,
            71..=77 | 85 | 86 => Self::Snow,
            80..=82 => Self::Showers,
            95..=99 => Self::Thunderstorm,
            _ => Self::Unknown,
        }
    }

    pub const fn label_key(self) -> &'static str {
        match self {
            Self::Clear => "weather_clear",
            Self::PartlyCloudy => "weather_partly_cloudy",
            Self::Cloudy => "weather_cloudy",
            Self::Fog => "weather_fog",
            Self::Drizzle => "weather_drizzle",
            Self::Rain => "weather_rain",
            Self::Snow => "weather_snow",
            Self::Showers => "weather_showers",
            Self::Thunderstorm => "weather_thunderstorm",
            Self::Unknown => "weather_unknown",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct HourlyPoint {
    /// Local time as reported by the provider, such as `2026-10-30T14:00`.
    pub time: String,
    pub temperature: f64,
    pub precipitation_probability: Option<u8>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct DailyPoint {
    /// Provider date such as `2026-10-30`.
    pub date: String,
    pub condition: Condition,
    pub max: f64,
    pub min: f64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Forecast {
    pub place: Place,
    pub units: Units,
    pub temperature: f64,
    pub apparent_temperature: f64,
    pub condition: Condition,
    pub hourly: Vec<HourlyPoint>,
    #[serde(default)]
    pub humidity_percent: Option<f64>,
    #[serde(default)]
    pub wind_speed: Option<f64>,
    #[serde(default)]
    pub uv_index: Option<f64>,
    #[serde(default)]
    pub daily: Vec<DailyPoint>,
    pub fetched_at: SystemTime,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Freshness {
    Fresh,
    /// Shown with its age; a refresh is due.
    Stale,
    /// Too old to show as current weather.
    Expired,
}

impl Forecast {
    pub fn age(&self, now: SystemTime) -> Duration {
        now.duration_since(self.fetched_at)
            .unwrap_or(Duration::ZERO)
    }

    pub fn freshness(&self, now: SystemTime) -> Freshness {
        let age = self.age(now);
        if age < FRESH_FOR {
            Freshness::Fresh
        } else if age < DISCARD_AFTER {
            Freshness::Stale
        } else {
            Freshness::Expired
        }
    }

    pub const fn temperature_symbol(&self) -> &'static str {
        match self.units {
            Units::Metric => "°C",
            Units::Imperial => "°F",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WeatherError {
    Network(String),
    Status(u16),
    TooLarge,
    Parse(String),
    NoResults,
}

impl std::fmt::Display for WeatherError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Network(error) => write!(f, "network error: {error}"),
            Self::Status(status) => write!(f, "weather service returned HTTP {status}"),
            Self::TooLarge => write!(f, "weather response is too large"),
            Self::Parse(error) => write!(f, "unexpected weather response: {error}"),
            Self::NoResults => write!(f, "no matching place"),
        }
    }
}

impl std::error::Error for WeatherError {}

pub fn forecast_url(place: &Place, units: Units) -> String {
    let mut url = format!(
        "{FORECAST_ENDPOINT}?latitude={:.2}&longitude={:.2}\
         &current=temperature_2m,apparent_temperature,weather_code,\
         relative_humidity_2m,wind_speed_10m,uv_index\
         &hourly=temperature_2m,precipitation_probability\
         &daily=weather_code,temperature_2m_max,temperature_2m_min&forecast_days=5\
         &forecast_hours={HOURS_KEPT}&timezone=auto",
        place.latitude, place.longitude
    );
    if units == Units::Imperial {
        url.push_str("&temperature_unit=fahrenheit");
    }
    url
}

pub fn geocoding_url(query: &str, language: &str) -> Option<String> {
    let query = query.trim();
    if query.is_empty() {
        return None;
    }
    reqwest::Url::parse_with_params(
        GEOCODING_ENDPOINT,
        &[
            ("name", query),
            ("count", "5"),
            ("language", language),
            ("format", "json"),
        ],
    )
    .ok()
    .map(String::from)
}

#[derive(Deserialize)]
struct ForecastResponse {
    current: CurrentResponse,
    hourly: Option<HourlyResponse>,
    daily: Option<DailyResponse>,
}

#[derive(Deserialize)]
struct CurrentResponse {
    temperature_2m: f64,
    apparent_temperature: f64,
    weather_code: u32,
    relative_humidity_2m: Option<f64>,
    wind_speed_10m: Option<f64>,
    uv_index: Option<f64>,
}

#[derive(Deserialize)]
struct DailyResponse {
    time: Vec<String>,
    weather_code: Vec<Option<u32>>,
    temperature_2m_max: Vec<Option<f64>>,
    temperature_2m_min: Vec<Option<f64>>,
}

#[derive(Deserialize)]
struct HourlyResponse {
    time: Vec<String>,
    temperature_2m: Vec<Option<f64>>,
    precipitation_probability: Option<Vec<Option<u8>>>,
}

pub fn parse_forecast(
    body: &[u8],
    place: Place,
    units: Units,
    now: SystemTime,
) -> Result<Forecast, WeatherError> {
    let response: ForecastResponse =
        serde_json::from_slice(body).map_err(|error| WeatherError::Parse(error.to_string()))?;
    let hourly = response
        .hourly
        .map(|hourly| {
            let probabilities = hourly.precipitation_probability.unwrap_or_default();
            hourly
                .time
                .into_iter()
                .zip(hourly.temperature_2m)
                .enumerate()
                .filter_map(|(index, (time, temperature))| {
                    Some(HourlyPoint {
                        time,
                        temperature: temperature?,
                        precipitation_probability: probabilities.get(index).copied().flatten(),
                    })
                })
                .take(HOURS_KEPT)
                .collect()
        })
        .unwrap_or_default();
    let daily = response
        .daily
        .map(|daily| {
            daily
                .time
                .into_iter()
                .enumerate()
                .filter_map(|(index, date)| {
                    Some(DailyPoint {
                        date,
                        condition: Condition::from_wmo(
                            daily.weather_code.get(index).copied().flatten()?,
                        ),
                        max: daily.temperature_2m_max.get(index).copied().flatten()?,
                        min: daily.temperature_2m_min.get(index).copied().flatten()?,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(Forecast {
        place,
        units,
        temperature: response.current.temperature_2m,
        apparent_temperature: response.current.apparent_temperature,
        condition: Condition::from_wmo(response.current.weather_code),
        hourly,
        humidity_percent: response.current.relative_humidity_2m,
        wind_speed: response.current.wind_speed_10m,
        uv_index: response.current.uv_index,
        daily,
        fetched_at: now,
    })
}

#[derive(Deserialize)]
struct GeocodingResponse {
    #[serde(default)]
    results: Vec<GeocodingResult>,
}

#[derive(Deserialize)]
struct GeocodingResult {
    name: String,
    latitude: f64,
    longitude: f64,
    admin1: Option<String>,
    country: Option<String>,
}

pub fn parse_places(body: &[u8]) -> Result<Vec<Place>, WeatherError> {
    let response: GeocodingResponse =
        serde_json::from_slice(body).map_err(|error| WeatherError::Parse(error.to_string()))?;
    if response.results.is_empty() {
        return Err(WeatherError::NoResults);
    }
    Ok(response
        .results
        .into_iter()
        .map(|result| {
            let detail = [result.admin1, result.country]
                .into_iter()
                .flatten()
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>()
                .join(", ");
            Place {
                name: result.name,
                detail,
                latitude: result.latitude,
                longitude: result.longitude,
            }
            .coarse()
        })
        .collect())
}

async fn get(url: &str) -> Result<Vec<u8>, WeatherError> {
    let response = HTTP_CLIENT
        .get(url)
        .send()
        .await
        .map_err(|error| WeatherError::Network(error.to_string()))?;
    let status = response.status();
    if !status.is_success() {
        return Err(WeatherError::Status(status.as_u16()));
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
    {
        return Err(WeatherError::TooLarge);
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|error| WeatherError::Network(error.to_string()))?;
    if bytes.len() > MAX_RESPONSE_BYTES {
        return Err(WeatherError::TooLarge);
    }
    Ok(bytes.to_vec())
}

/// Fetches the forecast for `place`. Only call after the user enabled weather.
pub async fn fetch_forecast(place: Place, units: Units) -> Result<Forecast, WeatherError> {
    let body = get(&forecast_url(&place, units)).await?;
    parse_forecast(&body, place, units, SystemTime::now())
}

/// Searches places by name. Only call in response to the user typing a search.
pub async fn search_places(query: &str, language: &str) -> Result<Vec<Place>, WeatherError> {
    let url = geocoding_url(query, language).ok_or(WeatherError::NoResults)?;
    parse_places(&get(&url).await?)
}

#[cfg(test)]
mod tests;
