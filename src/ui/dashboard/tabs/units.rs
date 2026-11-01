//! Units tab: a two-pane converter.
//!
//! The left card lists the categories (length, mass, temperature, data, volume, speed,
//! time and currency). The right card shows an editable "From" amount, a swap button, the
//! "To" result with a copy button, three related conversions and the recent conversions.
//! Unit conversion uses `winisland_core::units`; currency uses ECB reference rates fetched
//! here on demand (and shared with the command palette through `palette::set_rates`).

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};

use winisland_core::currency::{MAX_AGE, Rates};
use winisland_core::text_edit::{EditKey, TextBuffer};
use winisland_core::units::{self as engine, Category, Unit};
use winisland_platform::Key;
use winisland_render::text::FontManager;
use winisland_render::tokens::{SemanticColors, TextStyle, radius};
use winisland_render::{
    FontStyle, Painter, Path, Point, Radius, Rect, Rgba, StrokeCap, StrokeJoin, Vec2,
};

use super::TabContext;
use crate::ui::components::text::{Align, draw_text};
use crate::ui::dashboard::DashboardHit;

// Hit ids (800-899).
const ID_CATEGORY: u16 = 800; // ..=807
const ID_FROM_FIELD: u16 = 810;
const ID_FROM_UNIT: u16 = 811;
const ID_TO_UNIT: u16 = 812;
const ID_SWAP: u16 = 813;
const ID_COPY: u16 = 814;
const ID_RELATED: u16 = 820; // ..=822
const ID_RECENT: u16 = 830; // ..=835
const ID_BACKGROUND: u16 = 899;

const SIDE_GAP: f32 = 8.0;
const PAD: f32 = 16.0;
const MAX_VALUE_CHARS: usize = 18;
const MAX_RECENT: usize = 6;
const RETRY_AFTER: Duration = Duration::from_secs(60);
const COPIED_FOR: Duration = Duration::from_millis(1500);

const LABEL: TextStyle = TextStyle {
    size: 11.0,
    weight: 500,
    tabular: false,
};
const ROW: TextStyle = TextStyle {
    size: 13.0,
    weight: 500,
    tabular: false,
};
const ROW_BOLD: TextStyle = TextStyle {
    size: 13.0,
    weight: 600,
    tabular: false,
};
const SMALL: TextStyle = TextStyle {
    size: 12.0,
    weight: 400,
    tabular: true,
};
const UNIT_NAME: TextStyle = TextStyle {
    size: 13.0,
    weight: 500,
    tabular: false,
};
const UNIT_SYMBOL: TextStyle = TextStyle {
    size: 13.0,
    weight: 650,
    tabular: false,
};
const VALUE_SIZE: f32 = 26.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Units(Category),
    Currency,
}

struct CategoryInfo {
    name: &'static str,
    kind: Kind,
    icon: &'static str,
    default_value: &'static str,
    default_from: &'static str,
    default_to: &'static str,
}

const CATEGORIES: [CategoryInfo; 8] = [
    CategoryInfo {
        name: "Length",
        kind: Kind::Units(Category::Length),
        icon: "M3 8 H21 V16 H3 Z M7 8 V11 M11 8 V12.5 M15 8 V11 M19 8 V12.5",
        default_value: "1",
        default_from: "km",
        default_to: "mi",
    },
    CategoryInfo {
        name: "Mass",
        kind: Kind::Units(Category::Mass),
        icon: "M5 20 L7.5 9 H16.5 L19 20 Z M9.5 9 A2.5 2.5 0 0 1 14.5 9",
        default_value: "1",
        default_from: "kg",
        default_to: "lb",
    },
    CategoryInfo {
        name: "Temperature",
        kind: Kind::Units(Category::Temperature),
        icon: "M10 14 V5 A2 2 0 0 1 14 5 V14 A4 4 0 1 1 10 14 Z M12 10 V17",
        default_value: "72",
        default_from: "°F",
        default_to: "°C",
    },
    CategoryInfo {
        name: "Data",
        kind: Kind::Units(Category::DataSize),
        icon: "M5 6 A7 2.5 0 0 0 19 6 A7 2.5 0 0 0 5 6 V18 A7 2.5 0 0 0 19 18 V6 \
               M5 12 A7 2.5 0 0 0 19 12",
        default_value: "1",
        default_from: "GB",
        default_to: "MB",
    },
    CategoryInfo {
        name: "Volume",
        kind: Kind::Units(Category::Volume),
        icon: "M6 4 H18 L16 20 H8 Z M6.8 10 H17.2",
        default_value: "1",
        default_from: "l",
        default_to: "gal",
    },
    CategoryInfo {
        name: "Speed",
        kind: Kind::Units(Category::Speed),
        icon: "M4 17 A8 8 0 0 1 20 17 M12 17 L16 11",
        default_value: "100",
        default_from: "km/h",
        default_to: "mph",
    },
    CategoryInfo {
        name: "Time",
        kind: Kind::Units(Category::Time),
        icon: "M12 3 A9 9 0 1 1 11.99 3 M12 7 V12 L15.5 14",
        default_value: "90",
        default_from: "min",
        default_to: "h",
    },
    CategoryInfo {
        name: "Currency",
        kind: Kind::Currency,
        icon: "M3 7 H21 V17 H3 Z M14 12 A2 2 0 1 1 13.99 12 M6.5 10 V14 M17.5 10 V14",
        default_value: "100",
        default_from: "USD",
        default_to: "EUR",
    },
];

/// Currencies with ECB reference rates, most used first.
const CURRENCIES: &[(&str, &str)] = &[
    ("USD", "US Dollar"),
    ("EUR", "Euro"),
    ("GBP", "British Pound"),
    ("JPY", "Japanese Yen"),
    ("INR", "Indian Rupee"),
    ("CNY", "Chinese Yuan"),
    ("CAD", "Canadian Dollar"),
    ("AUD", "Australian Dollar"),
    ("CHF", "Swiss Franc"),
    ("HKD", "Hong Kong Dollar"),
    ("SGD", "Singapore Dollar"),
    ("KRW", "South Korean Won"),
    ("SEK", "Swedish Krona"),
    ("NOK", "Norwegian Krone"),
    ("DKK", "Danish Krone"),
    ("NZD", "New Zealand Dollar"),
    ("BRL", "Brazilian Real"),
    ("MXN", "Mexican Peso"),
    ("ZAR", "South African Rand"),
    ("PLN", "Polish Zloty"),
    ("TRY", "Turkish Lira"),
    ("CZK", "Czech Koruna"),
    ("HUF", "Hungarian Forint"),
    ("IDR", "Indonesian Rupiah"),
    ("THB", "Thai Baht"),
    ("PHP", "Philippine Peso"),
    ("MYR", "Malaysian Ringgit"),
    ("ILS", "Israeli Shekel"),
];

fn unit_name(symbol: &str) -> &'static str {
    match symbol {
        "mm" => "Millimeter",
        "cm" => "Centimeter",
        "m" => "Meter",
        "km" => "Kilometer",
        "in" => "Inch",
        "ft" => "Foot",
        "yd" => "Yard",
        "mi" => "Mile",
        "nmi" => "Nautical mile",
        "mg" => "Milligram",
        "g" => "Gram",
        "kg" => "Kilogram",
        "t" => "Tonne",
        "oz" => "Ounce",
        "lb" => "Pound",
        "st" => "Stone",
        "°C" => "Celsius",
        "°F" => "Fahrenheit",
        "K" => "Kelvin",
        "ml" => "Milliliter",
        "l" => "Liter",
        "m³" => "Cubic meter",
        "tsp" => "Teaspoon",
        "tbsp" => "Tablespoon",
        "fl oz" => "Fluid ounce",
        "cup" => "Cup",
        "gal" => "Gallon",
        "m/s" => "Meters per second",
        "km/h" => "Kilometers per hour",
        "mph" => "Miles per hour",
        "kn" => "Knot",
        "B" => "Byte",
        "KB" => "Kilobyte",
        "MB" => "Megabyte",
        "GB" => "Gigabyte",
        "TB" => "Terabyte",
        "KiB" => "Kibibyte",
        "MiB" => "Mebibyte",
        "GiB" => "Gibibyte",
        "bit" => "Bit",
        "ms" => "Millisecond",
        "s" => "Second",
        "min" => "Minute",
        "h" => "Hour",
        "d" => "Day",
        "wk" => "Week",
        _ => "",
    }
}

/// One selectable unit (or currency) of a category.
#[derive(Clone, Copy)]
struct Opt {
    symbol: &'static str,
    name: &'static str,
    unit: Option<&'static Unit>,
}

fn options(kind: Kind) -> Vec<Opt> {
    match kind {
        Kind::Units(category) => engine::units_in(category)
            .map(|unit| Opt {
                symbol: unit.symbol,
                name: unit_name(unit.symbol),
                unit: Some(unit),
            })
            .collect(),
        Kind::Currency => CURRENCIES
            .iter()
            .map(|(code, name)| Opt {
                symbol: code,
                name,
                unit: None,
            })
            .collect(),
    }
}

fn option_index(kind: Kind, symbol: &str) -> usize {
    options(kind)
        .iter()
        .position(|opt| opt.symbol == symbol)
        .unwrap_or(0)
}

fn convert(value: f64, from: &Opt, to: &Opt, rates: Option<&Rates>) -> Option<f64> {
    match (from.unit, to.unit) {
        (Some(from), Some(to)) => engine::convert(value, from, to).ok(),
        _ => rates?.convert(value, from.symbol, to.symbol),
    }
}

struct CategoryState {
    value: TextBuffer,
    from: usize,
    to: usize,
}

#[derive(Clone)]
struct Recent {
    category: usize,
    value: String,
    from: usize,
    to: usize,
    label: String,
}

struct State {
    category: usize,
    categories: Vec<CategoryState>,
    focused: bool,
    focus_since: Instant,
    last_drawn: Option<Instant>,
    copied_at: Option<Instant>,
    recent: Vec<Recent>,
    rates: Option<Rates>,
    /// When the rates arrived, and the local time to show.
    fetched: Option<(Instant, String)>,
    pending: Option<Receiver<Result<Rates, String>>>,
    want_rates: bool,
    retry_at: Option<Instant>,
    rates_failed: bool,
}

impl State {
    fn new() -> Self {
        let categories = CATEGORIES
            .iter()
            .map(|info| CategoryState {
                value: TextBuffer::with_text(info.default_value, MAX_VALUE_CHARS),
                from: option_index(info.kind, info.default_from),
                to: option_index(info.kind, info.default_to),
            })
            .collect();
        Self {
            category: 0,
            categories,
            focused: false,
            focus_since: Instant::now(),
            last_drawn: None,
            copied_at: None,
            recent: Vec::new(),
            rates: None,
            fetched: None,
            pending: None,
            want_rates: false,
            retry_at: None,
            rates_failed: false,
        }
    }

    fn kind(&self) -> Kind {
        CATEGORIES[self.category].kind
    }

    fn current(&self) -> &CategoryState {
        &self.categories[self.category]
    }

    fn current_mut(&mut self) -> &mut CategoryState {
        &mut self.categories[self.category]
    }

    fn input(&self) -> Option<f64> {
        parse_number(self.current().value.text())
    }

    /// The converted amount, when the input and (for currency) the rates allow it.
    fn result(&self) -> Option<f64> {
        let opts = options(self.kind());
        let state = self.current();
        convert(
            self.input()?,
            &opts[state.from],
            &opts[state.to],
            self.rates.as_ref(),
        )
    }

    /// Adds the shown conversion to the recent list (most recent first, no duplicates).
    fn record(&mut self) {
        let Some(input) = self.input() else {
            return;
        };
        let Some(result) = self.result() else {
            return;
        };
        let opts = options(self.kind());
        let state = self.current();
        let label = format!(
            "{} {} → {} {}",
            group(&format_number(input)),
            opts[state.from].symbol,
            group(&format_number(result)),
            opts[state.to].symbol
        );
        let entry = Recent {
            category: self.category,
            value: state.value.text().to_string(),
            from: state.from,
            to: state.to,
            label,
        };
        self.recent.retain(|recent| recent.label != entry.label);
        self.recent.insert(0, entry);
        self.recent.truncate(MAX_RECENT);
    }

    fn blur(&mut self) {
        if self.focused {
            self.focused = false;
            self.record();
        }
    }
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::new());
    static ICONS: RefCell<HashMap<&'static str, Option<Path>>> = RefCell::new(HashMap::new());
}

fn parse_number(text: &str) -> Option<f64> {
    let cleaned: String = text.chars().filter(|c| *c != ',' && *c != ' ').collect();
    if cleaned.is_empty() || cleaned == "-" || cleaned == "." {
        return None;
    }
    cleaned
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
}

/// A readable amount: more decimals for small numbers, scientific notation for extremes.
fn format_number(value: f64) -> String {
    if !value.is_finite() {
        return "—".to_string();
    }
    let magnitude = value.abs();
    if magnitude != 0.0 && !(1e-6..1e15).contains(&magnitude) {
        return format!("{value:.3e}");
    }
    let decimals = if magnitude >= 1000.0 {
        2
    } else if magnitude >= 1.0 {
        4
    } else {
        6
    };
    engine::format_value(value, decimals)
}

/// Adds thousands separators to the integer part of a plain number.
fn group(text: &str) -> String {
    if text.contains('e') {
        return text.to_string();
    }
    let (sign, rest) = match text.strip_prefix('-') {
        Some(rest) => ("-", rest),
        None => ("", text),
    };
    let (integer, fraction) = match rest.split_once('.') {
        Some((integer, fraction)) => (integer, Some(fraction)),
        None => (rest, None),
    };
    let mut grouped = String::new();
    for (index, character) in integer.chars().enumerate() {
        if index > 0 && (integer.len() - index) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(character);
    }
    match fraction {
        Some(fraction) => format!("{sign}{grouped}.{fraction}"),
        None => format!("{sign}{grouped}"),
    }
}

fn font_style(style: TextStyle) -> FontStyle {
    if style.weight >= 600 {
        FontStyle::bold()
    } else {
        FontStyle::normal()
    }
}

fn text_width(text: &str, style: TextStyle) -> f32 {
    FontManager::global().measure_text_cached(text, style.size, font_style(style))
}

fn value_style(size: f32) -> TextStyle {
    TextStyle {
        size,
        weight: 650,
        tabular: true,
    }
}

/// The largest value size (down to 14) at which `text` fits `width`.
fn fit_value_size(text: &str, width: f32) -> f32 {
    let mut size = VALUE_SIZE;
    while size > 14.0 && text_width(text, value_style(size)) > width {
        size -= 1.0;
    }
    size
}

fn draw_right(painter: Painter<'_>, text: &str, style: TextStyle, rect: Rect, color: Rgba) {
    let width = text_width(text, style).min(rect.width());
    draw_text(
        painter,
        text,
        style,
        Rect::from_ltrb(rect.right - width - 1.0, rect.top, rect.right, rect.bottom),
        Align::Start,
        color,
    );
}

/// Strokes a 24×24 SVG path icon centered in `rect`.
fn draw_svg_icon(painter: Painter<'_>, svg: &'static str, rect: Rect, color: Rgba, stroke: f32) {
    let size = rect.width().min(rect.height());
    if size <= 0.0 {
        return;
    }
    ICONS.with(|cache| {
        let mut cache = cache.borrow_mut();
        let Some(path) = cache
            .entry(svg)
            .or_insert_with(|| Path::from_svg(svg))
            .as_ref()
        else {
            return;
        };
        let scale = size / 24.0;
        let save = painter.save();
        painter.translate(Vec2::new(
            rect.center_x() - 12.0 * scale,
            rect.center_y() - 12.0 * scale,
        ));
        painter.scale(Vec2::new(scale, scale));
        painter.stroke_path(path, stroke, color, StrokeCap::Round, StrokeJoin::Round);
        painter.restore_to(save);
    });
}

const SWAP_ICON: &str = "M8 5 V19 M5 8 L8 5 L11 8 M16 19 V5 M13 16 L16 19 L19 16";
const COPY_ICON: &str = "M9 9 H19 V19 H9 Z M15 9 V5 H5 V15 H9";
const CHECK_ICON: &str = "M5 12.5 L10 17 L19 7";
const CHEVRON_ICON: &str = "M7 10 L12 15 L17 10";

/// Where everything sits for the current state.
struct Layout {
    side: Rect,
    main: Rect,
    rows: Vec<Rect>,
    from_label: Rect,
    from_field: Rect,
    from_unit: Rect,
    divider_y: f32,
    swap: Rect,
    to_label: Rect,
    to_field: Rect,
    to_unit: Rect,
    copy: Option<Rect>,
    info: Rect,
    related: Vec<(Rect, usize)>,
    recent_label: Rect,
    recent: Vec<(Rect, usize)>,
    from_text: String,
    from_size: f32,
    to_text: String,
    to_size: f32,
}

fn unit_pill_width(opt: &Opt) -> f32 {
    let name = if opt.name.is_empty() {
        0.0
    } else {
        // Room for the gaps `draw_unit_pill` leaves around the name, plus a little slack so
        // rounding never ellipsizes it.
        text_width(opt.name, UNIT_NAME) + 10.0
    };
    name + text_width(opt.symbol, UNIT_SYMBOL) + 4.0 + 14.0 + 24.0
}

/// Up to three other options to show, preferring those giving readable magnitudes.
fn related_options(state: &State) -> Vec<usize> {
    let kind = state.kind();
    let opts = options(kind);
    let current = state.current();
    let candidates = (0..opts.len()).filter(|index| *index != current.from && *index != current.to);
    match kind {
        Kind::Currency => candidates.take(3).collect(),
        Kind::Units(_) => {
            let input = state.input().unwrap_or(1.0);
            let mut scored: Vec<(f64, usize)> = candidates
                .map(|index| {
                    let value = convert(input, &opts[current.from], &opts[index], None)
                        .map_or(f64::INFINITY, f64::abs);
                    let score = if value == 0.0 || !value.is_finite() {
                        100.0
                    } else {
                        (value.log10() - 1.0).abs()
                    };
                    (score, index)
                })
                .collect();
            scored.sort_by(|a, b| a.0.total_cmp(&b.0));
            let mut picked: Vec<usize> = scored.into_iter().take(3).map(|(_, i)| i).collect();
            picked.sort_unstable();
            picked
        }
    }
}

fn compute_layout(grid: Rect, state: &State) -> Layout {
    let side_width = (grid.width() * 0.24).clamp(140.0, 180.0);
    let side = Rect::from_xywh(grid.left, grid.top, side_width, grid.height());
    let main = Rect::from_ltrb(side.right + SIDE_GAP, grid.top, grid.right, grid.bottom);

    let inner = side.inset(6.0);
    let row_height = (inner.height() / CATEGORIES.len() as f32).clamp(18.0, 30.0);
    let rows_top =
        inner.top + (inner.height() - row_height * CATEGORIES.len() as f32).max(0.0) / 2.0;
    let rows = (0..CATEGORIES.len())
        .map(|index| {
            Rect::from_xywh(
                inner.left,
                rows_top + index as f32 * row_height,
                inner.width(),
                row_height,
            )
        })
        .collect();

    let x0 = main.left + PAD;
    let x1 = main.right - PAD;
    let top = main.top + 10.0;
    let opts = options(state.kind());
    let current = state.current();

    let from_label = Rect::from_ltrb(x0, top, x1, top + 14.0);
    let from_row = Rect::from_ltrb(x0, top + 14.0, x1, top + 46.0);
    let from_pill = unit_pill_width(&opts[current.from]);
    let from_unit = Rect::from_xywh(
        x1 + 6.0 - from_pill,
        from_row.center_y() - 14.0,
        from_pill,
        28.0,
    );
    let from_field = Rect::from_ltrb(
        x0 - 8.0,
        from_row.top,
        from_unit.left - 12.0,
        from_row.bottom,
    );
    let from_text = current.value.text().to_string();
    let from_size = fit_value_size(&from_text, from_field.width() - 20.0);

    let divider_y = top + 54.0;
    let swap_x = x0 + (main.width() * 0.3).min(150.0);
    let swap = Rect::from_xywh(swap_x - 14.0, divider_y - 14.0, 28.0, 28.0);

    let to_label = Rect::from_ltrb(x0, top + 60.0, swap.left - 8.0, top + 74.0);
    let to_row = Rect::from_ltrb(x0, top + 74.0, x1, top + 106.0);
    let to_pill = unit_pill_width(&opts[current.to]);
    let to_unit = Rect::from_xywh(x1 + 6.0 - to_pill, to_row.center_y() - 14.0, to_pill, 28.0);
    let to_text = match state.result() {
        Some(result) => group(&format_number(result)),
        None => "—".to_string(),
    };
    let to_space = to_unit.left - 12.0 - x0 - 34.0;
    let to_size = fit_value_size(&to_text, to_space);
    let to_width = text_width(&to_text, value_style(to_size)).min(to_space);
    let to_field = Rect::from_ltrb(x0, to_row.top, x0 + to_width, to_row.bottom);
    let copy = state
        .result()
        .map(|_| Rect::from_xywh(to_field.right + 6.0, to_row.center_y() - 13.0, 26.0, 26.0));

    let info = Rect::from_ltrb(x0, top + 108.0, x1, top + 122.0);
    let related = related_options(state)
        .into_iter()
        .enumerate()
        .map(|(slot, index)| {
            (
                Rect::from_xywh(x0, top + 126.0 + slot as f32 * 19.0, x1 - x0, 19.0),
                index,
            )
        })
        .collect();

    let recent_top = top + 186.0;
    let recent_label = Rect::from_xywh(x0, recent_top, text_width("Recent", LABEL) + 2.0, 16.0);
    let mut recent = Vec::new();
    let mut x = recent_label.right + 10.0;
    for (index, entry) in state.recent.iter().enumerate().take(MAX_RECENT) {
        let width = text_width(&entry.label, SMALL) + 12.0;
        if x + width > x1 + 6.0 {
            break;
        }
        recent.push((Rect::from_xywh(x, recent_top - 2.0, width, 20.0), index));
        x += width + 10.0;
    }

    Layout {
        side,
        main,
        rows,
        from_label,
        from_field,
        from_unit,
        divider_y,
        swap,
        to_label,
        to_field,
        to_unit,
        copy,
        info,
        related,
        recent_label,
        recent,
        from_text,
        from_size,
        to_text,
        to_size,
    }
}

/// Short month name for a provider date such as `2026-10-30`.
fn short_date(date: &str) -> String {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let mut parts = date.split('-');
    let (_, month, day) = (parts.next(), parts.next(), parts.next());
    match (
        month.and_then(|m| m.parse::<usize>().ok()),
        day.and_then(|d| d.parse::<u32>().ok()),
    ) {
        (Some(month @ 1..=12), Some(day)) => format!("{day} {}", MONTHS[month - 1]),
        _ => date.to_string(),
    }
}

fn local_time_label() -> String {
    let now = crate::platform::shell().local_datetime();
    let hour = now.hour % 12;
    let hour = if hour == 0 { 12 } else { hour };
    let suffix = if now.hour < 12 { "AM" } else { "PM" };
    format!("{hour}:{:02} {suffix}", now.minute)
}

/// Draws the tab. Return `false` to fall back to the legacy page.
pub fn draw(ctx: &TabContext<'_>) -> bool {
    STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        state.last_drawn = Some(ctx.now);
        if state.kind() == Kind::Currency {
            state.want_rates = true;
        }
        if state
            .copied_at
            .is_some_and(|at| ctx.now.duration_since(at) > COPIED_FOR)
        {
            state.copied_at = None;
        }
        draw_state(ctx, &state);
    });
    true
}

fn hovered(ctx: &TabContext<'_>, id: u16) -> bool {
    ctx.hover == Some(DashboardHit::Tab(id))
}

fn draw_state(ctx: &TabContext<'_>, state: &State) {
    let painter = ctx.painter;
    let colors = ctx.colors;
    let layout = compute_layout(ctx.grid, state);
    let save = painter.save();
    painter.clip_rect(ctx.grid);

    let card_radius = Radius::uniform(radius::CARD + 6.0);
    painter.fill_round_rect(layout.side, card_radius, colors.surface_raised);
    painter.fill_round_rect(layout.main, card_radius, colors.surface_raised);

    // Categories.
    for (index, (row, info)) in layout.rows.iter().zip(CATEGORIES.iter()).enumerate() {
        let selected = index == state.category;
        let hover = hovered(ctx, ID_CATEGORY + index as u16);
        if selected {
            painter.fill_round_rect(
                *row,
                Radius::uniform(radius::ROW),
                colors.surface_interactive,
            );
        } else if hover {
            painter.fill_round_rect(*row, Radius::uniform(radius::ROW), colors.surface_overlay);
        }
        let color = if selected || hover {
            colors.text_primary
        } else {
            colors.text_secondary
        };
        let icon = Rect::from_xywh(row.left + 10.0, row.center_y() - 8.0, 16.0, 16.0);
        draw_svg_icon(painter, info.icon, icon, color, 1.8);
        draw_text(
            painter,
            info.name,
            if selected { ROW_BOLD } else { ROW },
            Rect::from_ltrb(icon.right + 10.0, row.top, row.right - 6.0, row.bottom),
            Align::Start,
            color,
        );
    }

    let opts = options(state.kind());
    let current = state.current();

    // From.
    draw_text(
        painter,
        "From",
        LABEL,
        layout.from_label,
        Align::Start,
        colors.text_secondary,
    );
    let field_hover = hovered(ctx, ID_FROM_FIELD);
    if state.focused || field_hover {
        painter.fill_round_rect(
            layout.from_field,
            Radius::uniform(radius::ROW),
            if state.focused {
                colors.surface_overlay
            } else {
                colors.surface_overlay.with_alpha_f(0.6)
            },
        );
    }
    if state.focused {
        painter.stroke_round_rect(
            layout.from_field,
            Radius::uniform(radius::ROW),
            1.0,
            colors.accent_primary,
        );
    }
    let text_left = layout.from_field.left + 8.0;
    let value_rect = Rect::from_ltrb(
        text_left,
        layout.from_field.top,
        layout.from_field.right - 8.0,
        layout.from_field.bottom,
    );
    if layout.from_text.is_empty() {
        draw_text(
            painter,
            "0",
            value_style(VALUE_SIZE),
            value_rect,
            Align::Start,
            colors.text_disabled,
        );
    } else {
        draw_text(
            painter,
            &layout.from_text,
            value_style(layout.from_size),
            value_rect,
            Align::Start,
            colors.text_primary,
        );
    }
    if state.focused {
        let blink = ctx.now.duration_since(state.focus_since).as_millis() / 530 % 2 == 0;
        if blink {
            let caret_x = (text_left
                + text_width(current.value.before_caret(), value_style(layout.from_size))
                + 1.0)
                .min(value_rect.right);
            let half = layout.from_size * 0.55;
            painter.stroke_line(
                Point::new(caret_x, layout.from_field.center_y() - half),
                Point::new(caret_x, layout.from_field.center_y() + half),
                1.5,
                colors.accent_primary,
                StrokeCap::Round,
            );
        }
    }
    draw_unit_pill(
        ctx,
        &opts[current.from],
        layout.from_unit,
        hovered(ctx, ID_FROM_UNIT),
    );

    // Divider and swap.
    painter.stroke_line(
        Point::new(layout.main.left + PAD, layout.divider_y),
        Point::new(layout.main.right - PAD, layout.divider_y),
        1.0,
        colors.border_subtle,
        StrokeCap::Butt,
    );
    let swap_center = Point::new(layout.swap.center_x(), layout.swap.center_y());
    painter.fill_circle(
        swap_center,
        14.0,
        if hovered(ctx, ID_SWAP) {
            colors.surface_interactive
        } else {
            colors.surface_overlay
        },
    );
    painter.stroke_circle(swap_center, 14.0, 1.0, colors.border_subtle);
    draw_svg_icon(
        painter,
        SWAP_ICON,
        layout.swap.inset(7.0),
        colors.text_primary,
        2.0,
    );

    // To.
    draw_text(
        painter,
        "To",
        LABEL,
        layout.to_label,
        Align::Start,
        colors.text_secondary,
    );
    draw_text(
        painter,
        &layout.to_text,
        value_style(layout.to_size),
        Rect::from_ltrb(
            layout.to_field.left,
            layout.to_field.top,
            layout.to_field.right + 2.0,
            layout.to_field.bottom,
        ),
        Align::Start,
        if state.result().is_some() {
            colors.text_primary
        } else {
            colors.text_disabled
        },
    );
    if let Some(copy) = layout.copy {
        let hover = hovered(ctx, ID_COPY);
        if hover {
            painter.fill_round_rect(
                copy,
                Radius::uniform(radius::CONTROL),
                colors.surface_overlay,
            );
        }
        if state.copied_at.is_some() {
            draw_svg_icon(
                painter,
                CHECK_ICON,
                copy.inset(6.0),
                colors.state_success,
                2.0,
            );
        } else {
            draw_svg_icon(
                painter,
                COPY_ICON,
                copy.inset(6.5),
                if hover {
                    colors.text_primary
                } else {
                    colors.text_secondary
                },
                1.7,
            );
        }
    }
    draw_unit_pill(
        ctx,
        &opts[current.to],
        layout.to_unit,
        hovered(ctx, ID_TO_UNIT),
    );

    // Info line: rate source for currency, the unit ratio otherwise.
    let info = match state.kind() {
        Kind::Currency => match (&state.rates, &state.fetched) {
            (Some(rates), Some((_, fetched))) => format!(
                "ECB rates from {} · updated {fetched}",
                short_date(&rates.date)
            ),
            (Some(rates), None) => format!("ECB rates from {}", short_date(&rates.date)),
            _ if state.rates_failed => "Rates unavailable, retrying shortly".to_string(),
            _ if crate::core::safe_mode::active() => "Rates are off in safe mode".to_string(),
            _ => "Fetching exchange rates…".to_string(),
        },
        Kind::Units(_) => {
            let (from, to) = (&opts[current.from], &opts[current.to]);
            match convert(1.0, from, to, None) {
                Some(ratio) => format!(
                    "1 {} = {} {}",
                    from.symbol,
                    group(&format_number(ratio)),
                    to.symbol
                ),
                None => String::new(),
            }
        }
    };
    draw_text(
        painter,
        &info,
        SMALL,
        layout.info,
        Align::Start,
        colors.text_secondary,
    );

    // Related conversions.
    let input = state.input();
    for (slot, (row, index)) in layout.related.iter().enumerate() {
        let opt = &opts[*index];
        let hover = hovered(ctx, ID_RELATED + slot as u16);
        if hover {
            painter.fill_round_rect(
                Rect::from_ltrb(row.left - 6.0, row.top, row.right + 6.0, row.bottom),
                Radius::uniform(radius::CONTROL),
                colors.surface_overlay,
            );
        }
        painter.stroke_line(
            Point::new(row.left, row.top),
            Point::new(row.right, row.top),
            1.0,
            colors.border_subtle,
            StrokeCap::Butt,
        );
        let name = if opt.name.is_empty() {
            opt.symbol
        } else {
            opt.name
        };
        draw_text(
            painter,
            name,
            ROW,
            Rect::from_ltrb(row.left, row.top, row.center_x(), row.bottom),
            Align::Start,
            colors.text_primary,
        );
        let symbol_width = text_width(opt.symbol, SMALL).max(28.0);
        draw_text(
            painter,
            opt.symbol,
            SMALL,
            Rect::from_ltrb(row.right - symbol_width, row.top, row.right, row.bottom),
            Align::Start,
            colors.text_secondary,
        );
        let value = input
            .and_then(|input| convert(input, &opts[current.from], opt, state.rates.as_ref()))
            .map_or_else(|| "—".to_string(), |value| group(&format_number(value)));
        draw_right(
            painter,
            &value,
            TextStyle {
                size: 13.0,
                weight: 500,
                tabular: true,
            },
            Rect::from_ltrb(
                row.center_x(),
                row.top,
                row.right - symbol_width - 6.0,
                row.bottom,
            ),
            colors.text_primary,
        );
    }

    // Recent.
    draw_text(
        painter,
        "Recent",
        LABEL,
        layout.recent_label,
        Align::Start,
        colors.text_secondary,
    );
    if layout.recent.is_empty() {
        draw_text(
            painter,
            "Press Enter or copy a result to keep it here",
            SMALL,
            Rect::from_ltrb(
                layout.recent_label.right + 10.0,
                layout.recent_label.top,
                layout.main.right - PAD,
                layout.recent_label.bottom,
            ),
            Align::Start,
            colors.text_disabled,
        );
    }
    for (slot, (rect, index)) in layout.recent.iter().enumerate() {
        let hover = hovered(ctx, ID_RECENT + slot as u16);
        if hover {
            painter.fill_round_rect(
                *rect,
                Radius::uniform(radius::CONTROL),
                colors.surface_overlay,
            );
        }
        draw_text(
            painter,
            &state.recent[*index].label,
            SMALL,
            *rect,
            Align::Center,
            if hover {
                colors.text_primary
            } else {
                colors.text_secondary
            },
        );
    }

    painter.restore_to(save);
}

fn draw_unit_pill(ctx: &TabContext<'_>, opt: &Opt, rect: Rect, hover: bool) {
    let painter = ctx.painter;
    let colors: &SemanticColors = ctx.colors;
    if hover {
        painter.fill_round_rect(rect, Radius::uniform(radius::ROW), colors.surface_overlay);
    }
    let chevron = Rect::from_xywh(rect.right - 10.0 - 14.0, rect.center_y() - 7.0, 14.0, 14.0);
    draw_svg_icon(painter, CHEVRON_ICON, chevron, colors.text_secondary, 2.0);
    let symbol_width = text_width(opt.symbol, UNIT_SYMBOL) + 1.0;
    let symbol_rect = Rect::from_ltrb(
        chevron.left - 4.0 - symbol_width,
        rect.top,
        chevron.left - 4.0,
        rect.bottom,
    );
    draw_text(
        painter,
        opt.symbol,
        UNIT_SYMBOL,
        symbol_rect,
        Align::Start,
        colors.text_primary,
    );
    if !opt.name.is_empty() {
        draw_text(
            painter,
            opt.name,
            UNIT_NAME,
            Rect::from_ltrb(
                rect.left + 10.0,
                rect.top,
                symbol_rect.left - 6.0,
                rect.bottom,
            ),
            Align::Start,
            colors.text_secondary,
        );
    }
}

/// `true` when this tab draws its own cards instead of sitting on the shared page card.
pub fn custom_layout() -> bool {
    true
}

/// Hit test in page coordinates. Use `DashboardHit::Tab(id)` with ids private to this tab.
pub fn hit(grid: Rect, point: Point) -> Option<DashboardHit> {
    if !grid.contains(point) {
        return None;
    }
    let id = STATE.with(|cell| {
        let state = cell.borrow();
        let layout = compute_layout(grid, &state);
        if let Some(index) = layout.rows.iter().position(|row| row.contains(point)) {
            return ID_CATEGORY + index as u16;
        }
        let swap_center = Point::new(layout.swap.center_x(), layout.swap.center_y());
        let (dx, dy) = (point.x - swap_center.x, point.y - swap_center.y);
        if dx * dx + dy * dy <= 16.0 * 16.0 {
            return ID_SWAP;
        }
        if layout.from_unit.contains(point) {
            return ID_FROM_UNIT;
        }
        if layout.to_unit.contains(point) {
            return ID_TO_UNIT;
        }
        if layout.copy.is_some_and(|copy| copy.contains(point)) {
            return ID_COPY;
        }
        if layout.from_field.contains(point) {
            return ID_FROM_FIELD;
        }
        if let Some(slot) = layout
            .related
            .iter()
            .position(|(row, _)| row.contains(point))
        {
            return ID_RELATED + slot as u16;
        }
        if let Some(slot) = layout
            .recent
            .iter()
            .position(|(rect, _)| rect.contains(point))
        {
            return ID_RECENT + slot as u16;
        }
        ID_BACKGROUND
    });
    Some(DashboardHit::Tab(id))
}

/// Next option after `index`, skipping `avoid` when there is another choice.
fn cycle(index: usize, avoid: usize, count: usize) -> usize {
    let mut next = (index + 1) % count;
    if next == avoid && count > 2 {
        next = (next + 1) % count;
    }
    next
}

/// Applies a click from [`hit`]. `true` when handled.
pub fn apply(hit: DashboardHit) -> bool {
    let DashboardHit::Tab(id @ 800..=899) = hit else {
        // Anything outside the tab (header, dock) ends editing.
        STATE.with(|cell| cell.borrow_mut().blur());
        return false;
    };
    STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        let count = options(state.kind()).len();
        match id {
            ID_FROM_FIELD => {
                if !state.focused {
                    state.focused = true;
                    state.focus_since = Instant::now();
                    let end = state.current().value.char_count();
                    state.current_mut().value.set_caret(end);
                }
            }
            ID_FROM_UNIT => {
                let current = state.current();
                let next = cycle(current.from, current.to, count);
                state.current_mut().from = next;
            }
            ID_TO_UNIT => {
                let current = state.current();
                let next = cycle(current.to, current.from, count);
                state.current_mut().to = next;
            }
            ID_SWAP => {
                state.record();
                let current = state.current_mut();
                std::mem::swap(&mut current.from, &mut current.to);
            }
            ID_COPY => {
                if let Some(result) = state.result() {
                    let text = format_number(result);
                    match winisland_platform_windows::set_clipboard_text(&text) {
                        Ok(()) => {
                            state.copied_at = Some(Instant::now());
                            // Redraw once the check mark should turn back into the copy icon.
                            let _ = std::thread::Builder::new()
                                .name("units-copied".into())
                                .spawn(|| {
                                    std::thread::sleep(COPIED_FOR + Duration::from_millis(50));
                                    crate::platform::wake();
                                });
                        }
                        Err(error) => log::warn!("Cannot copy conversion: {error}"),
                    }
                    state.record();
                }
            }
            ID_BACKGROUND => state.blur(),
            _ if (ID_CATEGORY..ID_CATEGORY + CATEGORIES.len() as u16).contains(&id) => {
                let category = usize::from(id - ID_CATEGORY);
                if category != state.category {
                    state.record();
                    state.category = category;
                    state.copied_at = None;
                }
            }
            _ if (ID_RELATED..ID_RELATED + 3).contains(&id) => {
                let slot = usize::from(id - ID_RELATED);
                if let Some(index) = related_options(&state).get(slot).copied() {
                    state.current_mut().to = index;
                }
            }
            _ if (ID_RECENT..ID_RECENT + MAX_RECENT as u16).contains(&id) => {
                let slot = usize::from(id - ID_RECENT);
                if let Some(entry) = state.recent.get(slot).cloned() {
                    state.category = entry.category;
                    let target = &mut state.categories[entry.category];
                    target.value = TextBuffer::with_text(&entry.value, MAX_VALUE_CHARS);
                    target.from = entry.from;
                    target.to = entry.to;
                }
            }
            _ => {}
        }
    });
    true
}

pub fn wants_keyboard() -> bool {
    STATE.with(|cell| cell.borrow().focused)
}

pub fn handle_key(key: &Key) -> bool {
    STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        if !state.focused {
            return false;
        }
        state.focus_since = Instant::now();
        match key {
            Key::Escape => state.blur(),
            Key::Enter => state.record(),
            Key::Backspace => {
                state.current_mut().value.apply(EditKey::Backspace);
            }
            Key::Delete => {
                state.current_mut().value.apply(EditKey::Delete);
            }
            Key::ArrowLeft => {
                state.current_mut().value.apply(EditKey::Left);
            }
            Key::ArrowRight => {
                state.current_mut().value.apply(EditKey::Right);
            }
            Key::Home => {
                state.current_mut().value.apply(EditKey::Home);
            }
            Key::End => {
                state.current_mut().value.apply(EditKey::End);
            }
            Key::ArrowUp | Key::ArrowDown => {
                let count = CATEGORIES.len();
                state.record();
                state.category = if matches!(key, Key::ArrowUp) {
                    (state.category + count - 1) % count
                } else {
                    (state.category + 1) % count
                };
                let end = state.current().value.char_count();
                state.current_mut().value.set_caret(end);
            }
            Key::Tab => {
                let current = state.current_mut();
                std::mem::swap(&mut current.from, &mut current.to);
            }
            Key::Character(text) => {
                let accepted: String = text
                    .chars()
                    .filter(|c| c.is_ascii_digit() || matches!(c, '.' | ',' | '-'))
                    .collect();
                if !accepted.is_empty() {
                    state.current_mut().value.insert(&accepted);
                }
            }
            Key::Other => {}
        }
        true
    })
}

/// Every frame, visible or not.
pub fn poll(now: Instant) {
    STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        // Stop editing once the tab is no longer on screen.
        if state.focused
            && state.last_drawn.is_none_or(|drawn| {
                now.saturating_duration_since(drawn) > Duration::from_millis(500)
            })
        {
            state.blur();
        }
        poll_rates(&mut state, now);
    });
}

fn poll_rates(state: &mut State, now: Instant) {
    if let Some(receiver) = &state.pending {
        match receiver.try_recv() {
            Ok(Ok(rates)) => {
                crate::ui::dashboard::palette::set_rates(rates.clone());
                state.rates = Some(rates);
                state.fetched = Some((now, local_time_label()));
                state.rates_failed = false;
                state.pending = None;
            }
            Ok(Err(error)) => {
                log::warn!("Currency rates unavailable: {error}");
                state.rates_failed = true;
                state.retry_at = Some(now + RETRY_AFTER);
                state.pending = None;
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                state.rates_failed = true;
                state.retry_at = Some(now + RETRY_AFTER);
                state.pending = None;
            }
        }
        return;
    }
    if !std::mem::take(&mut state.want_rates) {
        return;
    }
    let stale = state
        .fetched
        .as_ref()
        .is_none_or(|(at, _)| now.saturating_duration_since(*at) >= MAX_AGE);
    let waiting = state.retry_at.is_some_and(|at| now < at);
    if !stale || waiting || crate::core::safe_mode::active() {
        return;
    }
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        return;
    };
    let (sender, receiver) = mpsc::channel();
    handle.spawn(async move {
        let _ = sender.send(winisland_core::currency::fetch_rates().await);
        crate::platform::wake();
    });
    state.pending = Some(receiver);
}
