use super::*;

fn place() -> Place {
    Place {
        name: "Berlin".into(),
        detail: "Berlin, Germany".into(),
        latitude: 52.52,
        longitude: 13.41,
    }
}

const FORECAST: &str = r#"{
    "latitude": 52.52,
    "longitude": 13.41,
    "current": {"time": "2026-10-30T14:00", "temperature_2m": 11.4,
                "apparent_temperature": 9.8, "weather_code": 61},
    "hourly": {
        "time": ["2026-10-30T14:00", "2026-10-30T15:00", "2026-10-30T16:00"],
        "temperature_2m": [11.4, null, 10.9],
        "precipitation_probability": [80, 70, null]
    }
}"#;

#[test]
fn parses_current_conditions_and_hourly_points() {
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000);
    let forecast = parse_forecast(FORECAST.as_bytes(), place(), Units::Metric, now).expect("parse");
    assert_eq!(forecast.temperature, 11.4);
    assert_eq!(forecast.apparent_temperature, 9.8);
    assert_eq!(forecast.condition, Condition::Rain);
    assert_eq!(forecast.temperature_symbol(), "°C");
    // The hour with a missing temperature is skipped; probabilities stay aligned.
    assert_eq!(forecast.hourly.len(), 2);
    assert_eq!(forecast.hourly[0].precipitation_probability, Some(80));
    assert_eq!(forecast.hourly[1].time, "2026-10-30T16:00");
    assert_eq!(forecast.hourly[1].precipitation_probability, None);
}

#[test]
fn rejects_malformed_forecasts() {
    let result = parse_forecast(
        b"{\"current\": {}}",
        place(),
        Units::Metric,
        SystemTime::now(),
    );
    assert!(matches!(result, Err(WeatherError::Parse(_))));
}

#[test]
fn freshness_follows_age() {
    let fetched = SystemTime::UNIX_EPOCH + Duration::from_secs(10_000);
    let mut forecast =
        parse_forecast(FORECAST.as_bytes(), place(), Units::Metric, fetched).expect("parse");
    assert_eq!(forecast.freshness(fetched), Freshness::Fresh);
    assert_eq!(forecast.freshness(fetched + FRESH_FOR), Freshness::Stale);
    assert_eq!(
        forecast.freshness(fetched + DISCARD_AFTER),
        Freshness::Expired
    );
    // A clock that moved backwards counts as just fetched.
    assert_eq!(
        forecast.age(fetched - Duration::from_secs(5)),
        Duration::ZERO
    );
    forecast.units = Units::Imperial;
    assert_eq!(forecast.temperature_symbol(), "°F");
}

#[test]
fn maps_wmo_codes() {
    assert_eq!(Condition::from_wmo(0), Condition::Clear);
    assert_eq!(Condition::from_wmo(2), Condition::PartlyCloudy);
    assert_eq!(Condition::from_wmo(48), Condition::Fog);
    assert_eq!(Condition::from_wmo(55), Condition::Drizzle);
    assert_eq!(Condition::from_wmo(75), Condition::Snow);
    assert_eq!(Condition::from_wmo(81), Condition::Showers);
    assert_eq!(Condition::from_wmo(96), Condition::Thunderstorm);
    assert_eq!(Condition::from_wmo(42), Condition::Unknown);
}

#[test]
fn builds_urls_with_coarse_coordinates() {
    let precise = Place {
        latitude: 52.520_008,
        longitude: 13.404_954,
        ..place()
    };
    let url = forecast_url(&precise, Units::Imperial);
    assert!(url.starts_with(FORECAST_ENDPOINT));
    assert!(url.contains("latitude=52.52&longitude=13.40"));
    assert!(url.contains("temperature_unit=fahrenheit"));
    assert!(!forecast_url(&place(), Units::Metric).contains("fahrenheit"));

    let search = geocoding_url("São Paulo", "pt").expect("url");
    assert!(search.starts_with(GEOCODING_ENDPOINT));
    assert!(search.contains("name=S%C3%A3o+Paulo"));
    assert!(geocoding_url("   ", "en").is_none());
}

#[test]
fn parses_places_and_rounds_coordinates() {
    let body = br#"{"results": [
        {"name": "Springfield", "latitude": 39.80172, "longitude": -89.64371,
         "admin1": "Illinois", "country": "United States"},
        {"name": "Springfield", "latitude": 37.21533, "longitude": -93.29824,
         "country": "United States"}
    ]}"#;
    let places = parse_places(body).expect("places");
    assert_eq!(places.len(), 2);
    assert_eq!(places[0].detail, "Illinois, United States");
    assert_eq!(places[0].latitude, 39.8);
    assert_eq!(places[0].longitude, -89.64);
    assert_eq!(places[1].detail, "United States");
    assert_eq!(parse_places(b"{}"), Err(WeatherError::NoResults));
}
