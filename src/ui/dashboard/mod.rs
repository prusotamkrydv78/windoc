//! Dashboard shell: header, responsive card grid and bottom dock (roadmap step 5).
//!
//! Content comes from deterministic sample data until modules provide snapshots. The
//! header always labels it as sample data.

// `semantics` and grid scrolling (`overflows`, `content_height`) wait for the UI Automation
// provider and scroll support.
#![allow(dead_code)]

pub mod layout;
pub mod sample;

use std::cell::{Cell, RefCell};

use winisland_core::i18n::tr;
use winisland_render::tokens::{ColorScheme, SemanticColors, SizeClass, radius, space, typography};
use winisland_render::{Painter, Point, Radius, Rect, Vec2};

use crate::ui::components::card::{CardLayout, draw_card};
use crate::ui::components::dock::draw_dock;
use crate::ui::components::semantics::{InteractionState, Role, SemanticNode};
use crate::ui::components::text::{Align, draw_text};
use layout::DashboardLayout;
use sample::{CardKind, HOME_CARDS, SampleCard};
use std::time::{Duration, Instant};
use winisland_core::focus::{
    FocusEvent, FocusTimer, FocusView, Phase, PomodoroConfig, TimerKind, TimerState,
};

/// What the pointer is over on the dashboard page.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DashboardHit {
    Card(usize),
    Destination(Destination),
}

struct PageState {
    active: Destination,
    hover: Option<DashboardHit>,
    /// The first live module, restored from disk on first use.
    focus: FocusTimer,
    /// A phase that ended while the app was closed, reported by the next [`poll`].
    restored_event: Option<FocusEvent>,
}

thread_local! {
    static PAGE: RefCell<PageState> = RefCell::new({
        let (focus, restored_event) =
            crate::core::focus_store::load(TimerKind::Pomodoro(PomodoroConfig::default()));
        PageState {
            active: Destination::Home,
            hover: None,
            focus,
            restored_event,
        }
    });
}

/// Advances the Focus timer. Called every frame, so phases finish and notify even while
/// the dashboard is hidden. Returns the event once.
pub fn poll(now: Instant) -> Option<FocusEvent> {
    let event = PAGE.with(|page| {
        let mut page = page.borrow_mut();
        let restored = page.restored_event.take();
        let ticked = page.focus.tick(now);
        if ticked.is_some() {
            crate::core::focus_store::save(&page.focus);
        }
        ticked.or(restored)
    })?;
    notify_phase_finished(event);
    Some(event)
}

fn notify_phase_finished(event: FocusEvent) {
    let FocusEvent::PhaseFinished { next, .. } = event;
    let message_key = match next {
        Some(Phase::ShortBreak) => "focus_notify_short_break",
        Some(Phase::LongBreak) => "focus_notify_long_break",
        Some(Phase::Work) => "focus_notify_work",
        Some(Phase::Countdown | Phase::Stopwatch) | None => "focus_notify_done",
    };
    crate::platform::notify().show_toast(&tr("focus_notify_title"), &tr(message_key));
}

/// Clock text for a duration. Remaining time rounds up so 0:00 appears only at the end.
fn format_clock(duration: Duration, round_up: bool) -> String {
    let millis = duration.as_millis();
    let seconds = if round_up {
        millis.div_ceil(1000)
    } else {
        millis / 1000
    };
    let (hours, minutes, seconds) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

thread_local! {
    /// Today's active time, or `None` while screen time tracking is off.
    static SCREEN_TIME_TOTAL: Cell<Option<Duration>> = const { Cell::new(None) };
}

/// Publishes today's screen time for the Home card. `None` means tracking is off.
pub fn set_screen_time_total(total: Option<Duration>) {
    SCREEN_TIME_TOTAL.with(|cell| cell.set(total));
}

/// Rows shown on the Clipboard page; more items stay in history but are not listed.
const MAX_CLIPBOARD_ROWS: usize = 50;
const MAX_ROW_CHARS: usize = 120;

#[derive(Default)]
struct ClipboardView {
    synced: bool,
    enabled: bool,
    revision: u64,
    count: usize,
    rows: Vec<(String, String)>,
}

thread_local! {
    static CLIPBOARD_VIEW: RefCell<ClipboardView> = RefCell::new(ClipboardView::default());
}

fn clipboard_row(item: &winisland_core::clipboard::ClipboardItem) -> (String, String) {
    use winisland_core::clipboard::ClipboardBody;
    let title = match &item.body {
        ClipboardBody::Text(text) => text
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .unwrap_or_default()
            .chars()
            .take(MAX_ROW_CHARS)
            .collect(),
        ClipboardBody::Link(url) => url.chars().take(MAX_ROW_CHARS).collect(),
        ClipboardBody::Image { width, height, .. } => {
            winisland_core::i18n::tr_args("clipboard_image", &[&format!("{width}×{height}")])
        }
        ClipboardBody::Files(files) => match files.as_slice() {
            [file] => file
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| file.display().to_string()),
            _ => winisland_core::i18n::tr_args("clipboard_files", &[&files.len().to_string()]),
        },
    };
    let subtitle = item.source_app.clone().unwrap_or_default();
    (title, subtitle)
}

/// Publishes clipboard history for the dashboard. Rebuilds rows only when it changed.
pub fn sync_clipboard(
    enabled: bool,
    revision: u64,
    items: &[winisland_core::clipboard::ClipboardItem],
) {
    CLIPBOARD_VIEW.with(|view| {
        let mut view = view.borrow_mut();
        if view.synced && view.enabled == enabled && view.revision == revision {
            return;
        }
        view.synced = true;
        view.enabled = enabled;
        view.revision = revision;
        view.count = items.len();
        view.rows = items
            .iter()
            .take(MAX_CLIPBOARD_ROWS)
            .map(clipboard_row)
            .collect();
    });
}

fn draw_clipboard_page(painter: Painter<'_>, grid: Rect, colors: &SemanticColors) {
    use crate::ui::components::list_row::{ListRowLayout, ListRowSpec, draw_list_row};
    use winisland_render::tokens::Density;
    CLIPBOARD_VIEW.with(|view| {
        let view = view.borrow();
        let empty = if !view.enabled {
            Some("clipboard_off_hint")
        } else if view.rows.is_empty() {
            Some("clipboard_empty_hint")
        } else {
            None
        };
        if let Some(key) = empty {
            draw_text(
                painter,
                &tr(key),
                typography::BODY,
                grid,
                Align::Center,
                colors.text_secondary,
            );
            return;
        }
        let spec = ListRowSpec {
            density: Density::Compact,
            has_leading: false,
            has_subtitle: true,
            trailing_width: 0.0,
        };
        let mut top = grid.top;
        for (title, subtitle) in &view.rows {
            if top >= grid.bottom {
                break;
            }
            let row = ListRowLayout::new(grid.left, top, grid.width(), spec);
            let subtitle = (!subtitle.is_empty()).then_some(subtitle.as_str());
            draw_list_row(
                painter,
                &row,
                title,
                subtitle,
                colors,
                InteractionState::default(),
            );
            top = row.bounds.bottom;
        }
    });
}

/// Modules on the More page: everything in [`Destination::MORE_ITEMS`] not in the dock.
fn more_entries(layout: &DashboardLayout) -> Vec<Destination> {
    Destination::MORE_ITEMS
        .into_iter()
        .filter(|destination| !layout.destinations.contains(destination))
        .collect()
}

/// Row geometry shared by list pages, so drawing and hit testing agree.
fn list_row(
    grid: Rect,
    index: usize,
    has_subtitle: bool,
) -> crate::ui::components::list_row::ListRowLayout {
    use crate::ui::components::list_row::{ListRowLayout, ListRowSpec};
    use winisland_render::tokens::Density;
    let spec = ListRowSpec {
        density: Density::Compact,
        has_leading: false,
        has_subtitle,
        trailing_width: 0.0,
    };
    let top = grid.top + index as f32 * spec.density.list_row_height();
    ListRowLayout::new(grid.left, top, grid.width(), spec)
}

fn hit_more_row(layout: &DashboardLayout, point: Point) -> Option<Destination> {
    if !layout.grid.contains(point) {
        return None;
    }
    more_entries(layout)
        .into_iter()
        .enumerate()
        .find(|(index, _)| list_row(layout.grid, *index, false).bounds.contains(point))
        .map(|(_, destination)| destination)
}

fn draw_more_page(
    painter: Painter<'_>,
    layout: &DashboardLayout,
    hovered_row: Option<usize>,
    colors: &SemanticColors,
) {
    use crate::ui::components::list_row::draw_list_row;
    for (index, destination) in more_entries(layout).into_iter().enumerate() {
        let row = list_row(layout.grid, index, false);
        if row.bounds.top >= layout.grid.bottom {
            break;
        }
        let state = InteractionState {
            hovered: hovered_row == Some(index),
            ..InteractionState::default()
        };
        draw_list_row(
            painter,
            &row,
            &tr(destination.label_key()),
            None,
            colors,
            state,
        );
    }
}

const NOTES_FILE: &str = "notes.json";
const MAX_NOTES_FILE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_NOTE_ROWS: usize = 50;

thread_local! {
    /// Note titles and checklist progress, read once from `notes.json`.
    static NOTE_ROWS: Vec<(String, String)> = load_note_rows();
}

fn load_note_rows() -> Vec<(String, String)> {
    use winisland_core::notes::{NoteStore, NotesSnapshot};
    let store =
        match crate::core::json_store::read::<NotesSnapshot>(NOTES_FILE, MAX_NOTES_FILE_BYTES) {
            Ok(Some(snapshot)) => NoteStore::restore(snapshot).unwrap_or_else(|error| {
                log::warn!("Ignoring saved notes: {error}");
                NoteStore::new()
            }),
            Ok(None) => NoteStore::new(),
            Err(error) => {
                log::warn!("Ignoring saved notes: {error}");
                NoteStore::new()
            }
        };
    store
        .list()
        .into_iter()
        .take(MAX_NOTE_ROWS)
        .map(|note| {
            let title = if note.title().is_empty() {
                tr("notes_untitled")
            } else {
                note.title().to_string()
            };
            let checklist = note.checklist();
            let detail = if checklist.is_empty() {
                note.tags().join(", ")
            } else {
                let done = checklist.iter().filter(|item| item.done).count();
                format!("{done}/{}", checklist.len())
            };
            (title, detail)
        })
        .collect()
}

fn draw_notes_page(painter: Painter<'_>, grid: Rect, colors: &SemanticColors) {
    use crate::ui::components::list_row::draw_list_row;
    NOTE_ROWS.with(|rows| {
        if rows.is_empty() {
            draw_text(
                painter,
                &tr("notes_empty_hint"),
                typography::BODY,
                grid,
                Align::Center,
                colors.text_secondary,
            );
            return;
        }
        for (index, (title, detail)) in rows.iter().enumerate() {
            let row = list_row(grid, index, true);
            if row.bounds.top >= grid.bottom {
                break;
            }
            let detail = (!detail.is_empty()).then_some(detail.as_str());
            draw_list_row(
                painter,
                &row,
                title,
                detail,
                colors,
                InteractionState::default(),
            );
        }
    });
}

thread_local! {
    static WEATHER: RefCell<Option<crate::core::weather_service::WeatherSummary>> =
        const { RefCell::new(None) };
}

/// Publishes the forecast for the Home card. `None` hides it behind the "off" state.
pub fn set_weather(summary: Option<crate::core::weather_service::WeatherSummary>) {
    WEATHER.with(|cell| {
        let mut current = cell.borrow_mut();
        if *current != summary {
            *current = summary;
        }
    });
}

fn card_text(card: &SampleCard, focus: Option<&FocusView>) -> (String, String) {
    if card.kind == CardKind::Weather {
        return WEATHER.with(|cell| match cell.borrow().as_ref() {
            Some(summary) => {
                let mut detail = format!("{} · {}", tr(summary.condition_key), summary.place);
                if summary.stale {
                    detail.push_str(" · ");
                    detail.push_str(&tr("weather_stale"));
                }
                (summary.temperature.clone(), detail)
            }
            None => (tr("card_off"), tr("weather_turn_on")),
        });
    }
    if card.kind == CardKind::Notes {
        return NOTE_ROWS.with(|rows| (rows.len().to_string(), tr("card_notes_count")));
    }
    if card.kind == CardKind::Clipboard {
        return CLIPBOARD_VIEW.with(|view| {
            let view = view.borrow();
            if view.enabled || view.count > 0 {
                (view.count.to_string(), tr("card_clipboard_items"))
            } else {
                (tr("card_off"), tr("clipboard_turn_on"))
            }
        });
    }
    if card.kind == CardKind::ScreenTime {
        return match SCREEN_TIME_TOTAL.with(Cell::get) {
            Some(total) => {
                let minutes = total.as_secs() / 60;
                (
                    format!("{}:{:02}", minutes / 60, minutes % 60),
                    tr(card.detail_key),
                )
            }
            None => (tr("card_off"), tr("screen_time_turn_on")),
        };
    }
    match (card.kind, focus) {
        (CardKind::Focus, Some(view)) => {
            let value = match view.remaining {
                Some(remaining) => format_clock(remaining, true),
                None => format_clock(view.elapsed, false),
            };
            let detail = match view.state {
                TimerState::Running => tr(view.label_key),
                TimerState::Paused => tr("focus_paused"),
                TimerState::Idle | TimerState::Finished => tr("focus_click_to_start"),
            };
            (value, detail)
        }
        _ => (card.value.to_string(), tr(card.detail_key)),
    }
}

/// Starts, pauses or resumes the Focus timer.
fn toggle_focus(timer: &mut FocusTimer, now: Instant) {
    match timer.state() {
        TimerState::Idle => {
            timer.start(now);
        }
        TimerState::Running => {
            timer.pause(now);
        }
        TimerState::Paused => {
            timer.resume(now);
        }
        TimerState::Finished => {
            timer.reset();
            timer.start(now);
        }
    }
}

/// `true` while a live value changes every second and the page must keep redrawing.
/// Stops and resets the Focus timer and deletes its saved state.
pub fn reset_focus() {
    PAGE.with(|page| page.borrow_mut().focus.reset());
    if let Err(error) = crate::core::json_store::remove(crate::core::focus_store::FILE_NAME) {
        log::warn!("Cannot delete saved Focus timer: {error}");
    }
}

pub fn is_animating() -> bool {
    PAGE.with(|page| page.borrow().focus.state() == TimerState::Running)
}

/// Text for the compact Focus widget and whether the timer is running.
pub fn focus_compact_text() -> (String, bool) {
    PAGE.with(|page| {
        let page = page.borrow();
        let view = page.focus.view(Instant::now());
        match view.state {
            TimerState::Running | TimerState::Paused => {
                let text = match view.remaining {
                    Some(remaining) => format_clock(remaining, true),
                    None => format_clock(view.elapsed, false),
                };
                (text, view.state == TimerState::Running)
            }
            TimerState::Idle | TimerState::Finished => (tr("dashboard_focus"), false),
        }
    })
}

/// How soon the compact Focus widget needs redrawing; `None` while the timer is stopped.
pub fn focus_refresh_delay() -> Option<Duration> {
    is_animating().then_some(Duration::from_millis(250))
}

/// Lays the page out in logical pixels. `scale` converts logical to device pixels.
fn page_layout(width: f32, height: f32, scale: f32) -> DashboardLayout {
    let scale = scale.max(f32::EPSILON);
    let (width, height) = (width / scale, height / scale);
    // The island has no monitor work area here, so the class follows the expanded width.
    let size_class = if width >= SizeClass::Standard.width_range().0 {
        SizeClass::Standard
    } else {
        SizeClass::Compact
    };
    let spans = HOME_CARDS.map(|card| card.column_span);
    DashboardLayout::new(Rect::from_xywh(0.0, 0.0, width, height), size_class, &spans)
}

fn cards_for(active: Destination) -> &'static [SampleCard] {
    if active == Destination::Home {
        &HOME_CARDS
    } else {
        &[]
    }
}

/// Hit test in device pixels for a page drawn at (`ox`, `oy`).
pub fn hit_test(
    ox: f32,
    oy: f32,
    width: f32,
    height: f32,
    scale: f32,
    point: Point,
) -> Option<DashboardHit> {
    let layout = page_layout(width, height, scale);
    let scale = scale.max(f32::EPSILON);
    let local = Point::new((point.x - ox) / scale, (point.y - oy) / scale);
    if let Some(destination) = layout.hit_destination(local) {
        return Some(DashboardHit::Destination(destination));
    }
    let active = PAGE.with(|page| page.borrow().active);
    if active == Destination::More {
        return hit_more_row(&layout, local).map(DashboardHit::Destination);
    }
    layout
        .hit_card(local)
        .filter(|index| *index < cards_for(active).len())
        .map(DashboardHit::Card)
}

/// Returns `true` when the hover target changed and the page needs a redraw.
pub fn set_hover(hit: Option<DashboardHit>) -> bool {
    PAGE.with(|page| {
        let mut page = page.borrow_mut();
        let changed = page.hover != hit;
        page.hover = hit;
        changed
    })
}

/// Applies a click. The Focus card toggles the timer; other cards have no action until
/// their modules exist.
pub fn apply(hit: DashboardHit) {
    PAGE.with(|page| {
        let mut page = page.borrow_mut();
        match hit {
            DashboardHit::Destination(destination) => page.active = destination,
            DashboardHit::Card(index) => {
                let is_focus = cards_for(page.active)
                    .get(index)
                    .is_some_and(|card| card.kind == CardKind::Focus);
                if is_focus {
                    toggle_focus(&mut page.focus, Instant::now());
                    crate::core::focus_store::save(&page.focus);
                }
            }
        }
    });
}

/// Draws the page at (`ox`, `oy`) with size in device pixels.
pub fn draw_page(
    painter: Painter<'_>,
    ox: f32,
    oy: f32,
    width: f32,
    height: f32,
    alpha: u8,
    scale: f32,
) {
    if alpha == 0 {
        return;
    }
    let layout = page_layout(width, height, scale);
    let now = Instant::now();
    // Ticking happens in `poll` from the frame loop; drawing only reads.
    let (active, hover, focus) = PAGE.with(|page| {
        let page = page.borrow();
        (page.active, page.hover, page.focus.view(now))
    });
    let interaction = DashboardInteraction {
        hovered_card: match hover {
            Some(DashboardHit::Card(index)) => Some(index),
            _ => None,
        },
        hovered_dock: match hover {
            Some(DashboardHit::Destination(destination)) => layout
                .destinations
                .iter()
                .position(|candidate| *candidate == destination),
            _ => None,
        },
        hovered_row: match hover {
            Some(DashboardHit::Destination(destination)) if active == Destination::More => {
                more_entries(&layout)
                    .iter()
                    .position(|candidate| *candidate == destination)
            }
            _ => None,
        },
        ..DashboardInteraction::default()
    };
    let colors = SemanticColors::for_scheme(ColorScheme::Dark);
    painter.save_alpha(alpha);
    painter.translate(Vec2::new(ox, oy));
    painter.scale(Vec2::new(scale, scale));
    draw_dashboard(DashboardParams {
        painter,
        layout: &layout,
        colors: &colors,
        active,
        cards: cards_for(active),
        focus: Some(focus),
        interaction,
    });
    painter.restore();
}

/// Primary destinations, matching the first-run modules in the design guide.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Destination {
    Home,
    Media,
    Clipboard,
    Shelf,
    Focus,
    More,
    /// Reached from More; not a dock destination.
    Notes,
    /// World clocks; reached from More.
    Clock,
}

impl Destination {
    pub const ALL: [Self; 6] = [
        Self::Home,
        Self::Media,
        Self::Clipboard,
        Self::Shelf,
        Self::Focus,
        Self::More,
    ];
    /// Shown in the compact size class; the rest move into More.
    pub const COMPACT: [Self; 3] = [Self::Home, Self::Focus, Self::More];
    /// Modules listed on the More page when they are not already in the dock.
    pub const MORE_ITEMS: [Self; 6] = [
        Self::Media,
        Self::Clipboard,
        Self::Shelf,
        Self::Focus,
        Self::Notes,
        Self::Clock,
    ];

    pub const fn label_key(self) -> &'static str {
        match self {
            Self::Home => "dashboard_home",
            Self::Media => "dashboard_media",
            Self::Clipboard => "dashboard_clipboard",
            Self::Shelf => "dashboard_shelf",
            Self::Focus => "dashboard_focus",
            Self::More => "dashboard_more",
            Self::Notes => "dashboard_notes",
            Self::Clock => "dashboard_clock",
        }
    }
}

/// Per-frame interaction state owned by the caller.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct DashboardInteraction {
    pub hovered_card: Option<usize>,
    pub focused_card: Option<usize>,
    pub hovered_dock: Option<usize>,
    pub focused_dock: Option<usize>,
    /// Hovered row on list pages such as More.
    pub hovered_row: Option<usize>,
}

pub struct DashboardParams<'a> {
    pub painter: Painter<'a>,
    pub layout: &'a DashboardLayout,
    pub colors: &'a SemanticColors,
    pub active: Destination,
    pub cards: &'a [SampleCard],
    /// Live Focus timer state; `None` shows the sample value.
    pub focus: Option<FocusView>,
    pub interaction: DashboardInteraction,
}

pub fn draw_dashboard(params: DashboardParams<'_>) {
    let DashboardParams {
        painter,
        layout,
        colors,
        active,
        cards,
        focus,
        interaction,
    } = params;
    painter.fill_round_rect(
        layout.bounds,
        Radius::uniform(radius::WORKSPACE),
        colors.surface_base,
    );
    draw_header(painter, layout.header, active, colors);

    let save = painter.save();
    painter.clip_rect(layout.grid);
    for (index, (rect, card)) in layout.cards.iter().zip(cards).enumerate() {
        let card_layout = CardLayout::new(*rect, 0);
        let state = InteractionState {
            hovered: interaction.hovered_card == Some(index),
            focused: interaction.focused_card == Some(index),
            ..InteractionState::default()
        };
        draw_card(painter, &card_layout, &tr(card.title_key), colors, state);
        let (value, detail) = card_text(card, focus.as_ref());
        draw_card_value(painter, &card_layout, &value, &detail, colors);
    }
    if active == Destination::Clipboard {
        draw_clipboard_page(painter, layout.grid, colors);
    } else if active == Destination::More {
        draw_more_page(painter, layout, interaction.hovered_row, colors);
    } else if active == Destination::Notes {
        draw_notes_page(painter, layout.grid, colors);
    } else if active == Destination::Clock {
        draw_clock_page(painter, layout.grid, colors);
    } else if cards.is_empty() {
        draw_text(
            painter,
            &tr("dashboard_not_ready"),
            typography::BODY,
            layout.grid,
            Align::Center,
            colors.text_secondary,
        );
    }
    painter.restore_to(save);

    let labels = dock_labels(layout);
    let label_refs: Vec<&str> = labels.iter().map(String::as_str).collect();
    let active_index = layout
        .destinations
        .iter()
        .position(|destination| *destination == active)
        .unwrap_or(0);
    draw_dock(
        painter,
        &layout.dock,
        &label_refs,
        active_index,
        interaction.hovered_dock,
        interaction.focused_dock,
        colors,
    );
}

fn draw_header(painter: Painter<'_>, header: Rect, active: Destination, colors: &SemanticColors) {
    let badge = tr("dashboard_sample_badge");
    let badge_width = (header.width() * 0.4).min(160.0);
    let title = Rect::from_ltrb(
        header.left,
        header.top,
        header.right - badge_width,
        header.bottom,
    );
    draw_text(
        painter,
        &tr(active.label_key()),
        typography::PAGE_TITLE,
        title,
        Align::Start,
        colors.text_primary,
    );
    let badge_rect = Rect::from_ltrb(
        header.right - badge_width,
        header.center_y() - 10.0,
        header.right,
        header.center_y() + 10.0,
    );
    painter.fill_round_rect(badge_rect, Radius::uniform(10.0), colors.accent_soft);
    draw_text(
        painter,
        &badge,
        typography::CAPTION,
        badge_rect.inset(space::S1),
        Align::Center,
        colors.text_primary,
    );
}

fn draw_card_value(
    painter: Painter<'_>,
    card_layout: &CardLayout,
    value: &str,
    detail: &str,
    colors: &SemanticColors,
) {
    let content = card_layout.content;
    let value_h = typography::NUMERIC.size + space::S2;
    let value_rect = Rect::from_ltrb(
        content.left,
        content.top,
        content.right,
        (content.top + value_h).min(content.bottom),
    );
    draw_text(
        painter,
        value,
        typography::NUMERIC,
        value_rect,
        Align::Start,
        colors.text_primary,
    );
    let detail_rect = Rect::from_ltrb(
        content.left,
        value_rect.bottom,
        content.right,
        (value_rect.bottom + typography::CAPTION.size + space::S2).min(content.bottom),
    );
    // Short cards show only the value rather than clipping the detail line.
    if detail_rect.height() < typography::CAPTION.size {
        return;
    }
    draw_text(
        painter,
        detail,
        typography::CAPTION,
        detail_rect,
        Align::Start,
        colors.text_secondary,
    );
}

fn dock_labels(layout: &DashboardLayout) -> Vec<String> {
    layout
        .destinations
        .iter()
        .map(|destination| tr(destination.label_key()))
        .collect()
}

pub fn semantics(
    layout: &DashboardLayout,
    active: Destination,
    cards: &[SampleCard],
    interaction: DashboardInteraction,
) -> SemanticNode {
    let card_nodes = layout
        .cards
        .iter()
        .zip(cards)
        .enumerate()
        .map(|(index, (rect, card))| {
            SemanticNode::new(
                Role::Group,
                format!("{} {}", tr(card.title_key), card.value),
                *rect,
                InteractionState {
                    focused: interaction.focused_card == Some(index),
                    ..InteractionState::default()
                },
            )
        })
        .collect();
    let labels = dock_labels(layout);
    let label_refs: Vec<&str> = labels.iter().map(String::as_str).collect();
    let active_index = layout
        .destinations
        .iter()
        .position(|destination| *destination == active)
        .unwrap_or(0);
    let dock = layout
        .dock
        .semantics(&label_refs, active_index, interaction.focused_dock);
    SemanticNode::new(
        Role::Group,
        tr(active.label_key()),
        layout.bounds,
        InteractionState::default(),
    )
    .with_children(vec![
        SemanticNode::new(Role::Group, "", layout.grid, InteractionState::default())
            .with_children(card_nodes),
        dock,
    ])
}

thread_local! {
    static WORLD_CLOCKS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

/// Publishes the configured world clock zone keys.
pub fn set_world_clocks(keys: &[String]) {
    WORLD_CLOCKS.with(|cell| {
        let mut current = cell.borrow_mut();
        if current.as_slice() != keys {
            *current = keys.to_vec();
        }
    });
}

fn draw_clock_page(painter: Painter<'_>, grid: Rect, colors: &SemanticColors) {
    use crate::ui::components::list_row::draw_list_row;
    WORLD_CLOCKS.with(|cell| {
        let keys = cell.borrow();
        if keys.is_empty() {
            draw_text(
                painter,
                &tr("world_clock_empty"),
                typography::BODY,
                grid,
                Align::Center,
                colors.text_secondary,
            );
            return;
        }
        for (index, key) in keys.iter().enumerate() {
            let row = list_row(grid, index, true);
            if row.bounds.top >= grid.bottom {
                break;
            }
            let time = winisland_platform_windows::now_in_zone(key)
                .map(|now| format!("{:02}:{:02}", now.hour, now.minute))
                .unwrap_or_else(|| "—".to_string());
            draw_list_row(
                painter,
                &row,
                &time,
                Some(key.as_str()),
                colors,
                InteractionState::default(),
            );
        }
    });
}
