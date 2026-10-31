use super::*;

fn unit(name: &str) -> &'static Unit {
    find(name).unwrap_or_else(|| panic!("unit {name}"))
}

fn close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() <= 1e-9 * expected.abs().max(1.0),
        "{actual} != {expected}"
    );
}

fn converted(value: f64, from: &str, to: &str) -> f64 {
    convert(value, unit(from), unit(to)).expect("convert")
}

#[test]
fn every_category_has_units_and_symbols_are_unique() {
    for category in Category::ALL {
        assert!(units_in(category).count() >= 3, "{category:?}");
    }
    for (index, a) in UNITS.iter().enumerate() {
        for b in &UNITS[index + 1..] {
            assert_ne!(a.symbol, b.symbol);
        }
    }
}

#[test]
fn lookup_accepts_symbols_and_aliases() {
    assert_eq!(unit("km").symbol, "km");
    assert_eq!(unit("Kilometres").symbol, "km");
    assert_eq!(unit("K").symbol, "K");
    assert_eq!(unit("c").symbol, "°C");
    assert_eq!(unit("MB").symbol, "MB");
    assert_eq!(unit("mib").symbol, "MiB");
    assert!(find("").is_none());
    assert!(find("furlong").is_none());
}

#[test]
fn linear_conversions() {
    close(converted(1.0, "mi", "km"), 1.609_344);
    close(converted(12.0, "in", "ft"), 1.0);
    close(converted(1.0, "lb", "g"), 453.592_37);
    close(converted(1.0, "GiB", "MiB"), 1024.0);
    close(converted(1.0, "GB", "MB"), 1000.0);
    close(converted(8.0, "bit", "B"), 1.0);
    close(converted(36.0, "km/h", "m/s"), 10.0);
    close(converted(1.0, "kWh", "kJ"), 3600.0);
    close(converted(1.0, "ha", "m²"), 10_000.0);
    close(converted(90.0, "min", "h"), 1.5);
}

#[test]
fn temperature_conversions() {
    close(converted(0.0, "°C", "°F"), 32.0);
    close(converted(100.0, "°C", "K"), 373.15);
    close(converted(-40.0, "°F", "°C"), -40.0);
    close(converted(0.0, "K", "°C"), -273.15);
    assert_eq!(
        convert(-300.0, unit("°C"), unit("K")),
        Err(ConvertError::InvalidValue)
    );
}

#[test]
fn rejects_mismatched_and_invalid_input() {
    assert_eq!(
        convert(1.0, unit("km"), unit("kg")),
        Err(ConvertError::IncompatibleCategories)
    );
    assert_eq!(
        convert(f64::NAN, unit("km"), unit("m")),
        Err(ConvertError::InvalidValue)
    );
}

#[test]
fn parses_natural_queries() {
    let query = parse("10 km to mi").expect("query");
    assert_eq!(
        (query.value, query.from.symbol, query.to.symbol),
        (10.0, "km", "mi")
    );

    let query = parse("72 F in c").expect("query");
    assert_eq!((query.from.symbol, query.to.symbol), ("°F", "°C"));

    let query = parse("5 fl oz in ml").expect("query");
    assert_eq!(query.from.symbol, "fl oz");

    let query = parse("10 in to cm").expect("query");
    assert_eq!(query.from.symbol, "in");

    let query = parse("-40 c to f").expect("query");
    assert_eq!(query.value, -40.0);

    let query = parse("1,000 m as km").expect("query");
    assert_eq!(query.value, 1000.0);

    let query = parse("10km->mi").expect("query");
    assert_eq!(query.from.symbol, "km");

    assert!(parse("10 km to kg").is_none());
    assert!(parse("timer 25").is_none());
    assert!(parse("km to mi").is_none());
}

#[test]
fn formats_without_trailing_zeros() {
    assert_eq!(format_value(1.0, 4), "1");
    assert_eq!(format_value(1.5, 4), "1.5");
    assert_eq!(format_value(1.234_567, 3), "1.235");
    assert_eq!(format_value(-0.000_01, 2), "0");
    assert_eq!(format_value(1200.0, 0), "1200");
}
