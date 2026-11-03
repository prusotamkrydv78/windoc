//! Memory tab: Notchy's timeline and "What did I miss?".
//!
//! Left card: the latest recap with "Recap last 3 hours", or (after "Settings") the AI and
//! database settings: base URL with OpenRouter / OpenAI presets, API key, model, and the SQL
//! Server connection string, each an editable field, plus Save, Test AI and live status.
//! Right card: a search field over the timeline and the newest matching entries.

use std::cell::{Cell, RefCell};
use std::time::{Duration, Instant};

use winisland_platform::Key;
use winisland_render::tokens::{TextStyle, radius, typography};
use winisland_render::{Point, Radius, Rect, Rgba};

use super::TabContext;
use super::shelf::kit::{self, style, text, text_center, text_right, text_width};
use crate::core::brain::{self, Settings};
use crate::core::memory::{self, Entry, Kind};
use crate::core::memory_sql::{self, Status};
use crate::core::recap::{self, Reason};
use crate::ui::dashboard::DashboardHit;

const ID_RECAP: u16 = 1700;
const ID_SETTINGS: u16 = 1701;
const ID_SEARCH: u16 = 1702;
const ID_SAVE: u16 = 1703;
const ID_TEST: u16 = 1704;
const ID_BACK: u16 = 1705;
const ID_PRESET_OPENROUTER: u16 = 1706;
const ID_PRESET_OPENAI: u16 = 1707;
const ID_FIELD: u16 = 1710;
const ID_MAGIC: u16 = 1720;
const MAGIC: [crate::core::assistant::Magic; 3] = [
    crate::core::assistant::Magic::Translate,
    crate::core::assistant::Magic::Summarise,
    crate::core::assistant::Magic::Fix,
];
const FIELDS: usize = 4;

const GAP: f32 = 10.0;
const PAD: f32 = 14.0;
const ROW_H: f32 = 40.0;
const FIELD_H: f32 = 28.0;

const TITLE: TextStyle = style(16.0, 650);
const NAME: TextStyle = style(13.0, 600);
const BUTTON: TextStyle = style(12.0, 600);
const BODY: TextStyle = style(13.0, 450);

const FIELD_LABELS: [&str; FIELDS] = ["AI base URL", "API key", "Model", "SQL Server"];
const FIELD_HINTS: [&str; FIELDS] = [
    "https://openrouter.ai/api/v1",
    "sk-… or sk-or-…",
    "openai/gpt-4o-mini",
    "Server=tcp:localhost,1433;Database=Notchy;User Id=…;Password=…;TrustServerCertificate=true",
];

#[derive(Default)]
struct State {
    settings_open: bool,
    /// The settings being edited, or `None` to show the saved ones.
    draft: Option<[String; FIELDS]>,
    focus: Option<usize>,
    search_focused: bool,
    query: String,
    saved_at: Option<Instant>,
    test: Option<Result<String, String>>,
    testing: bool,
    /// The left card shows the last answer instead of the recap.
    show_answer: bool,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
    static LAST_GRID: Cell<Rect> = const { Cell::new(Rect::from_xywh(0.0, 0.0, 720.0, 230.0)) };
}

static TEST_RESULT: parking_lot::Mutex<Option<Result<String, String>>> = parking_lot::Mutex::new(None);

fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> R {
    STATE.with(|cell| f(&mut cell.borrow_mut()))
}

fn fields_of(settings: &Settings) -> [String; FIELDS] {
    [
        settings.ai_base_url.clone(),
        settings.ai_key.clone(),
        settings.ai_model.clone(),
        settings.sql_connection.clone(),
    ]
}

struct Layout {
    left: Rect,
    right: Rect,
    search: Rect,
    rows: Rect,
    recap_button: Rect,
    settings_button: Rect,
    fields: [Rect; FIELDS],
    magic: [Rect; 3],
    presets: [Rect; 2],
    save: Rect,
    test: Rect,
    back: Rect,
}

fn layout(grid: Rect) -> Layout {
    let left_w = (grid.width() * 0.48).clamp(300.0, 430.0).min(grid.width());
    let left = Rect::from_xywh(grid.left, grid.top, left_w, grid.height());
    let right = Rect::from_ltrb(left.right + GAP, grid.top, grid.right, grid.bottom);
    let inner = left.inset(PAD);
    let right_inner = right.inset(PAD);
    let search = Rect::from_xywh(right_inner.left, right_inner.top, right_inner.width(), 30.0);
    let rows = Rect::from_ltrb(right_inner.left - 6.0, search.bottom + 8.0, right_inner.right + 6.0, right.bottom - 8.0);
    let bottom = inner.bottom - 30.0;
    let recap_w = text_width("Recap last 3 hours", BUTTON) + 30.0;
    let recap_button = Rect::from_xywh(inner.left, bottom, recap_w, 30.0);
    let settings_w = text_width("Settings", BUTTON) + 28.0;
    let settings_button = Rect::from_xywh(inner.right - settings_w, bottom, settings_w, 30.0);
    // Settings view: four label+field rows, presets beside the URL label, buttons at the bottom.
    let label_w = 78.0;
    let mut fields = [Rect::default(); FIELDS];
    for (index, field) in fields.iter_mut().enumerate() {
        let top = inner.top + 26.0 + index as f32 * (FIELD_H + 8.0);
        *field = Rect::from_ltrb(inner.left + label_w, top, inner.right, top + FIELD_H);
    }
    let openai_w = text_width("OpenAI", BUTTON) + 20.0;
    let openrouter_w = text_width("OpenRouter", BUTTON) + 20.0;
    let presets = [
        Rect::from_xywh(inner.right - openai_w - openrouter_w - 6.0, inner.top - 2.0, openrouter_w, 22.0),
        Rect::from_xywh(inner.right - openai_w, inner.top - 2.0, openai_w, 22.0),
    ];
    let save_w = text_width("Save", BUTTON) + 36.0;
    let test_w = text_width("Test AI", BUTTON) + 30.0;
    let back_w = text_width("Done", BUTTON) + 30.0;
    let save = Rect::from_xywh(inner.left, bottom, save_w, 30.0);
    let test = Rect::from_xywh(save.right + 8.0, bottom, test_w, 30.0);
    let back = Rect::from_xywh(inner.right - back_w, bottom, back_w, 30.0);
    // Clipboard magic: three small pills above the bottom buttons.
    let mut magic = [Rect::default(); 3];
    let mut left_x = inner.left;
    for (index, rect) in magic.iter_mut().enumerate() {
        let width = text_width(MAGIC[index].label(), BUTTON) + 22.0;
        *rect = Rect::from_xywh(left_x, bottom - 32.0, width, 24.0);
        left_x += width + 6.0;
    }
    Layout {
        magic,
        left,
        right,
        search,
        rows,
        recap_button,
        settings_button,
        fields,
        presets,
        save,
        test,
        back,
    }
}

fn hovered(ctx: &TabContext<'_>, id: u16) -> bool {
    ctx.hover == Some(DashboardHit::Tab(id))
}

fn card(ctx: &TabContext<'_>, rect: Rect) {
    ctx.painter
        .fill_round_rect(rect, Radius::uniform(radius::CARD), ctx.colors.surface_raised);
    ctx.painter.stroke_round_rect(rect.inset(0.5), Radius::uniform(radius::CARD), 1.0, ctx.colors.border_subtle);
}

fn button(ctx: &TabContext<'_>, rect: Rect, label: &str, hover: bool, primary: bool) {
    let colors = ctx.colors;
    let fill = match (primary, hover) {
        (true, false) => colors.accent_primary,
        (true, true) => colors.accent_primary.with_alpha(220),
        (false, false) => colors.surface_overlay,
        (false, true) => colors.surface_interactive,
    };
    ctx.painter.fill_round_rect(rect, Radius::uniform(rect.height() / 2.0), fill);
    text_center(
        ctx.painter,
        label,
        BUTTON,
        rect,
        if primary { Rgba::WHITE } else if hover { colors.text_primary } else { colors.text_secondary },
    );
}

/// Greedy word wrap into at most `max_lines` lines that fit `width`.
fn wrap(text: &str, style: TextStyle, width: f32, max_lines: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        let candidate = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
        if text_width(&candidate, style) <= width || line.is_empty() {
            line = candidate;
        } else {
            lines.push(std::mem::take(&mut line));
            line = word.to_string();
            if lines.len() == max_lines {
                break;
            }
        }
    }
    if !line.is_empty() && lines.len() < max_lines {
        lines.push(line);
    } else if lines.len() == max_lines
        && let Some(last) = lines.last_mut()
    {
        last.push('…');
    }
    lines
}

fn ago(at_ms: u64) -> String {
    let seconds = memory::now_ms().saturating_sub(at_ms) / 1000;
    match seconds {
        0..60 => "now".to_string(),
        60..3600 => format!("{}m", seconds / 60),
        3600..86_400 => format!("{}h", seconds / 3600),
        _ => format!("{}d", seconds / 86_400),
    }
}

pub fn draw(ctx: &TabContext<'_>) -> bool {
    LAST_GRID.with(|cell| cell.set(ctx.grid));
    let layout = layout(ctx.grid);
    card(ctx, layout.left);
    card(ctx, layout.right);
    if let Some(result) = TEST_RESULT.lock().take() {
        with_state(|state| {
            state.test = Some(result);
            state.testing = false;
        });
    }
    with_state(|state| {
        if state.settings_open {
            draw_settings(ctx, &layout, state);
        } else {
            draw_recap(ctx, &layout, state);
        }
        draw_timeline(ctx, &layout, state);
    });
    true
}

fn draw_recap(ctx: &TabContext<'_>, layout: &Layout, state: &State) {
    let painter = ctx.painter;
    let colors = ctx.colors;
    let inner = layout.left.inset(PAD);
    let answer = state.show_answer.then(crate::core::assistant::last_answer).flatten();
    let heading = if state.show_answer { "Answer" } else { "What did I miss?" };
    text(painter, heading, TITLE, Rect::from_xywh(inner.left, inner.top, inner.width(), 22.0), colors.text_primary);
    let (body, meta) = match recap::latest() {
        _ if state.show_answer && crate::core::assistant::busy() => ("Thinking…".to_string(), String::new()),
        _ if state.show_answer => match answer {
            Some((question, answer)) => (answer, format!("You asked: {question}")),
            None => ("Thinking…".to_string(), String::new()),
        },
        _ if recap::busy() => ("Thinking…".to_string(), String::new()),
        Some(latest) => {
            let by = latest.model.map_or("Built-in summary".to_string(), |model| format!("by {model}"));
            (latest.text, format!("{} · {} ago", by, ago(latest.at)))
        }
        None => (
            "When you step away and come back, the robot sums up what your phone got. Ask any time with the button below.".to_string(),
            match recap::model() {
                Some(model) => format!("AI: {model}"),
                None => "Built-in summary · add an AI key in Settings for smarter recaps".to_string(),
            },
        ),
    };
    let lines = wrap(&body, BODY, inner.width(), 4);
    let mut top = inner.top + 30.0;
    for line in &lines {
        text(painter, line, BODY, Rect::from_xywh(inner.left, top, inner.width(), 18.0), colors.text_primary);
        top += 19.0;
    }
    text(painter, &meta, typography::CAPTION, Rect::from_xywh(inner.left, top + 4.0, inner.width(), 16.0), colors.text_secondary);
    for (index, rect) in layout.magic.iter().enumerate() {
        let id = ID_MAGIC + index as u16;
        let hover = hovered(ctx, id);
        painter.fill_round_rect(*rect, Radius::uniform(12.0), if hover { colors.surface_interactive } else { colors.surface_overlay });
        text_center(painter, MAGIC[index].label(), BUTTON, *rect, if hover { colors.text_primary } else { colors.text_secondary });
    }
    button(ctx, layout.recap_button, if recap::busy() { "Recapping…" } else { "Recap last 3 hours" }, hovered(ctx, ID_RECAP), true);
    button(ctx, layout.settings_button, "Settings", hovered(ctx, ID_SETTINGS), false);
}

fn draw_settings(ctx: &TabContext<'_>, layout: &Layout, state: &State) {
    let painter = ctx.painter;
    let colors = ctx.colors;
    let inner = layout.left.inset(PAD);
    text(painter, "AI and database", TITLE, Rect::from_xywh(inner.left, inner.top, inner.width(), 22.0), colors.text_primary);
    for (index, (rect, id)) in layout.presets.iter().zip([ID_PRESET_OPENROUTER, ID_PRESET_OPENAI]).enumerate() {
        let hover = hovered(ctx, id);
        painter.fill_round_rect(*rect, Radius::uniform(11.0), if hover { colors.surface_interactive } else { colors.surface_overlay });
        text_center(painter, ["OpenRouter", "OpenAI"][index], BUTTON, *rect, colors.text_secondary);
    }
    let values = state.draft.clone().unwrap_or_else(|| fields_of(&brain::settings()));
    for index in 0..FIELDS {
        let field = layout.fields[index];
        text(
            painter,
            FIELD_LABELS[index],
            typography::CAPTION,
            Rect::from_ltrb(inner.left, field.top, field.left - 6.0, field.bottom),
            colors.text_secondary,
        );
        let focused = state.focus == Some(index);
        painter.fill_round_rect(field, Radius::uniform(radius::CONTROL), colors.surface_base);
        painter.stroke_round_rect(
            field,
            Radius::uniform(radius::CONTROL),
            1.0,
            if focused { colors.accent_primary } else { colors.border_subtle },
        );
        let value = &values[index];
        // Secrets are masked unless being edited.
        let shown = match index {
            1 if !value.is_empty() && !focused => format!("{}••••••••", value.chars().take(6).collect::<String>()),
            3 if !value.is_empty() && !focused => mask_password(value),
            _ => value.clone(),
        };
        let area = Rect::from_ltrb(field.left + 8.0, field.top, field.right - 8.0, field.bottom);
        if shown.is_empty() && !focused {
            text(painter, FIELD_HINTS[index], typography::CAPTION, area, colors.text_disabled);
        } else {
            // Keep the end (where the caret is) visible in long values.
            let mut visible = shown.clone();
            while text_width(&visible, typography::CAPTION) > area.width() - 4.0 && !visible.is_empty() {
                visible.remove(0);
            }
            text(painter, &visible, typography::CAPTION, area, colors.text_primary);
            if focused && kit::caret_visible() {
                let x = (area.left + text_width(&visible, typography::CAPTION) + 1.0).min(area.right);
                painter.fill_rect(Rect::from_xywh(x, field.center_y() - 7.0, 1.2, 14.0), colors.accent_primary);
            }
        }
    }
    // Status lines under the fields.
    let mut top = layout.fields[FIELDS - 1].bottom + 8.0;
    let ai_line = match (&state.test, state.testing) {
        (_, true) => ("Testing the AI…".to_string(), colors.text_secondary),
        (Some(Ok(reply)), _) => (format!("AI works: “{}”", reply.trim()), colors.state_success),
        (Some(Err(error)), _) => (error.clone(), colors.state_error),
        (None, _) => ("AI: OpenRouter, OpenAI or any OpenAI-compatible URL".to_string(), colors.text_secondary),
    };
    text(painter, &ai_line.0, typography::CAPTION, Rect::from_xywh(inner.left, top, inner.width(), 16.0), ai_line.1);
    top += 18.0;
    let (sql_line, sql_color) = match memory_sql::status() {
        Status::Off => ("Database: local files only".to_string(), colors.text_secondary),
        Status::Connecting => ("Database: connecting to SQL Server…".to_string(), colors.text_secondary),
        Status::Connected { rows } => (format!("Database: SQL Server connected · {rows} rows"), colors.state_success),
        Status::Failed(error) => (format!("Database: {error}"), colors.state_error),
    };
    text(painter, &sql_line, typography::CAPTION, Rect::from_xywh(inner.left, top, inner.width(), 16.0), sql_color);

    let saved = state.saved_at.is_some_and(|at| at.elapsed() < Duration::from_millis(1600));
    button(ctx, layout.save, if saved { "Saved" } else { "Save" }, hovered(ctx, ID_SAVE), true);
    button(ctx, layout.test, "Test AI", hovered(ctx, ID_TEST), false);
    button(ctx, layout.back, "Done", hovered(ctx, ID_BACK), false);
}

/// `Password=secret` → `Password=••••` in a connection string.
fn mask_password(connection: &str) -> String {
    connection
        .split(';')
        .map(|part| match part.split_once('=') {
            Some((key, _)) if matches!(key.trim().to_lowercase().as_str(), "password" | "pwd") => format!("{key}=••••"),
            _ => part.to_string(),
        })
        .collect::<Vec<_>>()
        .join(";")
}

fn kind_color(kind: Kind, ctx: &TabContext<'_>) -> Rgba {
    match kind {
        Kind::PhoneAlert => ctx.colors.accent_primary,
        Kind::Clip => ctx.colors.state_info,
        Kind::FileIn | Kind::FileOut | Kind::Download => ctx.colors.state_success,
        Kind::Focus | Kind::Meeting => ctx.colors.state_warning,
        Kind::Recap | Kind::Note => ctx.colors.text_secondary,
    }
}

fn draw_timeline(ctx: &TabContext<'_>, layout: &Layout, state: &State) {
    let painter = ctx.painter;
    let colors = ctx.colors;
    if state.search_focused {
        kit::search_field(painter, layout.search, &state.query, "Search, or ask a question ending in ?", colors);
    } else {
        let hover = hovered(ctx, ID_SEARCH);
        painter.fill_round_rect(layout.search, Radius::uniform(15.0), if hover { colors.surface_interactive } else { colors.surface_overlay });
        text(
            painter,
            if state.query.is_empty() { "Search, or ask a question ending in ?" } else { &state.query },
            typography::LABEL,
            Rect::from_ltrb(layout.search.left + 14.0, layout.search.top, layout.search.right - 10.0, layout.search.bottom),
            colors.text_secondary,
        );
    }
    let fit = (layout.rows.height() / ROW_H).floor().max(0.0) as usize;
    let mut entries: Vec<Entry> = memory::search(&state.query, fit);
    // Universal search: Notes and the clipboard history join the timeline's matches.
    if !state.query.trim().is_empty() && !state.query.trim_end().ends_with('?') {
        let now = memory::now_ms();
        for (title, detail) in crate::ui::dashboard::note_matches(&state.query, 3) {
            entries.push(Entry { at: now, kind: Kind::Note, title: format!("Note · {title}"), detail, app: String::new() });
        }
        for text in crate::core::phone_link::history_matches(&state.query, 3) {
            entries.push(Entry { at: now, kind: Kind::Clip, title: "Clipboard".to_string(), detail: text, app: String::new() });
        }
        entries.truncate(fit);
    }
    if entries.is_empty() {
        text_center(
            painter,
            if state.query.is_empty() { "Your timeline fills up as the phone and PC talk" } else { "Nothing matches" },
            typography::LABEL,
            Rect::from_xywh(layout.rows.left, layout.rows.center_y() - 10.0, layout.rows.width(), 20.0),
            colors.text_secondary,
        );
        return;
    }
    for (slot, entry) in entries.iter().enumerate() {
        let row = Rect::from_xywh(layout.rows.left, layout.rows.top + slot as f32 * ROW_H, layout.rows.width(), ROW_H - 2.0);
        let badge_w = text_width(entry.kind.label(), style(10.0, 650)) + 12.0;
        let badge = Rect::from_xywh(row.left + 8.0, row.top + 5.0, badge_w, 16.0);
        let color = kind_color(entry.kind, ctx);
        painter.fill_round_rect(badge, Radius::uniform(8.0), color.with_alpha(40));
        text_center(painter, entry.kind.label(), style(10.0, 650), badge, color);
        let when_w = text_right(
            painter,
            &ago(entry.at),
            typography::CAPTION,
            Rect::from_ltrb(row.left, row.top + 4.0, row.right - 8.0, row.top + 20.0),
            colors.text_disabled,
        );
        let title = if entry.app.is_empty() { entry.title.clone() } else { format!("{} · {}", entry.app, entry.title) };
        text(
            painter,
            &title,
            NAME,
            Rect::from_ltrb(badge.right + 8.0, row.top + 3.0, row.right - 8.0 - when_w - 6.0, row.top + 21.0),
            colors.text_primary,
        );
        text(
            painter,
            &entry.detail.replace('\n', " "),
            typography::CAPTION,
            Rect::from_ltrb(row.left + 8.0, row.top + 21.0, row.right - 8.0, row.bottom),
            colors.text_secondary,
        );
    }
}

pub fn custom_layout() -> bool {
    true
}

pub fn hit(grid: Rect, point: Point) -> Option<DashboardHit> {
    if !grid.contains(point) {
        return None;
    }
    LAST_GRID.with(|cell| cell.set(grid));
    let layout = layout(grid);
    let open = with_state(|state| state.settings_open);
    let id = if layout.search.contains(point) {
        Some(ID_SEARCH)
    } else if let Some(index) = (!open).then(|| layout.magic.iter().position(|rect| rect.contains(point))).flatten() {
        Some(ID_MAGIC + index as u16)
    } else if !open && layout.recap_button.contains(point) {
        Some(ID_RECAP)
    } else if !open && layout.settings_button.contains(point) {
        Some(ID_SETTINGS)
    } else if open && layout.save.contains(point) {
        Some(ID_SAVE)
    } else if open && layout.test.contains(point) {
        Some(ID_TEST)
    } else if open && layout.back.contains(point) {
        Some(ID_BACK)
    } else if open && layout.presets[0].contains(point) {
        Some(ID_PRESET_OPENROUTER)
    } else if open && layout.presets[1].contains(point) {
        Some(ID_PRESET_OPENAI)
    } else if open {
        layout
            .fields
            .iter()
            .position(|field| field.contains(point))
            .map(|index| ID_FIELD + index as u16)
    } else {
        None
    };
    // A click elsewhere in the tab ends editing.
    Some(DashboardHit::Tab(id.unwrap_or(ID_FIELD + 99)))
}

fn draft(state: &mut State) -> &mut [String; FIELDS] {
    state.draft.get_or_insert_with(|| fields_of(&brain::settings()))
}

fn save(state: &mut State) {
    if let Some(values) = state.draft.take() {
        let [ai_base_url, ai_key, ai_model, sql_connection] = values;
        brain::save(Settings {
            ai_base_url,
            ai_key,
            ai_model,
            sql_connection,
        });
    }
    state.focus = None;
    state.saved_at = Some(Instant::now());
}

pub fn apply(hit: DashboardHit) -> bool {
    let DashboardHit::Tab(id) = hit else {
        return false;
    };
    if !(1700..=1810).contains(&id) {
        return false;
    }
    with_state(|state| {
        state.search_focused = id == ID_SEARCH;
        if !(ID_FIELD..ID_FIELD + FIELDS as u16).contains(&id) {
            state.focus = None;
        }
        match id {
            _ if (ID_MAGIC..ID_MAGIC + 3).contains(&id) => {
                let text = winisland_platform_windows::get_clipboard_text().unwrap_or_default();
                crate::core::assistant::magic(MAGIC[(id - ID_MAGIC) as usize], text);
            }
            ID_RECAP => {
                state.show_answer = false;
                recap::request(memory::now_ms().saturating_sub(3 * 3_600_000), Reason::Manual);
            }
            ID_SETTINGS => state.settings_open = true,
            ID_BACK => {
                state.settings_open = false;
                state.draft = None;
            }
            ID_SAVE => save(state),
            ID_TEST => {
                save(state);
                state.testing = true;
                state.test = None;
                if let Ok(handle) = tokio::runtime::Handle::try_current() {
                    handle.spawn(async {
                        let result = brain::test_ai().await;
                        *TEST_RESULT.lock() = Some(result);
                        crate::platform::wake();
                    });
                }
            }
            ID_PRESET_OPENROUTER | ID_PRESET_OPENAI => {
                let values = draft(state);
                if id == ID_PRESET_OPENROUTER {
                    values[0] = brain::OPENROUTER.to_string();
                    values[2] = "openai/gpt-4o-mini".to_string();
                } else {
                    values[0] = brain::OPENAI.to_string();
                    values[2] = "gpt-4o-mini".to_string();
                }
            }
            _ if (ID_FIELD..ID_FIELD + FIELDS as u16).contains(&id) => {
                draft(state);
                state.focus = Some((id - ID_FIELD) as usize);
            }
            _ => {}
        }
    });
    true
}

pub fn wants_keyboard() -> bool {
    with_state(|state| state.search_focused || state.focus.is_some())
}

pub fn handle_key(key: &Key) -> bool {
    with_state(|state| {
        if state.search_focused {
            match key {
                Key::Escape => {
                    state.search_focused = false;
                    state.query.clear();
                }
                Key::Backspace => {
                    state.query.pop();
                }
                Key::Enter => {
                    state.search_focused = false;
                    // A question: the robot answers from Memory.
                    if state.query.trim_end().ends_with('?') {
                        crate::core::assistant::ask(state.query.trim().to_string(), crate::core::assistant::Asker::Tab);
                        state.show_answer = true;
                        state.settings_open = false;
                    }
                }
                Key::Character(text) => {
                    if state.query.chars().count() < 80 {
                        state.query.extend(text.chars().filter(|character| !character.is_control()));
                    }
                }
                _ => {}
            }
            return true;
        }
        let Some(index) = state.focus else {
            return false;
        };
        match key {
            Key::Escape => state.focus = None,
            Key::Enter => save(state),
            Key::Tab => state.focus = Some((index + 1) % FIELDS),
            Key::Backspace => {
                draft(state)[index].pop();
            }
            Key::Character(text) => {
                let value = &mut draft(state)[index];
                if value.chars().count() < 500 {
                    value.extend(text.chars().filter(|character| !character.is_control()));
                }
            }
            _ => {}
        }
        true
    })
}

pub fn poll(_now: Instant) {}

/// Whether the tab needs frames (a caret blinks or a recap is running).
pub fn animating() -> bool {
    wants_keyboard() || recap::busy() || crate::core::assistant::busy() || with_state(|state| state.testing)
}
