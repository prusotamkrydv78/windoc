//! Offline unit conversion (workflow §12).
//!
//! Physical conversions use fixed, versioned constants and never touch the network.
//! Currency is handled elsewhere, because rates change and need a provider and timestamp.

pub const CONSTANTS_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Category {
    Length,
    Mass,
    Temperature,
    Volume,
    Speed,
    DataSize,
    Time,
    Area,
    Energy,
}

impl Category {
    pub const ALL: [Self; 9] = [
        Self::Length,
        Self::Mass,
        Self::Temperature,
        Self::Volume,
        Self::Speed,
        Self::DataSize,
        Self::Time,
        Self::Area,
        Self::Energy,
    ];

    pub const fn label_key(self) -> &'static str {
        match self {
            Self::Length => "units_length",
            Self::Mass => "units_mass",
            Self::Temperature => "units_temperature",
            Self::Volume => "units_volume",
            Self::Speed => "units_speed",
            Self::DataSize => "units_data_size",
            Self::Time => "units_time",
            Self::Area => "units_area",
            Self::Energy => "units_energy",
        }
    }
}

/// How a unit maps to its category's base unit.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Scale {
    /// base = value × factor
    Linear(f64),
    /// Temperature: base kelvin = (value + offset) × factor
    Affine { factor: f64, offset: f64 },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Unit {
    pub symbol: &'static str,
    pub category: Category,
    /// Other accepted spellings, lowercase.
    aliases: &'static [&'static str],
    scale: Scale,
}

const fn linear(
    symbol: &'static str,
    category: Category,
    factor: f64,
    aliases: &'static [&'static str],
) -> Unit {
    Unit {
        symbol,
        category,
        aliases,
        scale: Scale::Linear(factor),
    }
}

use Category::*;

/// Base units: metre, kilogram, kelvin, litre, metre per second, byte, second, square
/// metre and joule.
pub const UNITS: &[Unit] = &[
    linear(
        "mm",
        Length,
        0.001,
        &["millimeter", "millimetre", "millimeters"],
    ),
    linear(
        "cm",
        Length,
        0.01,
        &["centimeter", "centimetre", "centimeters"],
    ),
    linear("m", Length, 1.0, &["meter", "metre", "meters", "metres"]),
    linear(
        "km",
        Length,
        1000.0,
        &["kilometer", "kilometre", "kilometers", "kilometres"],
    ),
    linear("in", Length, 0.0254, &["inch", "inches", "\""]),
    linear("ft", Length, 0.3048, &["foot", "feet", "'"]),
    linear("yd", Length, 0.9144, &["yard", "yards"]),
    linear("mi", Length, 1609.344, &["mile", "miles"]),
    linear("nmi", Length, 1852.0, &["nautical mile", "nautical miles"]),
    linear("mg", Mass, 0.000_001, &["milligram", "milligrams"]),
    linear("g", Mass, 0.001, &["gram", "grams"]),
    linear("kg", Mass, 1.0, &["kilogram", "kilograms", "kilo", "kilos"]),
    linear("t", Mass, 1000.0, &["tonne", "tonnes", "metric ton"]),
    linear("oz", Mass, 0.028_349_523_125, &["ounce", "ounces"]),
    linear("lb", Mass, 0.453_592_37, &["lbs", "pound", "pounds"]),
    linear("st", Mass, 6.350_293_18, &["stone", "stones"]),
    Unit {
        symbol: "°C",
        category: Temperature,
        aliases: &["c", "celsius", "degc"],
        scale: Scale::Affine {
            factor: 1.0,
            offset: 273.15,
        },
    },
    Unit {
        symbol: "°F",
        category: Temperature,
        aliases: &["f", "fahrenheit", "degf"],
        scale: Scale::Affine {
            factor: 5.0 / 9.0,
            offset: 459.67,
        },
    },
    Unit {
        symbol: "K",
        category: Temperature,
        aliases: &["k", "kelvin"],
        scale: Scale::Affine {
            factor: 1.0,
            offset: 0.0,
        },
    },
    linear(
        "ml",
        Volume,
        0.001,
        &["milliliter", "millilitre", "milliliters"],
    ),
    linear("l", Volume, 1.0, &["liter", "litre", "liters", "litres"]),
    linear("m³", Volume, 1000.0, &["m3", "cubic meter", "cubic metre"]),
    linear(
        "tsp",
        Volume,
        0.004_928_921_593_75,
        &["teaspoon", "teaspoons"],
    ),
    linear(
        "tbsp",
        Volume,
        0.014_786_764_781_25,
        &["tablespoon", "tablespoons"],
    ),
    linear(
        "fl oz",
        Volume,
        0.029_573_529_562_5,
        &["floz", "fluid ounce", "fluid ounces"],
    ),
    linear("cup", Volume, 0.236_588_236_5, &["cups"]),
    linear("gal", Volume, 3.785_411_784, &["gallon", "gallons"]),
    linear("m/s", Speed, 1.0, &["mps", "meters per second"]),
    linear(
        "km/h",
        Speed,
        1000.0 / 3600.0,
        &["kmh", "kph", "kilometers per hour"],
    ),
    linear("mph", Speed, 1609.344 / 3600.0, &["miles per hour"]),
    linear("kn", Speed, 1852.0 / 3600.0, &["knot", "knots", "kt"]),
    linear("B", DataSize, 1.0, &["b", "byte", "bytes"]),
    linear("KB", DataSize, 1000.0, &["kb", "kilobyte", "kilobytes"]),
    linear("MB", DataSize, 1e6, &["mb", "megabyte", "megabytes"]),
    linear("GB", DataSize, 1e9, &["gb", "gigabyte", "gigabytes"]),
    linear("TB", DataSize, 1e12, &["tb", "terabyte", "terabytes"]),
    linear("KiB", DataSize, 1024.0, &["kib", "kibibyte", "kibibytes"]),
    linear(
        "MiB",
        DataSize,
        1_048_576.0,
        &["mib", "mebibyte", "mebibytes"],
    ),
    linear(
        "GiB",
        DataSize,
        1_073_741_824.0,
        &["gib", "gibibyte", "gibibytes"],
    ),
    linear("bit", DataSize, 0.125, &["bits"]),
    linear("ms", Time, 0.001, &["millisecond", "milliseconds"]),
    linear("s", Time, 1.0, &["sec", "secs", "second", "seconds"]),
    linear("min", Time, 60.0, &["mins", "minute", "minutes"]),
    linear("h", Time, 3600.0, &["hr", "hrs", "hour", "hours"]),
    linear("d", Time, 86_400.0, &["day", "days"]),
    linear("wk", Time, 604_800.0, &["week", "weeks"]),
    linear(
        "m²",
        Area,
        1.0,
        &["m2", "sq m", "square meter", "square metre"],
    ),
    linear("km²", Area, 1e6, &["km2", "sq km", "square kilometer"]),
    linear(
        "ft²",
        Area,
        0.092_903_04,
        &["ft2", "sq ft", "square foot", "square feet"],
    ),
    linear("ha", Area, 10_000.0, &["hectare", "hectares"]),
    linear("ac", Area, 4_046.856_422_4, &["acre", "acres"]),
    linear("J", Energy, 1.0, &["j", "joule", "joules"]),
    linear("kJ", Energy, 1000.0, &["kj", "kilojoule", "kilojoules"]),
    linear("cal", Energy, 4.184, &["calorie", "calories"]),
    linear(
        "kcal",
        Energy,
        4184.0,
        &["kilocalorie", "kilocalories", "food calorie"],
    ),
    linear("Wh", Energy, 3600.0, &["wh", "watt hour", "watt hours"]),
    linear(
        "kWh",
        Energy,
        3_600_000.0,
        &["kwh", "kilowatt hour", "kilowatt hours"],
    ),
];

/// Finds a unit by symbol (exact, then case-insensitive) or alias.
pub fn find(name: &str) -> Option<&'static Unit> {
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    if let Some(unit) = UNITS.iter().find(|unit| unit.symbol == name) {
        return Some(unit);
    }
    let lower = name.to_lowercase();
    UNITS.iter().find(|unit| {
        unit.symbol.to_lowercase() == lower || unit.aliases.iter().any(|alias| *alias == lower)
    })
}

pub fn units_in(category: Category) -> impl Iterator<Item = &'static Unit> {
    UNITS.iter().filter(move |unit| unit.category == category)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConvertError {
    IncompatibleCategories,
    /// Below absolute zero, NaN or infinite.
    InvalidValue,
}

impl std::fmt::Display for ConvertError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::IncompatibleCategories => write!(f, "units measure different things"),
            Self::InvalidValue => write!(f, "value is out of range"),
        }
    }
}

impl std::error::Error for ConvertError {}

fn to_base(value: f64, unit: &Unit) -> f64 {
    match unit.scale {
        Scale::Linear(factor) => value * factor,
        Scale::Affine { factor, offset } => (value + offset) * factor,
    }
}

fn from_base(value: f64, unit: &Unit) -> f64 {
    match unit.scale {
        Scale::Linear(factor) => value / factor,
        Scale::Affine { factor, offset } => value / factor - offset,
    }
}

pub fn convert(value: f64, from: &Unit, to: &Unit) -> Result<f64, ConvertError> {
    if from.category != to.category {
        return Err(ConvertError::IncompatibleCategories);
    }
    if !value.is_finite() {
        return Err(ConvertError::InvalidValue);
    }
    let base = to_base(value, from);
    if from.category == Category::Temperature && base < -1e-9 {
        return Err(ConvertError::InvalidValue);
    }
    let result = from_base(base, to);
    if result.is_finite() {
        Ok(result)
    } else {
        Err(ConvertError::InvalidValue)
    }
}

/// A parsed request such as `10 km to mi` or `72 f in c`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Query {
    pub value: f64,
    pub from: &'static Unit,
    pub to: &'static Unit,
}

/// Parses `<number> <unit> (to|in|as|->) <unit>`. The number and unit may be joined
/// (`10km`). Returns `None` for anything else, so a command palette can try other parsers.
pub fn parse(input: &str) -> Option<Query> {
    let lower = input.trim().to_lowercase();
    let (left, right) = [" to ", " in ", " as ", " -> ", "->"]
        .iter()
        .find_map(|separator| lower.split_once(separator))?;
    let left = left.trim();
    let number_end = left
        .char_indices()
        .find(|(index, character)| {
            !(character.is_ascii_digit()
                || *character == '.'
                || *character == ','
                || (*index == 0 && (*character == '-' || *character == '+')))
        })
        .map_or(left.len(), |(index, _)| index);
    let value: f64 = left[..number_end].replace(',', "").parse().ok()?;
    let from = find(&left[number_end..])?;
    let to = find(right)?;
    (from.category == to.category).then_some(Query { value, from, to })
}

/// Formats a result with up to `max_decimals`, trimming trailing zeros.
pub fn format_value(value: f64, max_decimals: usize) -> String {
    let text = format!("{value:.max_decimals$}");
    let text = if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        text
    };
    if text == "-0" { "0".to_string() } else { text }
}

#[cfg(test)]
mod tests;
