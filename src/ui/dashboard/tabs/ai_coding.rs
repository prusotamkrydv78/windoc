//! AI coding tab: today's tokens, estimated API value, prompts, tool calls and sessions;
//! this month's estimated cost; active hours; and usage by model.
//!
//! The tab reads Claude Code and Codex logs itself (see [`scan`]), only while the user's
//! AI usage setting is on (as published to the dashboard by `sync_ai_usage`) and the tab
//! is open, because the app's service keeps only per-day totals.

mod scan;

use std::cell::{Cell, RefCell};
use std::time::{Duration, Instant};

use winisland_core::ai_usage::{Tool, format_tokens};
use winisland_platform::Key;
use winisland_render::text::FontManager;
use winisland_render::tokens::{SemanticColors, TextStyle, radius, typography};
use winisland_render::{FontStyle, Painter, Point, Radius, Rect, Rgba, StrokeCap};

use super::TabContext;
use crate::ui::components::icon::{Icon, draw_icon};
use crate::ui::components::text::{Align, draw_text};
use crate::ui::dashboard::{AI_USAGE, DashboardHit, Destination};

const PAD: f32 = 14.0;
const GAP: f32 = 8.0;
const MODEL_ROW: f32 = 42.0;
const CHIP_WIDTH: f32 = 116.0;
const RESCAN: Duration = Duration::from_secs(60);
const CLAUDE_ORANGE: Rgba = Rgba::from_rgb(217, 119, 87);

const BIG: TextStyle = TextStyle {
    size: 30.0,
    weight: 700,
    tabular: true,
};
const HERO: TextStyle = TextStyle {
    size: 26.0,
    weight: 650,
    tabular: true,
};
const STAT: TextStyle = TextStyle {
    size: 22.0,
    weight: 650,
    tabular: true,
};
const NAME: TextStyle = TextStyle {
    size: 13.0,
    weight: 500,
    tabular: false,
};
const FIGURE: TextStyle = TextStyle {
    size: 12.0,
    weight: 650,
    tabular: true,
};
const BODY_BOLD: TextStyle = TextStyle {
    size: 14.0,
    weight: 650,
    tabular: true,
};

// Ids for `DashboardHit::Tab`: 1200 tool chip, 1210..1233 hour bars, 1240..1259 model rows.
const ID_CHIP: u16 = 1200;
const ID_HOUR: u16 = 1210;
const ID_MODEL: u16 = 1240;
const MAX_MODEL_ROWS: usize = 20;

#[derive(Clone, Copy, PartialEq, Eq)]
enum PageState {
    Off,
    SafeMode,
    Loading,
    Empty,
    Data,
}

thread_local! {
    /// `None` shows every tool.
    static FILTER: Cell<Option<Tool>> = const { Cell::new(None) };
    static LAST_SCAN: Cell<Option<Instant>> = const { Cell::new(None) };
    /// What the last draw showed, for hit testing.
    static STATE: Cell<PageState> = const { Cell::new(PageState::Loading) };
    static MODEL_ROWS: Cell<usize> = const { Cell::new(0) };
    static TOOLS: RefCell<Vec<Tool>> = const { RefCell::new(Vec::new()) };
}

/// Whether the user turned AI usage on (and safe mode is off).
fn enabled() -> bool {
    AI_USAGE.with(|cell| matches!(cell.borrow().key, Some((true, _))))
}

fn page_state() -> PageState {
    if !enabled() {
        PageState::Off
    } else if crate::core::safe_mode::active() {
        PageState::SafeMode
    } else {
        match scan::snapshot() {
            None => PageState::Loading,
            Some(snapshot) if snapshot.days.values().all(|day| !day.is_active()) => {
                PageState::Empty
            }
            Some(_) => PageState::Data,
        }
    }
}

struct Cards {
    today: Rect,
    hours: Rect,
    month: Rect,
    models: Rect,
}

fn cards(grid: Rect) -> Cards {
    let split = grid.left + (grid.width() * 0.57).round();
    let top_height = (grid.height() * 0.53).clamp(112.0, 132.0);
    let month_height = (grid.height() * 0.39).clamp(80.0, 98.0);
    let today = Rect::from_ltrb(grid.left, grid.top, split - GAP, grid.top + top_height);
    let month = Rect::from_ltrb(split, grid.top, grid.right, grid.top + month_height);
    Cards {
        today,
        hours: Rect::from_ltrb(grid.left, today.bottom + GAP, split - GAP, grid.bottom),
        month,
        models: Rect::from_ltrb(split, month.bottom + GAP, grid.right, grid.bottom),
    }
}

fn chip_rect(card: Rect) -> Rect {
    Rect::from_xywh(
        card.right - PAD - CHIP_WIDTH + 6.0,
        card.top + 8.0,
        CHIP_WIDTH,
        20.0,
    )
}

fn hour_columns(card: Rect) -> Vec<Rect> {
    let inner = Rect::from_ltrb(
        card.left + PAD,
        card.top + 30.0,
        card.right - PAD,
        card.bottom - 4.0,
    );
    let slot = inner.width() / 24.0;
    (0..24)
        .map(|hour| {
            Rect::from_xywh(
                inner.left + slot * hour as f32,
                inner.top,
                slot,
                inner.height(),
            )
        })
        .collect()
}

fn model_row(card: Rect, index: usize) -> Rect {
    Rect::from_xywh(
        card.left + 6.0,
        card.top + 32.0 + index as f32 * MODEL_ROW,
        card.width() - 12.0,
        MODEL_ROW - 2.0,
    )
}

fn fitting_model_rows(card: Rect) -> usize {
    (((card.bottom - 4.0 - (card.top + 32.0)) / MODEL_ROW)
        .floor()
        .max(0.0) as usize)
        .min(MAX_MODEL_ROWS)
}

fn settings_button(grid: Rect) -> Rect {
    Rect::from_xywh(grid.center_x() - 66.0, grid.center_y() + 40.0, 132.0, 30.0)
}

// ---------------------------------------------------------------------------------------
// Figures

#[derive(Default)]
struct View {
    tokens: u64,
    cost: f64,
    messages: u32,
    tool_calls: u32,
    sessions: usize,
    hours: [u64; 24],
    project: Option<String>,
    streak: usize,
    month_cost: f64,
    month_tokens: u64,
    /// `(model, tokens, cost)`, largest first.
    models: Vec<(String, u64, f64)>,
    models_scope: &'static str,
    /// Tools with any activity, Claude Code first.
    tools: Vec<Tool>,
}

fn build_view(snapshot: &scan::Snapshot, filter: Option<Tool>, today: i64) -> View {
    use std::collections::{BTreeMap, BTreeSet};
    let (year, month, _) = scan::civil_from_days(today);
    let month_start = scan::days_from_civil(year, month, 1);
    let mut view = View::default();
    let mut sessions = BTreeSet::new();
    let mut active_days = BTreeSet::new();
    let mut today_models: BTreeMap<&str, (u64, f64)> = BTreeMap::new();
    let mut month_models: BTreeMap<&str, (u64, f64)> = BTreeMap::new();
    let mut project: Option<(i64, &str)> = None;
    let mut latest_project: Option<(i64, &str)> = None;
    let mut tools = BTreeSet::new();
    for ((day, tool), stats) in &snapshot.days {
        if stats.is_active() {
            tools.insert(*tool);
        }
        if filter.is_some_and(|filter| filter != *tool) || !stats.is_active() {
            continue;
        }
        active_days.insert(*day);
        if let Some((at, name)) = &stats.project
            && latest_project.is_none_or(|(seen, _)| seen <= *at)
        {
            latest_project = Some((*at, name.as_str()));
        }
        if *day == today {
            view.tokens += stats.tokens;
            view.cost += stats.cost;
            view.messages += stats.messages;
            view.tool_calls += stats.tool_calls;
            sessions.extend(stats.sessions.iter());
            for (hour, tokens) in stats.hours.iter().enumerate() {
                view.hours[hour] += tokens;
            }
            for (model, (tokens, cost)) in &stats.models {
                let entry = today_models.entry(model.as_str()).or_default();
                entry.0 += tokens;
                entry.1 += cost;
            }
            if let Some((at, name)) = &stats.project
                && project.is_none_or(|(seen, _)| seen <= *at)
            {
                project = Some((*at, name.as_str()));
            }
        }
        if (month_start..=today).contains(day) {
            view.month_cost += stats.cost;
            view.month_tokens += stats.tokens;
            for (model, (tokens, cost)) in &stats.models {
                let entry = month_models.entry(model.as_str()).or_default();
                entry.0 += tokens;
                entry.1 += cost;
            }
        }
    }
    view.sessions = sessions.len();
    view.project = project.or(latest_project).map(|(_, name)| name.to_string());
    let mut day = if active_days.contains(&today) {
        today
    } else {
        today - 1
    };
    while active_days.contains(&day) {
        view.streak += 1;
        day -= 1;
    }
    let (models, scope) = if today_models.is_empty() {
        (month_models, "This month")
    } else {
        (today_models, "Today")
    };
    view.models = models
        .into_iter()
        .filter(|(_, (tokens, _))| *tokens > 0)
        .map(|(model, (tokens, cost))| (model.to_string(), tokens, cost))
        .collect();
    view.models.sort_by(|a, b| b.1.cmp(&a.1));
    view.models_scope = scope;
    view.tools = tools.into_iter().collect();
    view
}

/// `Opus 4.1`, `Sonnet 3.5`, `GPT-5 Codex`; anything else as written.
fn model_label(model: &str) -> String {
    let lower = model.to_ascii_lowercase();
    let parts: Vec<&str> = lower
        .split(['-', '_', '.', '[', ']'])
        .filter(|part| !part.is_empty())
        .collect();
    for family in ["opus", "sonnet", "haiku"] {
        if parts.contains(&family) {
            let version: Vec<&str> = parts
                .iter()
                .copied()
                .filter(|part| part.len() <= 2 && part.chars().all(|c| c.is_ascii_digit()))
                .collect();
            let name = capitalize(family);
            return if version.is_empty() {
                name
            } else {
                format!("{name} {}", version.join("."))
            };
        }
    }
    if let Some(rest) = lower.strip_prefix("gpt-") {
        let mut words = rest.split('-').filter(|word| !word.is_empty());
        let mut label = format!("GPT-{}", words.next().unwrap_or_default());
        for word in words {
            label.push(' ');
            label.push_str(&capitalize(word));
        }
        return label;
    }
    if model == "codex" {
        return "Codex".to_string();
    }
    model.to_string()
}

fn capitalize(word: &str) -> String {
    let mut chars = word.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}

fn is_claude_model(model: &str) -> bool {
    let lower = model.to_ascii_lowercase();
    ["claude", "opus", "sonnet", "haiku"]
        .iter()
        .any(|name| lower.contains(name))
}

/// `$1.26`, `$1,204`.
fn dollars(value: f64) -> String {
    if value < 1000.0 {
        return format!("${value:.2}");
    }
    let whole = format!("{:.0}", value);
    let mut grouped = String::new();
    for (index, digit) in whole.chars().enumerate() {
        if index > 0 && (whole.len() - index) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    format!("${grouped}")
}

fn filter_label(filter: Option<Tool>) -> &'static str {
    filter.map_or("All tools", Tool::label)
}

// ---------------------------------------------------------------------------------------
// Tab entry points

/// Draws the tab. Return `false` to fall back to the legacy page.
pub fn draw(ctx: &TabContext<'_>) -> bool {
    let painter = ctx.painter;
    let colors = ctx.colors;
    let state = page_state();
    STATE.with(|cell| cell.set(state));
    let snapshot = match (state, scan::snapshot()) {
        (PageState::Data, Some(snapshot)) => snapshot,
        _ => {
            draw_message(painter, ctx.grid, state, ctx.hover, colors);
            return true;
        }
    };
    let offset = scan::utc_offset();
    let now = scan::unix_now();
    let today = (now + offset).div_euclid(24 * 60 * 60);
    let hour = ((now + offset).rem_euclid(24 * 60 * 60) / 3600) as usize;
    let mut filter = FILTER.with(Cell::get);
    let mut view = build_view(&snapshot, filter, today);
    if filter.is_some_and(|tool| !view.tools.contains(&tool)) {
        filter = None;
        FILTER.with(|cell| cell.set(None));
        view = build_view(&snapshot, None, today);
    }
    TOOLS.with(|cell| *cell.borrow_mut() = view.tools.clone());

    let cards = cards(ctx.grid);
    for card in [cards.today, cards.hours, cards.month, cards.models] {
        fill_card(painter, card, colors);
    }
    draw_today(painter, cards.today, &view, filter, ctx.hover, colors);
    draw_hours(painter, cards.hours, &view, hour, ctx.hover, colors);
    draw_month(painter, cards.month, &view, colors);
    draw_models(painter, cards.models, &view, ctx.hover, colors);
    true
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
    match STATE.with(Cell::get) {
        PageState::Off => {
            return settings_button(grid)
                .contains(point)
                .then_some(DashboardHit::OpenSettings);
        }
        PageState::Data => {}
        _ => return None,
    }
    let cards = cards(grid);
    if chip_rect(cards.today).inset(-4.0).contains(point)
        && TOOLS.with(|cell| cell.borrow().len()) > 1
    {
        return Some(DashboardHit::Tab(ID_CHIP));
    }
    if cards.hours.contains(point) {
        return hour_columns(cards.hours)
            .iter()
            .position(|column| column.contains(point))
            .map(|hour| DashboardHit::Tab(ID_HOUR + hour as u16));
    }
    if cards.models.contains(point) {
        let rows = MODEL_ROWS.with(Cell::get);
        return (0..rows)
            .find(|index| model_row(cards.models, *index).contains(point))
            .map(|index| DashboardHit::Tab(ID_MODEL + index as u16));
    }
    None
}

/// Applies a click from [`hit`]. `true` when handled.
pub fn apply(hit: DashboardHit) -> bool {
    match hit {
        DashboardHit::Tab(ID_CHIP) => {
            // All tools, then each tool with activity in turn.
            let mut options = vec![None];
            options.extend(
                TOOLS
                    .with(|cell| cell.borrow().clone())
                    .into_iter()
                    .map(Some),
            );
            let current = FILTER.with(Cell::get);
            let index = options
                .iter()
                .position(|option| *option == current)
                .unwrap_or(0);
            FILTER.with(|cell| cell.set(options[(index + 1) % options.len()]));
            true
        }
        DashboardHit::Tab(id) => (ID_CHIP..ID_MODEL + MAX_MODEL_ROWS as u16).contains(&id),
        _ => false,
    }
}

pub fn wants_keyboard() -> bool {
    false
}

pub fn handle_key(_key: &Key) -> bool {
    false
}

/// Every frame, visible or not. Scans the logs once a minute while the tab is open.
pub fn poll(now: Instant) {
    if !enabled() || crate::core::safe_mode::active() {
        scan::reset();
        LAST_SCAN.with(|cell| cell.set(None));
        return;
    }
    if !crate::ui::dashboard::showing(Destination::AiUsage) || scan::scanning() {
        return;
    }
    if LAST_SCAN
        .with(Cell::get)
        .is_some_and(|at| now.saturating_duration_since(at) < RESCAN)
    {
        return;
    }
    if scan::start(scan::utc_offset()) {
        LAST_SCAN.with(|cell| cell.set(Some(now)));
    }
}

// ---------------------------------------------------------------------------------------
// Cards

fn draw_today(
    painter: Painter<'_>,
    card: Rect,
    view: &View,
    filter: Option<Tool>,
    hover: Option<DashboardHit>,
    colors: &SemanticColors,
) {
    card_header(painter, card, Icon::Chart, "Today", colors);

    // Tool chip: cycles All tools / Claude Code / Codex when more than one has activity.
    let chip = chip_rect(card);
    let clickable = view.tools.len() > 1;
    let hovered = clickable && hover == Some(DashboardHit::Tab(ID_CHIP));
    painter.fill_round_rect(
        chip,
        Radius::uniform(chip.height() / 2.0),
        colors
            .text_primary
            .with_alpha(if hovered { 38 } else { 16 }),
    );
    let shown_tool = filter.or_else(|| (view.tools.len() == 1).then(|| view.tools[0]));
    let icon_center = Point::new(chip.left + 13.0, chip.center_y());
    match shown_tool {
        Some(Tool::ClaudeCode) => draw_spark(painter, icon_center, 5.0, CLAUDE_ORANGE),
        Some(Tool::Codex) => draw_codex_mark(painter, icon_center, 5.0, colors.text_primary),
        None => draw_icon(
            painter,
            Icon::Grid,
            Rect::from_xywh(icon_center.x - 5.0, icon_center.y - 5.0, 10.0, 10.0),
            colors.text_secondary,
        ),
    }
    let label = shown_tool.map_or(filter_label(filter), Tool::label);
    text_at(
        painter,
        label,
        typography::LABEL,
        chip.left + 23.0,
        chip.center_y() + 4.2,
        chip.width() - 23.0 - if clickable { 18.0 } else { 6.0 },
        if hovered {
            colors.text_primary
        } else {
            colors.text_secondary
        },
    );
    if clickable {
        let x = chip.right - 12.0;
        let y = chip.center_y();
        for (from, to) in [
            ((x - 3.0, y - 1.5), (x, y + 1.5)),
            ((x, y + 1.5), (x + 3.0, y - 1.5)),
        ] {
            painter.stroke_line(
                Point::new(from.0, from.1),
                Point::new(to.0, to.1),
                1.4,
                colors.text_secondary,
                StrokeCap::Round,
            );
        }
    }

    // Tokens and their API value.
    let left = card.left + PAD;
    let tokens = format_tokens(view.tokens);
    text_at(
        painter,
        &tokens,
        BIG,
        left,
        card.top + 60.0,
        card.width() / 2.0,
        colors.text_primary,
    );
    let tokens_width = measure(&tokens, BIG);
    text_at(
        painter,
        "tokens",
        typography::BODY,
        left + tokens_width + 6.0,
        card.top + 60.0,
        80.0,
        colors.text_secondary,
    );
    let mut block_width = tokens_width + 6.0 + measure("tokens", typography::BODY);
    if view.tokens > 0 {
        let value = format!("≈ {}", dollars(view.cost));
        text_at(
            painter,
            &value,
            BODY_BOLD,
            left,
            card.top + 82.0,
            card.width() / 2.0,
            colors.text_primary,
        );
        let value_width = measure(&value, BODY_BOLD);
        text_at(
            painter,
            "API value today",
            typography::BODY,
            left + value_width + 5.0,
            card.top + 82.0,
            card.width() / 2.0,
            colors.text_secondary,
        );
        block_width =
            block_width.max(value_width + 5.0 + measure("API value today", typography::BODY));
    } else {
        text_at(
            painter,
            "No activity yet today",
            typography::BODY,
            left,
            card.top + 82.0,
            card.width() / 2.0,
            colors.text_secondary,
        );
        block_width = block_width.max(measure("No activity yet today", typography::BODY));
    }

    // Messages, tool calls and sessions.
    let stats_left = (card.left + card.width() * 0.48).max(left + block_width + 12.0);
    let stats_right = card.right - PAD;
    let column = ((stats_right - stats_left) / 3.0).max(0.0);
    for (index, (value, label)) in [
        (view.messages.to_string(), "Messages"),
        (view.tool_calls.to_string(), "Tool calls"),
        (view.sessions.to_string(), "Sessions"),
    ]
    .iter()
    .enumerate()
    {
        let center = stats_left + column * (index as f32 + 0.5);
        text_center(
            painter,
            value,
            STAT,
            center,
            card.top + 60.0,
            column,
            colors.text_primary,
        );
        text_center(
            painter,
            label,
            typography::CAPTION,
            center,
            card.top + 78.0,
            column,
            colors.text_secondary,
        );
    }

    // Footer: project and streak.
    let baseline = card.bottom - 12.0;
    let mut x = left;
    let max_right = card.right - PAD;
    if let Some(project) = &view.project {
        draw_folder(
            painter,
            Point::new(x, baseline - 9.0),
            colors.text_secondary,
        );
        x += 17.0;
        let room = (max_right - x - if view.streak >= 2 { 120.0 } else { 0.0 }).max(40.0);
        let width = measure(project, typography::LABEL).min(room);
        text_at(
            painter,
            project,
            typography::LABEL,
            x,
            baseline,
            width + 1.0,
            colors.text_secondary,
        );
        x += width;
        if view.streak >= 2 {
            text_at(
                painter,
                "·",
                typography::LABEL,
                x + 6.0,
                baseline,
                10.0,
                colors.text_secondary,
            );
            x += 18.0;
        }
    }
    if view.streak >= 2 {
        draw_icon(
            painter,
            Icon::Bolt,
            Rect::from_xywh(x, baseline - 10.0, 12.0, 12.0),
            colors.text_secondary,
        );
        text_at(
            painter,
            &format!("{}-day streak", view.streak),
            typography::LABEL,
            x + 16.0,
            baseline,
            (max_right - x - 16.0).max(0.0),
            colors.text_secondary,
        );
    }
}

fn draw_hours(
    painter: Painter<'_>,
    card: Rect,
    view: &View,
    current_hour: usize,
    hover: Option<DashboardHit>,
    colors: &SemanticColors,
) {
    card_header(painter, card, Icon::Clock, "Active hours", colors);
    let hovered = match hover {
        Some(DashboardHit::Tab(id)) if (ID_HOUR..ID_HOUR + 24).contains(&id) => {
            Some(usize::from(id - ID_HOUR))
        }
        _ => None,
    };
    // The busiest two-hour window.
    let peak = (0..23)
        .map(|hour| (hour, view.hours[hour] + view.hours[hour + 1]))
        .max_by_key(|(hour, tokens)| (*tokens, std::cmp::Reverse(*hour)))
        .filter(|(_, tokens)| *tokens > 0)
        .map(|(hour, _)| hour);
    let note = match hovered {
        Some(hour) if view.hours[hour] > 0 => {
            format!("{hour:02}:00 · {} tokens", format_tokens(view.hours[hour]))
        }
        Some(hour) => format!("{hour:02}:00 · idle"),
        None => peak.map_or_else(
            || "No activity yet today".to_string(),
            |hour| format!("Peak {hour:02}:00 – {:02}:00", (hour + 2) % 24),
        ),
    };
    text_right(
        painter,
        &note,
        typography::LABEL,
        card.right - PAD,
        card.top + 22.0,
        if hovered.is_some() {
            colors.text_primary
        } else {
            colors.text_secondary
        },
    );

    let plot_top = card.top + 34.0;
    let plot_bottom = card.bottom - 24.0;
    let plot_height = (plot_bottom - plot_top).max(4.0);
    let max = view.hours.iter().copied().max().unwrap_or(0).max(1);
    let columns = hour_columns(card);
    for (hour, column) in columns.iter().enumerate() {
        let tokens = view.hours[hour];
        let is_hovered = hovered == Some(hour);
        let in_peak = peak.is_some_and(|peak| hour == peak || hour == peak + 1);
        let bar_width = (column.width() * 0.42).clamp(3.0, 8.0);
        let x = column.center_x() - bar_width / 2.0;
        let color = if is_hovered || hour == current_hour {
            colors.text_primary
        } else if in_peak {
            colors.text_primary.with_alpha(140)
        } else {
            colors.text_primary.with_alpha(70)
        };
        if tokens == 0 {
            let dot = if hour == current_hour || is_hovered {
                colors.text_primary
            } else {
                colors.text_primary.with_alpha(26)
            };
            painter.fill_round_rect(
                Rect::from_xywh(x, plot_bottom - 3.0, bar_width, 3.0),
                Radius::uniform(1.5),
                dot,
            );
            continue;
        }
        let height = (tokens as f32 / max as f32 * plot_height).max(5.0);
        painter.fill_round_rect(
            Rect::from_xywh(x, plot_bottom - height, bar_width, height),
            Radius::uniform(bar_width / 2.0),
            color,
        );
    }
    for hour in [0usize, 6, 12, 18] {
        text_at(
            painter,
            &format!("{hour:02}:00"),
            typography::CAPTION,
            columns[hour].left,
            card.bottom - 9.0,
            44.0,
            colors.text_secondary,
        );
    }
}

fn draw_month(painter: Painter<'_>, card: Rect, view: &View, colors: &SemanticColors) {
    card_header(painter, card, Icon::Calendar, "This month", colors);
    if view.month_tokens > 0 {
        text_right(
            painter,
            &format!("{} tokens", format_tokens(view.month_tokens)),
            typography::LABEL,
            card.right - PAD,
            card.top + 22.0,
            colors.text_secondary,
        );
    }
    text_at(
        painter,
        &format!("≈ {}", dollars(view.month_cost)),
        HERO,
        card.left + PAD,
        card.top + 58.0,
        card.width() - PAD * 2.0,
        colors.text_primary,
    );
    text_at(
        painter,
        "Estimated at API prices",
        typography::CAPTION,
        card.left + PAD,
        (card.top + 77.0).min(card.bottom - 8.0),
        card.width() - PAD * 2.0,
        colors.text_secondary,
    );
}

fn draw_models(
    painter: Painter<'_>,
    card: Rect,
    view: &View,
    hover: Option<DashboardHit>,
    colors: &SemanticColors,
) {
    card_header(painter, card, Icon::Chart, "By model", colors);
    let fits = fitting_model_rows(card);
    let shown = view.models.len().min(fits);
    MODEL_ROWS.with(|cell| cell.set(shown));
    let hidden = view.models.len() - shown;
    let note = if hidden > 0 {
        format!("{} · +{hidden} more", view.models_scope)
    } else {
        view.models_scope.to_string()
    };
    text_right(
        painter,
        &note,
        typography::LABEL,
        card.right - PAD,
        card.top + 22.0,
        colors.text_secondary,
    );
    if view.models.is_empty() {
        draw_text(
            painter,
            "No model usage yet",
            typography::LABEL,
            Rect::from_ltrb(card.left, card.top + 30.0, card.right, card.bottom - 6.0),
            Align::Center,
            colors.text_secondary,
        );
        return;
    }
    let max = view.models.first().map_or(1, |model| model.1.max(1));
    for (index, (model, tokens, cost)) in view.models.iter().take(shown).enumerate() {
        let row = model_row(card, index);
        if hover == Some(DashboardHit::Tab(ID_MODEL + index as u16)) {
            painter.fill_round_rect(
                row,
                Radius::uniform(10.0),
                colors.text_primary.with_alpha(14),
            );
        }
        let icon = Point::new(row.left + 18.0, row.center_y());
        if is_claude_model(model) {
            draw_spark(painter, icon, 8.0, CLAUDE_ORANGE);
        } else {
            draw_codex_mark(painter, icon, 7.0, colors.text_primary);
        }
        let name_left = row.left + 36.0;
        let right = row.right - 8.0;
        let token_text = format_tokens(*tokens);
        let cost_text = format!("≈ {}", dollars(*cost));
        let column = measure(&token_text, FIGURE)
            .max(measure(&cost_text, typography::CAPTION))
            .max(44.0);
        text_right(
            painter,
            &token_text,
            FIGURE,
            right,
            row.top + 16.0,
            colors.text_primary,
        );
        text_right(
            painter,
            &cost_text,
            typography::CAPTION,
            right,
            row.top + 32.0,
            colors.text_secondary,
        );
        let name_right = right - column - 10.0;
        text_at(
            painter,
            &model_label(model),
            NAME,
            name_left,
            row.top + 16.0,
            (name_right - name_left).max(0.0),
            colors.text_primary,
        );
        let bar = Rect::from_xywh(
            name_left,
            row.top + 25.0,
            (name_right - name_left).max(0.0),
            3.0,
        );
        painter.fill_round_rect(
            bar,
            Radius::uniform(1.5),
            colors.text_primary.with_alpha(22),
        );
        let share = (*tokens as f32 / max as f32).clamp(0.0, 1.0);
        painter.fill_round_rect(
            Rect::from_xywh(
                bar.left,
                bar.top,
                (bar.width() * share).max(bar.height()),
                bar.height(),
            ),
            Radius::uniform(1.5),
            colors.text_primary,
        );
    }
}

/// Off, safe mode, loading and empty states: one card with a short explanation.
fn draw_message(
    painter: Painter<'_>,
    grid: Rect,
    state: PageState,
    hover: Option<DashboardHit>,
    colors: &SemanticColors,
) {
    let (title, lines): (&str, [&str; 2]) = match state {
        PageState::Off => (
            "AI coding stats are off",
            [
                "Turn on AI usage in Settings to read Claude Code and Codex logs on this PC.",
                "Only token counts and timestamps are read. Nothing leaves your computer.",
            ],
        ),
        PageState::SafeMode => (
            "Paused in safe mode",
            [
                "Restart WinIsland normally to see your AI coding stats.",
                "",
            ],
        ),
        PageState::Loading => (
            "Reading local logs…",
            ["Claude Code and Codex usage appears here in a moment.", ""],
        ),
        PageState::Empty | PageState::Data => (
            "No AI coding activity yet",
            [
                "Use Claude Code or Codex on this PC and your tokens, sessions",
                "and models show up here.",
            ],
        ),
    };
    fill_card(painter, grid, colors);
    let cx = grid.center_x();
    let cy = grid.center_y();
    draw_spark(
        painter,
        Point::new(cx, cy - 52.0),
        11.0,
        if state == PageState::Off {
            colors.text_secondary
        } else {
            CLAUDE_ORANGE
        },
    );
    text_center(
        painter,
        title,
        typography::SECTION_TITLE,
        cx,
        cy - 14.0,
        grid.width() - PAD * 2.0,
        colors.text_primary,
    );
    for (index, line) in lines.iter().enumerate() {
        text_center(
            painter,
            line,
            typography::LABEL,
            cx,
            cy + 8.0 + index as f32 * 17.0,
            grid.width() - PAD * 2.0,
            colors.text_secondary,
        );
    }
    if state == PageState::Off {
        let button = settings_button(grid);
        let hovered = hover == Some(DashboardHit::OpenSettings);
        painter.fill_round_rect(
            button,
            Radius::uniform(button.height() / 2.0),
            colors
                .text_primary
                .with_alpha(if hovered { 44 } else { 24 }),
        );
        draw_text(
            painter,
            "Open Settings",
            typography::LABEL,
            button,
            Align::Center,
            colors.text_primary,
        );
    }
}

// ---------------------------------------------------------------------------------------
// Drawing helpers

fn fill_card(painter: Painter<'_>, card: Rect, colors: &SemanticColors) {
    painter.fill_round_rect(
        card,
        Radius::uniform(radius::CARD + 6.0),
        colors.surface_raised,
    );
}

fn card_header(painter: Painter<'_>, card: Rect, icon: Icon, title: &str, colors: &SemanticColors) {
    draw_icon(
        painter,
        icon,
        Rect::from_xywh(card.left + PAD, card.top + 11.0, 13.0, 13.0),
        colors.text_secondary,
    );
    text_at(
        painter,
        title,
        typography::LABEL,
        card.left + PAD + 19.0,
        card.top + 22.0,
        card.width() / 2.0,
        colors.text_secondary,
    );
}

/// Claude's mark: twelve rays from the centre.
fn draw_spark(painter: Painter<'_>, center: Point, radius: f32, color: Rgba) {
    for step in 0..6 {
        let angle = (step as f32 * 30.0).to_radians();
        let (dx, dy) = (angle.cos() * radius, angle.sin() * radius);
        painter.stroke_line(
            Point::new(center.x - dx, center.y - dy),
            Point::new(center.x + dx, center.y + dy),
            (radius * 0.26).max(1.4),
            color,
            StrokeCap::Round,
        );
    }
}

/// A plain ring with a dot for Codex and other models.
fn draw_codex_mark(painter: Painter<'_>, center: Point, radius: f32, color: Rgba) {
    painter.stroke_circle(center, radius, 1.6, color);
    painter.fill_circle(center, radius * 0.32, color);
}

/// A small folder outline whose top left is `origin`.
fn draw_folder(painter: Painter<'_>, origin: Point, color: Rgba) {
    painter.stroke_round_rect(
        Rect::from_xywh(origin.x, origin.y + 2.0, 12.0, 9.0),
        Radius::uniform(2.0),
        1.3,
        color,
    );
    painter.stroke_line(
        Point::new(origin.x + 1.0, origin.y + 1.0),
        Point::new(origin.x + 5.0, origin.y + 1.0),
        1.3,
        color,
        StrokeCap::Round,
    );
}

fn measure(text: &str, style: TextStyle) -> f32 {
    let font = if style.weight >= 600 {
        FontStyle::bold()
    } else {
        FontStyle::normal()
    };
    FontManager::global().measure_text_cached(text, style.size, font)
}

/// One line of text whose baseline sits at `baseline`, starting at `x`.
fn text_at(
    painter: Painter<'_>,
    text: &str,
    style: TextStyle,
    x: f32,
    baseline: f32,
    width: f32,
    color: Rgba,
) {
    let center = baseline - style.size * 0.35;
    draw_text(
        painter,
        text,
        style,
        Rect::from_xywh(x, center - style.size * 0.75, width, style.size * 1.5),
        Align::Start,
        color,
    );
}

/// One line of text ending at `right`.
fn text_right(
    painter: Painter<'_>,
    text: &str,
    style: TextStyle,
    right: f32,
    baseline: f32,
    color: Rgba,
) {
    let width = measure(text, style);
    text_at(
        painter,
        text,
        style,
        right - width - 1.0,
        baseline,
        width + 2.0,
        color,
    );
}

/// One line of text centred on `center_x`, at most `width` wide.
fn text_center(
    painter: Painter<'_>,
    text: &str,
    style: TextStyle,
    center_x: f32,
    baseline: f32,
    width: f32,
    color: Rgba,
) {
    let center = baseline - style.size * 0.35;
    draw_text(
        painter,
        text,
        style,
        Rect::from_xywh(
            center_x - width / 2.0,
            center - style.size * 0.75,
            width,
            style.size * 1.5,
        ),
        Align::Center,
        color,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_labels() {
        assert_eq!(model_label("claude-opus-4-1-20250805"), "Opus 4.1");
        assert_eq!(model_label("claude-3-5-sonnet-20241022"), "Sonnet 3.5");
        assert_eq!(model_label("gpt-5-codex"), "GPT-5 Codex");
        assert_eq!(dollars(1204.4), "$1,204");
        assert_eq!(dollars(1.256), "$1.26");
    }
}
