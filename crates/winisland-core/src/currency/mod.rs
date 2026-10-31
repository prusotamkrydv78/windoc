//! Currency conversion with rates from one provider (Frankfurter, ECB reference rates).
//!
//! Rates are fetched only after the user types a currency query, and every result shows
//! the rate date. A converted amount is never presented as a transaction price.

use std::collections::BTreeMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};

pub const RATES_URL: &str = "https://api.frankfurter.app/latest?from=EUR";
pub const MAX_AGE: Duration = Duration::from_secs(12 * 60 * 60);

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Rates {
    /// Provider date of the rates, such as `2026-10-30`.
    pub date: String,
    /// Units of each currency per one EUR, EUR included.
    pub per_eur: BTreeMap<String, f64>,
}

#[derive(Deserialize)]
struct FrankfurterResponse {
    date: String,
    rates: BTreeMap<String, f64>,
}

impl Rates {
    pub fn parse(body: &[u8]) -> Result<Self, String> {
        let response: FrankfurterResponse =
            serde_json::from_slice(body).map_err(|error| error.to_string())?;
        let mut per_eur = response.rates;
        per_eur.insert("EUR".to_string(), 1.0);
        Ok(Self {
            date: response.date,
            per_eur,
        })
    }

    pub fn convert(&self, value: f64, from: &str, to: &str) -> Option<f64> {
        let from = self.per_eur.get(from)?;
        let to = self.per_eur.get(to)?;
        (*from > 0.0).then(|| value / from * to)
    }
}

/// Parses `10 usd to inr` or `10usd in eur` into `(value, FROM, TO)`.
pub fn parse_query(input: &str) -> Option<(f64, String, String)> {
    let lower = input.trim().to_lowercase();
    let (left, right) = [" to ", " in "]
        .iter()
        .find_map(|separator| lower.split_once(separator))?;
    let left = left.trim();
    let split = left
        .find(|character: char| character.is_ascii_alphabetic())
        .unwrap_or(left.len());
    let value: f64 = left[..split].trim().replace(',', "").parse().ok()?;
    let code = |text: &str| {
        let text = text.trim();
        (text.len() == 3
            && text
                .chars()
                .all(|character| character.is_ascii_alphabetic()))
        .then(|| text.to_uppercase())
    };
    Some((value, code(&left[split..])?, code(right)?))
}

/// Fetches the latest reference rates.
pub async fn fetch_rates() -> Result<Rates, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|error| error.to_string())?;
    let response = client
        .get(RATES_URL)
        .send()
        .await
        .map_err(|error| error.to_string())?;
    if !response.status().is_success() {
        return Err(format!("HTTP {}", response.status().as_u16()));
    }
    let body = response.bytes().await.map_err(|error| error.to_string())?;
    if body.len() > 64 * 1024 {
        return Err("response too large".into());
    }
    Rates::parse(&body)
}
