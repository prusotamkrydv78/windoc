//! Dashboard shell: header, responsive card grid and bottom dock (roadmap step 5).
//!
//! Content comes from deterministic sample data until modules provide snapshots. The
//! header always labels it as sample data.

// `semantics` and grid scrolling (`overflows`, `content_height`) wait for the UI Automation
// provider and scroll support.
#![allow(dead_code)]

pub mod editor;
pub mod layout;
pub mod palette;
pub mod sample;
pub mod system_page;
pub mod tabs;

use std::cell::{Cell, RefCell};

use winisland_core::i18n::tr;
use winisland_render::tokens::{SemanticColors, SizeClass, radius, space, typography};
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
    /// A control inside a tab; the id is private to that tab (see `tabs`).
    Tab(u16),
    /// System tab: the volume slider at a percentage.
    SystemVolume(u8),
    /// System tab: the mute button.
    SystemMute,
    Card(usize),
    Destination(Destination),
    /// Now playing controls: 0 previous, 1 play or pause, 2 next.
    MediaButton(u8),
    /// A click on the progress bar, as thousandths of the track.
    MediaSeek(u16),
    /// Index of a row on the Shelf page.
    ShelfEntry(usize),
    /// The header outside its buttons (the search field while the palette is open).
    Header,
    /// The header's search button, which opens the command palette.
    Search,
    /// A workspace tab in the header.
    Workspace(u8),
    /// The "+" after the workspace tabs.
    AddWorkspace,
    /// Toggles arrange mode (choose which modules the workspace shows).
    Arrange,
    /// Arrange mode, on a Home card: move it one place earlier.
    CardMove(usize),
    /// Arrange mode, on a Home card: toggle one or two columns wide.
    CardResize(usize),
    /// Arrange mode, on a Home card: remove it from this workspace.
    CardRemove(usize),
    /// Focus page buttons: 0 reset, 1 play or pause, 2 skip.
    FocusButton(u8),
    /// The header's settings button.
    OpenSettings,
    /// The header's close button.
    Close,
    /// Row on the Notes page: 0 is "New note".
    NoteRow(usize),
    /// Row on the Links page.
    LinkRow(usize),
    /// Row on the Clipboard page; clicking copies it again.
    ClipRow(usize),
}

struct PageState {
    active: Destination,
    hover: Option<DashboardHit>,
    /// The first live module, restored from disk on first use.
    focus: FocusTimer,
    /// A phase that ended while the app was closed, reported by the next [`poll`].
    restored_event: Option<FocusEvent>,
    /// When the current page was shown, for the staggered entrance.
    shown_at: Instant,
}

thread_local! {
    static PAGE: RefCell<PageState> = RefCell::new({
        let (focus, restored_event) =
            crate::core::focus_store::load(TimerKind::Pomodoro(PomodoroConfig::default()));
        PageState {
            shown_at: Instant::now(),
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
    tabs::poll_all(now);
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

/// The translated message announcing what follows a finished Focus phase.
pub fn phase_finished_message(event: FocusEvent) -> String {
    let FocusEvent::PhaseFinished { next, .. } = event;
    tr(match next {
        Some(Phase::ShortBreak) => "focus_notify_short_break",
        Some(Phase::LongBreak) => "focus_notify_long_break",
        Some(Phase::Work) => "focus_notify_work",
        Some(Phase::Countdown | Phase::Stopwatch) | None => "focus_notify_done",
    })
}

fn notify_phase_finished(event: FocusEvent) {
    crate::platform::notify().show_toast(&tr("focus_notify_title"), &phase_finished_message(event));
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
    /// Longer text for the preview pane, aligned with `rows`.
    previews: Vec<String>,
}

thread_local! {
    static CLIPBOARD_COPY: Cell<Option<usize>> = const { Cell::new(None) };
    static MEDIA_REQUEST: Cell<Option<u8>> = const { Cell::new(None) };
    static MEDIA_SEEK: Cell<Option<u16>> = const { Cell::new(None) };
    /// The current track, for the Now playing page.
    static MEDIA: RefCell<Option<crate::core::smtc::MediaInfo>> = const { RefCell::new(None) };
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
        view.previews = items
            .iter()
            .take(MAX_CLIPBOARD_ROWS)
            .map(clipboard_preview)
            .collect();
    });
}

fn draw_clipboard_page(
    painter: Painter<'_>,
    grid: Rect,
    hovered_row: Option<usize>,
    colors: &SemanticColors,
) {
    use crate::ui::components::list_row::{ListRowLayout, ListRowSpec};
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
        let (list, preview) = clipboard_split(grid);
        let mut top = list.top;
        for (index, (title, subtitle)) in view.rows.iter().enumerate() {
            if top >= list.bottom {
                break;
            }
            let row = ListRowLayout::new(list.left, top, list.width(), spec);
            let subtitle = (!subtitle.is_empty()).then_some(subtitle.as_str());
            draw_row(
                painter,
                &row,
                title,
                subtitle,
                colors,
                InteractionState {
                    hovered: hovered_row == Some(index),
                    selected: preview.is_some() && hovered_row.unwrap_or(0) == index,
                    ..InteractionState::default()
                },
            );
            top = row.bounds.bottom;
        }
        if let Some(pane) = preview {
            let index = hovered_row
                .unwrap_or(0)
                .min(view.previews.len().saturating_sub(1));
            if let Some(text) = view.previews.get(index) {
                draw_clipboard_preview(painter, pane, text, colors);
            }
        }
    });
}

/// Modules on the More page: everything in [`Destination::MORE_ITEMS`] not in the dock.
fn more_entries(layout: &DashboardLayout) -> Vec<Destination> {
    Destination::MORE_ITEMS
        .into_iter()
        .filter(|destination| !layout.destinations.contains(destination))
        .filter(|destination| {
            HIDDEN_MODULES.with(|cell| {
                !cell
                    .borrow()
                    .iter()
                    .any(|key| key == destination.label_key())
            })
        })
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
        has_leading: ROW_ICON.with(Cell::get).is_some(),
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
    use crate::ui::components::list_row::{ListRowLayout, ListRowSpec};
    use winisland_render::tokens::Density;
    let spec = ListRowSpec {
        density: Density::Compact,
        has_leading: true,
        has_subtitle: false,
        trailing_width: 0.0,
    };
    for (index, destination) in more_entries(layout).into_iter().enumerate() {
        // Same height as `list_row`, so hit testing in `hit_more_row` still matches.
        let row = ListRowLayout::new(
            layout.grid.left,
            layout.grid.top + index as f32 * spec.density.list_row_height(),
            layout.grid.width(),
            spec,
        );
        if row.bounds.top >= layout.grid.bottom {
            break;
        }
        let hovered = hovered_row == Some(index);
        let state = InteractionState {
            hovered,
            ..InteractionState::default()
        };
        draw_row(
            painter,
            &row,
            &tr(destination.label_key()),
            None,
            colors,
            state,
        );
        if let Some(leading) = row.leading {
            crate::ui::components::icon::draw_icon(
                painter,
                destination.icon(),
                leading.inset(3.0),
                if hovered {
                    colors.accent_primary
                } else {
                    colors.text_secondary
                },
            );
        }
    }
}

const NOTES_FILE: &str = "notes.json";
const MAX_NOTES_FILE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_NOTE_ROWS: usize = 50;

thread_local! {
    /// Note titles and checklist progress, read once from `notes.json`.
    static NOTE_ROWS: RefCell<Vec<(u64, String, String)>> = RefCell::new(note_rows(&load_notes()));
}

fn load_notes() -> winisland_core::notes::NoteStore {
    use winisland_core::notes::{NoteStore, NotesSnapshot};
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
    }
}

/// Saves a quick note from the command palette and refreshes the Notes page.
pub fn add_quick_note(text: &str) {
    let mut store = load_notes();
    if let Err(error) = store.create(text, std::time::SystemTime::now()) {
        log::warn!("Cannot add note: {error}");
        return;
    }
    if let Err(error) = crate::core::json_store::write(NOTES_FILE, &store.snapshot()) {
        log::warn!("Cannot save notes: {error}");
        return;
    }
    NOTE_ROWS.with(|cell| *cell.borrow_mut() = note_rows(&store));
}

/// Time left in a running timed Focus phase, for the phone's countdown.
pub fn focus_remaining(now: Instant) -> Option<Duration> {
    PAGE.with(|page| {
        let page = page.borrow();
        if page.focus.state() != TimerState::Running {
            return None;
        }
        let length = page.focus.phase_length()?;
        Some(length.saturating_sub(page.focus.elapsed(now)))
    })
}

/// Switches the dashboard to `destination`.
pub fn show(destination: Destination) {
    PAGE.with(|page| {
        let mut page = page.borrow_mut();
        if page.active != destination {
            page.active = destination;
            page.shown_at = Instant::now();
        }
    });
}

fn start_timer(kind: TimerKind) {
    PAGE.with(|page| {
        let mut page = page.borrow_mut();
        page.focus = FocusTimer::new(kind);
        page.focus.start(Instant::now());
        crate::core::focus_store::save(&page.focus);
        page.active = Destination::Home;
    });
}

pub fn start_countdown(duration: Duration) {
    start_timer(TimerKind::Countdown(duration));
}

pub fn start_pomodoro() {
    start_timer(TimerKind::Pomodoro(PomodoroConfig::default()));
}

pub fn start_stopwatch() {
    start_timer(TimerKind::Stopwatch);
}

fn note_rows(store: &winisland_core::notes::NoteStore) -> Vec<(u64, String, String)> {
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
            (note.id(), title, detail)
        })
        .collect()
}

fn draw_notes_page(
    painter: Painter<'_>,
    grid: Rect,
    hovered_row: Option<usize>,
    colors: &SemanticColors,
) {
    if editor::is_open() {
        editor::draw(painter, grid, colors);
        return;
    }
    NOTE_ROWS.with(|cell| {
        let rows = cell.borrow();
        let new_row = list_row(grid, 0, true);
        draw_row(
            painter,
            &new_row,
            &tr("notes_new"),
            Some(tr("notes_new_detail").as_str()),
            colors,
            InteractionState {
                hovered: hovered_row == Some(0),
                ..InteractionState::default()
            },
        );
        for (index, (_, title, detail)) in rows.iter().enumerate() {
            let row = list_row(grid, index + 1, true);
            if row.bounds.top >= grid.bottom {
                break;
            }
            let detail = (!detail.is_empty()).then_some(detail.as_str());
            draw_row(
                painter,
                &row,
                title,
                detail,
                colors,
                InteractionState {
                    hovered: hovered_row == Some(index + 1),
                    ..InteractionState::default()
                },
            );
        }
    });
}

/// Row 0 is "New note"; row n is the (n-1)th note.
fn hit_note_row(layout: &DashboardLayout, point: Point) -> Option<usize> {
    if !layout.grid.contains(point) || editor::is_open() {
        return None;
    }
    let count = NOTE_ROWS.with(|cell| cell.borrow().len()) + 1;
    (0..count).find(|index| list_row(layout.grid, *index, true).bounds.contains(point))
}

fn open_note_row(index: usize) {
    if index == 0 {
        editor::open(None, "");
        return;
    }
    let id = NOTE_ROWS.with(|cell| cell.borrow().get(index - 1).map(|row| row.0));
    if let Some(id) = id {
        let store = load_notes();
        if let Some(note) = store.note(id) {
            editor::open(Some(id), note.body());
        }
    }
}

/// Creates or updates a note from the editor and refreshes the Notes page.
pub(crate) fn save_note(id: Option<u64>, body: &str) {
    let mut store = load_notes();
    let now = std::time::SystemTime::now();
    let result = match id {
        Some(id) => store.update_body(id, body, now).map(|_| ()),
        None if body.trim().is_empty() => return,
        None => store.create(body, now).map(|_| ()),
    };
    if let Err(error) = result {
        log::warn!("Cannot save note: {error}");
        return;
    }
    if let Err(error) = crate::core::json_store::write(NOTES_FILE, &store.snapshot()) {
        log::warn!("Cannot save notes: {error}");
        return;
    }
    NOTE_ROWS.with(|cell| *cell.borrow_mut() = note_rows(&store));
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
            None => (String::new(), tr("weather_turn_on")),
        });
    }
    if card.kind == CardKind::Notes {
        return NOTE_ROWS.with(|cell| match cell.borrow().len() {
            0 => (String::new(), tr("notes_empty_hint")),
            count => (count.to_string(), tr("card_notes_count")),
        });
    }
    if card.kind == CardKind::Clipboard {
        return CLIPBOARD_VIEW.with(|view| {
            let view = view.borrow();
            if view.enabled || view.count > 0 {
                (view.count.to_string(), tr("card_clipboard_items"))
            } else {
                (String::new(), tr("clipboard_turn_on"))
            }
        });
    }
    if card.kind == CardKind::ScreenTime {
        return match SCREEN_TIME_TOTAL.with(Cell::get) {
            Some(total) => {
                let minutes = total.as_secs() / 60;
                (
                    if minutes >= 60 {
                        format!("{}h {}m", minutes / 60, minutes % 60)
                    } else {
                        format!("{minutes}m")
                    },
                    tr(card.detail_key),
                )
            }
            None => (String::new(), tr("screen_time_turn_on")),
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
    wants_keyboard()
        || PAGE.with(|page| {
            let page = page.borrow();
            page.focus.state() == TimerState::Running || page.shown_at.elapsed().as_secs_f32() < 0.8
        })
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
    let spans: Vec<usize> = cards_for(Destination::Home)
        .iter()
        .map(|card| card.column_span)
        .collect();
    let mut layout =
        DashboardLayout::new(Rect::from_xywh(0.0, 0.0, width, height), size_class, &spans);
    let active = PAGE.with(|page| page.borrow().active);
    ROW_ICON.with(|cell| cell.set(row_icon(active)));
    // List pages sit inside one card with even padding, so nothing touches the panel edge.
    if in_card(active) {
        layout.grid = layout.grid.inset(PAGE_PAD);
    }
    layout
}

thread_local! {
    /// The icon at the start of every list row on the current page, if it has one.
    static ROW_ICON: Cell<Option<crate::ui::components::icon::Icon>> = const { Cell::new(None) };
}

/// The icon that leads each row on `destination`'s list.
fn row_icon(destination: Destination) -> Option<crate::ui::components::icon::Icon> {
    use crate::ui::components::icon::Icon;
    match destination {
        Destination::Notes => Some(Icon::Notes),
        Destination::Shelf => Some(Icon::Shelf),
        Destination::Links => Some(Icon::Link),
        Destination::Agenda => Some(Icon::Calendar),
        Destination::Clock => Some(Icon::Clock),
        Destination::AiUsage => Some(Icon::Chart),
        Destination::Clipboard => Some(Icon::Clipboard),
        _ => None,
    }
}

/// A list row with the page's icon in a small tile at its start.
fn draw_row(
    painter: Painter<'_>,
    row: &crate::ui::components::list_row::ListRowLayout,
    title: &str,
    subtitle: Option<&str>,
    colors: &SemanticColors,
    state: InteractionState,
) {
    crate::ui::components::list_row::draw_list_row(painter, row, title, subtitle, colors, state);
    if let (Some(rect), Some(icon)) = (row.leading, ROW_ICON.with(Cell::get)) {
        painter.fill_round_rect(rect, Radius::uniform(6.0), colors.surface_interactive);
        crate::ui::components::icon::draw_icon(
            painter,
            icon,
            rect.inset(5.0),
            colors.text_secondary,
        );
    }
}

/// Padding between a page's card and its content.
const PAGE_PAD: f32 = 16.0;

/// Pages drawn inside a single card: everything but Home and the pages with their own cards.
fn in_card(destination: Destination) -> bool {
    !tabs::custom_layout(destination)
        && !matches!(
            destination,
            Destination::Home
                | Destination::Media
                | Destination::ScreenTime
                | Destination::Clipboard
                | Destination::System
        )
}

fn cards_for(active: Destination) -> &'static [SampleCard] {
    if active == Destination::Home {
        home_cards()
    } else {
        &[]
    }
}

/// Mouse wheel over the visible tab, in device pixels like [`hit_test`]. `lines` is positive
/// for wheel up. `true` when the tab scrolled (or at least claimed the wheel).
pub fn scroll(
    ox: f32,
    oy: f32,
    width: f32,
    height: f32,
    scale: f32,
    point: Point,
    lines: f32,
) -> bool {
    let layout = page_layout(width, height, scale);
    let scale = scale.max(f32::EPSILON);
    let local = Point::new((point.x - ox) / scale, (point.y - oy) / scale);
    let active = PAGE.with(|page| page.borrow().active);
    tabs::scroll(active, layout.grid, local, lines)
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
    if layout.header.contains(local) {
        let (settings, close) = header_buttons(layout.header);
        if search_button(layout.header).contains(local) {
            return Some(DashboardHit::Search);
        }
        let active = PAGE.with(|page| page.borrow().active);
        if let Some((_, hit)) = workspace_tabs(layout.header, active)
            .into_iter()
            .find(|(rect, _)| rect.contains(local))
        {
            return Some(hit);
        }
        if settings.contains(local) {
            return Some(DashboardHit::OpenSettings);
        }
        if close.contains(local) {
            return Some(DashboardHit::Close);
        }
        return Some(DashboardHit::Header);
    }
    if PAGE.with(|page| page.borrow().active) == Destination::Home
        && let Some(hit) = hit_card_badge(&layout.cards, local)
    {
        return Some(hit);
    }
    if let Some(hit) = tabs::hit(PAGE.with(|page| page.borrow().active), layout.grid, local) {
        return Some(hit);
    }
    if PAGE.with(|page| page.borrow().active) == Destination::System
        && let Some(hit) = system_page::hit(layout.grid, local)
    {
        return Some(hit);
    }
    let active = PAGE.with(|page| page.borrow().active);
    if active == Destination::More {
        return hit_more_row(&layout, local).map(DashboardHit::Destination);
    }
    if active == Destination::Shelf {
        return hit_shelf_row(&layout, local).map(DashboardHit::ShelfEntry);
    }
    if active == Destination::Notes {
        return hit_note_row(&layout, local).map(DashboardHit::NoteRow);
    }
    if active == Destination::Focus {
        return focus_buttons(layout.grid)
            .iter()
            .position(|button| button.contains(local))
            .map(|index| DashboardHit::FocusButton(index as u8));
    }
    if active == Destination::Media {
        let media = media_layout(layout.grid);
        if media.bar.inset(-6.0).contains(local) {
            let fraction = ((local.x - media.bar.left) / media.bar.width()).clamp(0.0, 1.0);
            return Some(DashboardHit::MediaSeek((fraction * 1000.0) as u16));
        }
        return media
            .buttons
            .iter()
            .position(|button| button.contains(local))
            .map(|index| DashboardHit::MediaButton(index as u8));
    }
    if active == Destination::Links {
        return hit_link_row(&layout, local).map(DashboardHit::LinkRow);
    }
    if active == Destination::Clipboard {
        return hit_clip_row(&layout, local).map(DashboardHit::ClipRow);
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
/// Files a press on `hit` could drag out of the panel (a Shelf row), if any.
pub fn drag_paths(hit: DashboardHit) -> Option<Vec<std::path::PathBuf>> {
    if PAGE.with(|page| page.borrow().active) != Destination::Shelf {
        return None;
    }
    tabs::shelf::drag_paths(hit)
}

pub fn apply(hit: DashboardHit) {
    if tabs::apply(PAGE.with(|page| page.borrow().active), hit) {
        return;
    }
    if !matches!(hit, DashboardHit::Search | DashboardHit::Header) {
        palette::hide();
    }
    PAGE.with(|page| {
        let mut page = page.borrow_mut();
        if system_page::apply(hit) {
            return;
        }
        if apply_card_badge(hit) {
            return;
        }
        if apply_workspace_hit(hit, &mut page) {
            return;
        }
        match hit {
            DashboardHit::Destination(destination) => {
                if page.active != destination {
                    page.active = destination;
                    page.shown_at = Instant::now();
                }
            }
            DashboardHit::ShelfEntry(index) => open_shelf_entry(index),
            DashboardHit::Search => palette::open(),
            // Clicks in the open search field keep it focused.
            DashboardHit::Header => {}
            DashboardHit::FocusButton(button) => {
                let now = Instant::now();
                match button {
                    0 => page.focus.reset(),
                    1 => toggle_focus(&mut page.focus, now),
                    _ => {
                        page.focus.skip();
                    }
                }
                crate::core::focus_store::save(&page.focus);
            }
            // Applied by `system_page::apply` above.
            DashboardHit::SystemVolume(_) | DashboardHit::SystemMute | DashboardHit::Tab(_) => {}
            DashboardHit::OpenSettings | DashboardHit::Close => {
                HEADER_REQUEST.with(|cell| cell.set(Some(hit)));
            }
            DashboardHit::NoteRow(index) => open_note_row(index),
            DashboardHit::LinkRow(index) => open_link(index),
            DashboardHit::ClipRow(index) => CLIPBOARD_COPY.with(|cell| cell.set(Some(index))),
            DashboardHit::MediaButton(button) => MEDIA_REQUEST.with(|cell| cell.set(Some(button))),
            DashboardHit::MediaSeek(fraction) => MEDIA_SEEK.with(|cell| cell.set(Some(fraction))),
            DashboardHit::Card(index) => {
                let is_focus = cards_for(page.active)
                    .get(index)
                    .is_some_and(|card| card.kind == CardKind::Focus);
                if is_focus {
                    toggle_focus(&mut page.focus, Instant::now());
                    crate::core::focus_store::save(&page.focus);
                }
            }
            // Handled above by `apply_card_badge` and `apply_workspace_hit`.
            DashboardHit::Workspace(_)
            | DashboardHit::AddWorkspace
            | DashboardHit::Arrange
            | DashboardHit::CardMove(_)
            | DashboardHit::CardResize(_)
            | DashboardHit::CardRemove(_) => {}
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
    let (active, hover, focus, entrance) = PAGE.with(|page| {
        let page = page.borrow();
        (
            page.active,
            page.hover,
            page.focus.view(now),
            now.saturating_duration_since(page.shown_at).as_secs_f32(),
        )
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
        hovered_focus: match hover {
            Some(DashboardHit::FocusButton(button)) => Some(button),
            _ => None,
        },
        hovered_header: hover.filter(|hit| {
            matches!(
                hit,
                DashboardHit::Search
                    | DashboardHit::OpenSettings
                    | DashboardHit::Close
                    | DashboardHit::Workspace(_)
                    | DashboardHit::AddWorkspace
                    | DashboardHit::Arrange
            )
        }),
        hovered_row: match hover {
            Some(DashboardHit::ShelfEntry(index)) => Some(index),
            Some(DashboardHit::NoteRow(index)) => Some(index),
            Some(DashboardHit::LinkRow(index)) => Some(index),
            Some(DashboardHit::ClipRow(index)) => Some(index),
            Some(DashboardHit::Destination(destination)) if active == Destination::More => {
                more_entries(&layout)
                    .iter()
                    .position(|candidate| *candidate == destination)
            }
            _ => None,
        },
        ..DashboardInteraction::default()
    };
    let colors = crate::ui::theme::colors();
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
        entrance,
    });
    let caret_visible = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(true, |elapsed| {
            (elapsed.as_millis() / 500).is_multiple_of(2)
        });
    palette::draw(painter, layout.header, &colors, caret_visible);
    painter.restore();
}

/// Delay between cards in the staggered entrance, in seconds.
const STAGGER_STEP: f32 = 0.045;
/// Duration of each card's entrance, in seconds.
const STAGGER_LENGTH: f32 = 0.26;
/// How far cards rise while appearing, in logical pixels.
const STAGGER_RISE: f32 = 8.0;

/// Primary destinations, matching the first-run modules in the design guide.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Destination {
    Home,
    /// Battery, network, performance and volume.
    System,
    /// Image conversion (JPG, PNG, PDF, ICO).
    Converter,
    /// Unit and currency conversion.
    Units,
    /// A custom banner shown in the notch.
    Message,
    Media,
    Clipboard,
    Shelf,
    Focus,
    More,
    /// Reached from More; not a dock destination.
    Notes,
    /// World clocks; reached from More.
    Clock,
    /// AI coding usage report; reached from More.
    AiUsage,
    /// Quick links; reached from More.
    Links,
    /// Forecast details; reached from More.
    Weather,
    /// Screen time report; reached from More.
    ScreenTime,
    /// Upcoming events from calendar feeds; reached from More.
    Agenda,
    /// Phone transfer by QR code; reached from More.
    Drop,
    /// Quick games; reached from More.
    Play,
    /// Notchy Link to the phone app; reached from More.
    Phone,
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
    /// The icon dock, in order: every module, like a launcher.
    /// Every page, as an icon row; each mode can hide the ones it does not need.
    pub const DOCK: [Self; 19] = [
        Self::Home,
        Self::System,
        Self::Media,
        Self::Clipboard,
        Self::Notes,
        Self::Focus,
        Self::Shelf,
        Self::ScreenTime,
        Self::Weather,
        Self::Clock,
        Self::Agenda,
        Self::AiUsage,
        Self::Links,
        Self::Converter,
        Self::Units,
        Self::Message,
        Self::Phone,
        Self::Drop,
        Self::Play,
    ];
    /// Shown in the compact size class; the rest move into More.
    pub const COMPACT: [Self; 3] = [Self::Home, Self::Focus, Self::More];
    /// Modules listed on the More page when they are not already in the dock.
    pub const MORE_ITEMS: [Self; 18] = [
        Self::System,
        Self::Media,
        Self::Clipboard,
        Self::Shelf,
        Self::Focus,
        Self::Notes,
        Self::Clock,
        Self::AiUsage,
        Self::Links,
        Self::Weather,
        Self::ScreenTime,
        Self::Agenda,
        Self::Converter,
        Self::Units,
        Self::Message,
        Self::Phone,
        Self::Drop,
        Self::Play,
    ];

    /// The context line shown after the page title.
    pub const fn subtitle_key(self) -> &'static str {
        match self {
            Self::Home | Self::More => "subtitle_home",
            Self::System => "subtitle_system",
            Self::Converter => "subtitle_converter",
            Self::Units => "subtitle_units",
            Self::Message => "subtitle_message",
            Self::Media => "subtitle_media",
            Self::Clipboard => "subtitle_clipboard",
            Self::Shelf => "subtitle_shelf",
            Self::Focus => "subtitle_focus",
            Self::Notes => "subtitle_notes",
            Self::Clock => "subtitle_clock",
            Self::AiUsage => "subtitle_ai_usage",
            Self::Links => "subtitle_links",
            Self::Weather => "subtitle_weather",
            Self::ScreenTime => "subtitle_screen_time",
            Self::Agenda => "subtitle_agenda",
            Self::Drop => "subtitle_drop",
            Self::Play => "subtitle_play",
            Self::Phone => "subtitle_phone",
        }
    }

    pub const fn icon(self) -> crate::ui::components::icon::Icon {
        use crate::ui::components::icon::Icon;
        match self {
            Self::Home => Icon::Home,
            Self::System => Icon::Gauge,
            Self::Converter => Icon::Convert,
            Self::Units => Icon::Ruler,
            Self::Message => Icon::Message,
            Self::Media => Icon::Music,
            Self::Clipboard => Icon::Clipboard,
            Self::Shelf => Icon::Shelf,
            Self::Focus => Icon::Focus,
            Self::More => Icon::More,
            Self::Notes => Icon::Notes,
            Self::Clock => Icon::Clock,
            Self::AiUsage => Icon::Sparkle,
            Self::Links => Icon::Link,
            Self::Weather => Icon::Weather,
            Self::ScreenTime => Icon::Chart,
            Self::Agenda => Icon::Calendar,
            Self::Drop => Icon::Phone,
            Self::Play => Icon::Pet,
            Self::Phone => Icon::Devices,
        }
    }

    pub const fn label_key(self) -> &'static str {
        match self {
            Self::Home => "dashboard_home",
            Self::System => "dashboard_system",
            Self::Converter => "dashboard_converter",
            Self::Units => "dashboard_units",
            Self::Message => "dashboard_message",
            Self::Media => "dashboard_media",
            Self::Clipboard => "dashboard_clipboard",
            Self::Shelf => "dashboard_shelf",
            Self::Focus => "dashboard_focus",
            Self::More => "dashboard_more",
            Self::Notes => "dashboard_notes",
            Self::Clock => "dashboard_clock",
            Self::AiUsage => "dashboard_ai_usage",
            Self::Links => "dashboard_links",
            Self::Weather => "dashboard_weather",
            Self::ScreenTime => "dashboard_screen_time",
            Self::Agenda => "dashboard_agenda",
            Self::Drop => "dashboard_drop",
            Self::Play => "dashboard_play",
            Self::Phone => "dashboard_phone",
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
    /// Hovered header button, if any.
    pub hovered_header: Option<DashboardHit>,
    /// Hovered Focus page button.
    pub hovered_focus: Option<u8>,
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
    /// Seconds since the page was shown, for the staggered card entrance.
    pub entrance: f32,
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
        entrance,
    } = params;
    painter.fill_round_rect(
        layout.bounds,
        Radius::uniform(radius::WORKSPACE),
        colors.surface_base,
    );
    draw_header(
        painter,
        layout.header,
        active,
        interaction.hovered_header,
        colors,
    );

    let save = painter.save();
    painter.clip_rect(layout.grid);
    for (index, (rect, card)) in layout.cards.iter().zip(cards).enumerate() {
        // Cards fade and rise in one after another when a page opens.
        let appear = ((entrance - index as f32 * STAGGER_STEP) / STAGGER_LENGTH).clamp(0.0, 1.0);
        let appear = 1.0 - (1.0 - appear).powi(3);
        if appear <= 0.0 {
            continue;
        }
        let card_save = painter.save();
        if appear < 1.0 {
            painter.save_alpha((appear * 255.0) as u8);
            painter.translate(Vec2::new(0.0, (1.0 - appear) * STAGGER_RISE));
        }
        let card_layout = CardLayout::new(*rect, 0);
        let state = InteractionState {
            hovered: interaction.hovered_card == Some(index),
            focused: interaction.focused_card == Some(index),
            ..InteractionState::default()
        };
        draw_card(
            painter,
            &card_layout,
            &tr(card.title_key),
            Some(card.kind.icon()),
            colors,
            state,
        );
        if card.kind == CardKind::System {
            draw_system_rings(painter, card_layout.content, colors);
        } else if card.kind == CardKind::Clipboard
            && draw_recent_clips(painter, card_layout.content, colors)
        {
        } else if card.kind == CardKind::Notes
            && draw_recent_notes(painter, card_layout.content, colors)
        {
        } else {
            let (value, detail) = card_text(card, focus.as_ref());
            draw_card_value(painter, &card_layout, &value, &detail, colors);
        }
        if card.kind == CardKind::Focus
            && let Some(view) = focus.as_ref()
        {
            if view.state == TimerState::Running
                && let Some(progress) = view.progress
            {
                draw_progress_ring(painter, card_layout.content, 1.0 - progress, colors);
            } else {
                draw_start_button(painter, card_layout.content, colors);
            }
        }
        if card.kind == CardKind::ScreenTime {
            draw_mini_week(painter, card_layout.content, colors);
        }
        painter.restore_to(card_save);
    }
    if in_card(active) {
        page_card(painter, layout.grid.inset(-PAGE_PAD), colors);
    }
    let tab_context = tabs::TabContext {
        painter,
        grid: layout.grid,
        hover: PAGE.with(|page| page.borrow().hover),
        colors,
        now: Instant::now(),
    };
    if tabs::draw(active, &tab_context) {
        // Drawn by its tab module.
    } else if active == Destination::Clipboard {
        draw_clipboard_page(painter, layout.grid, interaction.hovered_row, colors);
    } else if active == Destination::More {
        draw_more_page(painter, layout, interaction.hovered_row, colors);
    } else if active == Destination::Notes {
        draw_notes_page(painter, layout.grid, interaction.hovered_row, colors);
    } else if active == Destination::Clock {
        draw_clock_page(painter, layout.grid, colors);
    } else if active == Destination::AiUsage {
        draw_ai_usage_page(painter, layout.grid, colors);
    } else if active == Destination::System {
        let hover = PAGE.with(|page| page.borrow().hover);
        system_page::draw(painter, layout.grid, hover, colors);
    } else if active == Destination::Weather {
        draw_weather_page(painter, layout.grid, colors);
    } else if active == Destination::Drop {
        draw_drop_page(painter, layout.grid, colors);
    } else if active == Destination::Agenda {
        draw_rows_page(
            &AGENDA_ROWS,
            "agenda_empty_hint",
            painter,
            layout.grid,
            colors,
        );
    } else if active == Destination::ScreenTime {
        draw_screen_time_page(painter, layout.grid, colors);
    } else if active == Destination::Focus {
        draw_focus_page(
            painter,
            layout.grid,
            focus.as_ref(),
            interaction.hovered_focus,
            colors,
        );
    } else if active == Destination::Media {
        draw_media_page(painter, layout.grid, colors);
    } else if active == Destination::Links {
        draw_links_page(painter, layout.grid, interaction.hovered_row, colors);
    } else if active == Destination::Shelf {
        draw_shelf_page(painter, layout.grid, interaction.hovered_row, colors);
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
        &layout
            .destinations
            .iter()
            .map(|destination| destination.icon())
            .collect::<Vec<_>>(),
        active_index,
        interaction.hovered_dock,
        interaction.focused_dock,
        colors,
    );
    draw_arrange_badges(painter, &layout.dock.items, &layout.destinations);
    if active == Destination::Home {
        draw_card_badges(painter, &layout.cards);
    }
}

/// Header button size and the two right-aligned buttons: settings, then close.
const HEADER_BUTTON: f32 = 26.0;

/// The search button, left of settings.
fn search_button(header: Rect) -> Rect {
    let (settings, _) = header_buttons(header);
    Rect::from_xywh(
        settings.left - HEADER_BUTTON - space::S1,
        settings.top,
        HEADER_BUTTON,
        HEADER_BUTTON,
    )
}

fn header_buttons(header: Rect) -> (Rect, Rect) {
    let top = header.center_y() - HEADER_BUTTON / 2.0;
    let close = Rect::from_xywh(
        header.right - HEADER_BUTTON,
        top,
        HEADER_BUTTON,
        HEADER_BUTTON,
    );
    let settings = Rect::from_xywh(
        close.left - HEADER_BUTTON - space::S1,
        top,
        HEADER_BUTTON,
        HEADER_BUTTON,
    );
    (settings, close)
}

fn today_label() -> String {
    const DAYS: [&str; 7] = [
        "Sunday",
        "Monday",
        "Tuesday",
        "Wednesday",
        "Thursday",
        "Friday",
        "Saturday",
    ];
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let now = crate::platform::shell().local_datetime();
    let day = DAYS
        .get(usize::from(now.day_of_week))
        .copied()
        .unwrap_or("");
    let month = MONTHS
        .get(usize::from(now.month.saturating_sub(1)))
        .copied()
        .unwrap_or("");
    format!("{day}, {month} {}", now.day)
}

fn draw_header(
    painter: Painter<'_>,
    header: Rect,
    active: Destination,
    hovered: Option<DashboardHit>,
    colors: &SemanticColors,
) {
    use crate::ui::components::icon::{Icon, draw_icon};
    // Left: on Home only the workspace tabs; on a module page its icon and title first.
    let divider_x = if active == Destination::Home {
        // The robot waves from the corner of Home, before the workspace tabs.
        let robot = header.height().min(28.0);
        crate::ui::widget::compact::mascot::draw_gesture(
            painter,
            Rect::from_xywh(header.left, header.center_y() - robot / 2.0, robot, robot),
            u8::MAX,
            crate::ui::widget::compact::mascot::Gesture::Wave,
        );
        header.left + robot + space::S2 - space::S3
    } else {
        let icon_size = 18.0;
        draw_icon(
            painter,
            active.icon(),
            Rect::from_xywh(
                header.left,
                header.center_y() - icon_size / 2.0,
                icon_size,
                icon_size,
            ),
            colors.text_primary,
        );
        let title = tr(active.label_key());
        let title_left = header.left + icon_size + space::S2;
        let title_width = winisland_render::text::FontManager::global().measure_text_cached(
            &title,
            typography::SECTION_TITLE.size,
            winisland_render::FontStyle::bold(),
        );
        draw_text(
            painter,
            &title,
            typography::SECTION_TITLE,
            Rect::from_xywh(title_left, header.top, title_width + 2.0, header.height()),
            Align::Start,
            colors.text_primary,
        );
        let divider_x = title_left + title_width + space::S3;
        painter.fill_rect(
            Rect::from_xywh(divider_x, header.center_y() - 8.0, 1.0, 16.0),
            colors.border_strong,
        );
        divider_x
    };
    let (settings, close) = header_buttons(header);
    let date_left = divider_x + space::S3;
    if active != Destination::Home {
        // A quiet line of context after the title, kept clear of the centred music pill.
        let playing = NOW_PLAYING.with(|cell| cell.borrow().is_some());
        let subtitle_right = if playing {
            header.center_x() - 124.0
        } else {
            search_button(header).left - space::S3
        };
        draw_text(
            painter,
            &tr(active.subtitle_key()),
            typography::BODY,
            Rect::from_ltrb(date_left, header.top, subtitle_right, header.bottom),
            Align::Start,
            colors.text_secondary,
        );
    }
    draw_workspace_tabs(painter, header, active, hovered, colors);
    // Centre: what is playing, as a pill.
    if let Some(title) = NOW_PLAYING.with(|cell| cell.borrow().clone()) {
        let pill_width = (header.width() * 0.3).clamp(120.0, 220.0);
        let pill = Rect::from_xywh(
            header.center_x() - pill_width / 2.0,
            header.center_y() - 13.0,
            pill_width,
            26.0,
        );
        if pill.left
            > workspace_tabs(header, active)
                .last()
                .map_or(date_left, |(rect, _)| rect.right)
                + 12.0
            && pill.right < search_button(header).left - BATTERY_CHIP_WIDTH
        {
            painter.fill_round_rect(pill, Radius::uniform(13.0), colors.surface_raised);
            painter.stroke_round_rect(pill, Radius::uniform(13.0), 1.0, colors.accent_soft);
            draw_icon(
                painter,
                Icon::Music,
                Rect::from_xywh(pill.left + 9.0, pill.center_y() - 7.0, 14.0, 14.0),
                colors.accent_primary,
            );
            draw_text(
                painter,
                &title,
                typography::LABEL,
                Rect::from_ltrb(pill.left + 29.0, pill.top, pill.right - 10.0, pill.bottom),
                Align::Start,
                colors.text_primary,
            );
        }
    }
    let search = search_button(header);
    draw_battery_chip(painter, header, search.left - space::S2, colors);
    // Right: search, settings and close.
    for (rect, icon, hit) in [
        (search, Icon::Search, DashboardHit::Search),
        (settings, Icon::Settings, DashboardHit::OpenSettings),
        (close, Icon::Plus, DashboardHit::Close),
    ] {
        if hovered == Some(hit) {
            painter.fill_round_rect(
                rect,
                Radius::uniform(rect.height() / 2.0),
                colors.surface_overlay,
            );
        }
        let inner = rect.inset(6.0);
        if hit == DashboardHit::Close {
            // The plus icon rotated 45 degrees reads as a close cross.
            let save = painter.save();
            painter.translate(Vec2::new(inner.center_x(), inner.center_y()));
            painter.rotate_degrees(45.0);
            draw_icon(
                painter,
                icon,
                Rect::from_xywh(
                    -inner.width() / 2.0,
                    -inner.height() / 2.0,
                    inner.width(),
                    inner.height(),
                ),
                colors.text_secondary,
            );
            painter.restore_to(save);
        } else {
            draw_icon(painter, icon, inner, colors.text_secondary);
        }
    }
}

thread_local! {
    static NOW_PLAYING: RefCell<Option<String>> = const { RefCell::new(None) };
    static HEADER_REQUEST: Cell<Option<DashboardHit>> = const { Cell::new(None) };
}

/// Publishes the playing track for the header pill, or `None` when nothing plays.
pub fn set_now_playing(title: Option<String>) {
    NOW_PLAYING.with(|cell| {
        let mut current = cell.borrow_mut();
        if *current != title {
            *current = title;
        }
    });
}

/// Whether the user clicked the header's settings (`true`) or close (`false`) button.
pub fn take_header_request() -> Option<bool> {
    HEADER_REQUEST
        .with(Cell::take)
        .map(|hit| hit == DashboardHit::OpenSettings)
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
    if value.is_empty() {
        // An empty state: the hint takes the figure's place, quietly.
        draw_text(
            painter,
            detail,
            typography::BODY,
            value_rect,
            Align::Start,
            colors.text_secondary,
        );
        return;
    }
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
    WORLD_CLOCKS.with(|cell| {
        let keys = cell.borrow();
        if keys.is_empty() {
            // Your own time, large, with how to add more cities under it.
            let now = crate::platform::shell().local_datetime();
            draw_text(
                painter,
                &format!("{:02}:{:02}", now.hour, now.minute),
                typography::DISPLAY,
                Rect::from_xywh(grid.left, grid.center_y() - 34.0, grid.width(), 40.0),
                Align::Center,
                colors.text_primary,
            );
            draw_text(
                painter,
                &tr("world_clock_empty"),
                typography::BODY,
                Rect::from_xywh(grid.left, grid.center_y() + 14.0, grid.width(), 20.0),
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
            draw_row(
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

struct ShelfView {
    revision: Option<u64>,
    name: String,
    entries: Vec<crate::core::shelf_store::ShelfRow>,
}

thread_local! {
    static SHELF: RefCell<ShelfView> = const {
        RefCell::new(ShelfView {
            revision: None,
            name: String::new(),
            entries: Vec::new(),
        })
    };
    static SHELF_REMOVAL: Cell<Option<u64>> = const { Cell::new(None) };
    static SHELF_CYCLE: Cell<bool> = const { Cell::new(false) };
}

/// Publishes shelf entries for the Shelf page, rebuilding them only when they changed.
pub fn sync_shelf(
    revision: u64,
    name: impl FnOnce() -> String,
    entries: impl FnOnce() -> Vec<crate::core::shelf_store::ShelfRow>,
) {
    let stale = SHELF.with(|cell| cell.borrow().revision != Some(revision));
    if stale {
        let name = name();
        let entries = entries();
        SHELF.with(|cell| {
            let mut view = cell.borrow_mut();
            view.revision = Some(revision);
            view.name = name;
            view.entries = entries;
        });
    }
}

/// An entry the user asked to remove (a missing file that was clicked).
pub fn take_shelf_removal() -> Option<u64> {
    SHELF_REMOVAL.with(Cell::take)
}

/// Whether the user asked to switch shelves since the last call.
pub fn take_shelf_cycle() -> bool {
    SHELF_CYCLE.with(Cell::take)
}

fn hit_shelf_row(layout: &DashboardLayout, point: Point) -> Option<usize> {
    if !layout.grid.contains(point) {
        return None;
    }
    let count = SHELF.with(|cell| cell.borrow().entries.len()) + 1;
    (0..count).find(|index| list_row(layout.grid, *index, true).bounds.contains(point))
}

/// Opens an existing file; clicking a missing one removes it from the shelf.
fn open_shelf_entry(index: usize) {
    if index == 0 {
        SHELF_CYCLE.with(|cell| cell.set(true));
        return;
    }
    let entry = SHELF.with(|cell| cell.borrow().entries.get(index - 1).cloned());
    let Some((id, _, path, _)) = entry else {
        return;
    };
    if !path.exists() {
        SHELF_REMOVAL.with(|cell| cell.set(Some(id)));
        return;
    }
    if let Err(error) = crate::platform::shell().open_url(&path.to_string_lossy()) {
        log::warn!("Cannot open shelf file: {error}");
    }
}

fn draw_shelf_page(
    painter: Painter<'_>,
    grid: Rect,
    hovered_row: Option<usize>,
    colors: &SemanticColors,
) {
    SHELF.with(|cell| {
        let view = cell.borrow();
        let header = list_row(grid, 0, true);
        draw_row(
            painter,
            &header,
            &view.name,
            Some(
                tr(if view.entries.is_empty() {
                    "shelf_empty_hint"
                } else {
                    "shelf_switch"
                })
                .as_str(),
            ),
            colors,
            InteractionState {
                hovered: hovered_row == Some(0),
                selected: true,
                ..InteractionState::default()
            },
        );
        for (index, (_, name, path, exists)) in view.entries.iter().enumerate() {
            let index = index + 1;
            let row = list_row(grid, index, true);
            if row.bounds.top >= grid.bottom {
                break;
            }
            let detail = if *exists {
                path.parent()
                    .map(|parent| parent.display().to_string())
                    .unwrap_or_default()
            } else {
                tr("shelf_missing")
            };
            let state = InteractionState {
                hovered: hovered_row == Some(index),
                disabled: !exists,
                ..InteractionState::default()
            };
            draw_row(painter, &row, name, Some(detail.as_str()), colors, state);
        }
    });
}

/// Flattens the dashboard's semantic tree into screen-space UI Automation elements.
/// `origin` is the island window's screen position in physical pixels.
pub fn accessible_elements(
    ox: f32,
    oy: f32,
    width: f32,
    height: f32,
    scale: f32,
    origin: (f64, f64),
) -> Vec<winisland_platform_windows::accessibility::AccessibleElement> {
    use crate::ui::components::semantics::Role;
    use winisland_platform_windows::accessibility::{AccessibleElement, AccessibleRole};

    let layout = page_layout(width, height, scale);
    let active = PAGE.with(|page| page.borrow().active);
    let tree = semantics(
        &layout,
        active,
        cards_for(active),
        DashboardInteraction::default(),
    );
    let mut elements = Vec::new();
    let mut stack = vec![tree];
    while let Some(node) = stack.pop() {
        if !node.name.is_empty() {
            let bounds = node.bounds;
            elements.push(AccessibleElement {
                name: node.name.clone(),
                role: match node.role {
                    Role::Button => AccessibleRole::Button,
                    Role::Group => AccessibleRole::Group,
                    Role::ListItem => AccessibleRole::ListItem,
                    Role::Tab => AccessibleRole::Tab,
                    Role::TabList => AccessibleRole::TabList,
                    Role::Text => AccessibleRole::Text,
                },
                bounds: (
                    origin.0 + f64::from(ox + bounds.left * scale),
                    origin.1 + f64::from(oy + bounds.top * scale),
                    f64::from(bounds.width() * scale),
                    f64::from(bounds.height() * scale),
                ),
                selected: node.state.selected,
            });
        }
        stack.extend(node.children.into_iter().rev());
    }
    elements
}

struct AiUsageView {
    key: Option<(bool, u64)>,
    /// `(title, detail)` rows: today, live sessions, top models, then recent days.
    rows: Vec<(String, String)>,
}

thread_local! {
    static AI_USAGE: RefCell<AiUsageView> = const {
        RefCell::new(AiUsageView {
            key: None,
            rows: Vec::new(),
        })
    };
}

/// Publishes the AI usage report, rebuilding rows only when it changed.
pub fn sync_ai_usage(
    enabled: bool,
    revision: u64,
    report: &winisland_core::ai_usage::UsageReport,
    active_sessions: usize,
) {
    use winisland_core::ai_usage::format_tokens;
    use winisland_core::i18n::tr_args;
    AI_USAGE.with(|cell| {
        let mut view = cell.borrow_mut();
        if view.key == Some((enabled, revision)) {
            return;
        }
        view.key = Some((enabled, revision));
        view.rows.clear();
        if !enabled {
            return;
        }
        if let Some((date, today)) = report.days.iter().next_back() {
            view.rows.push((
                tr_args("ai_usage_tokens", &[&format_tokens(today.tokens)]),
                tr_args(
                    "ai_usage_today_detail",
                    &[
                        date,
                        &today.sessions.to_string(),
                        &today.responses.to_string(),
                    ],
                ),
            ));
        }
        view.rows.push((
            if active_sessions == 1 {
                tr("ai_usage_live_one")
            } else {
                tr_args("ai_usage_live", &[&active_sessions.to_string()])
            },
            tr("ai_usage_live_detail"),
        ));
        view.rows.push((
            tr_args("ai_usage_cost", &[&thousands(report.total_cost_usd())]),
            tr("ai_usage_cost_detail"),
        ));
        view.rows.push((
            tr_args("ai_usage_streak", &[&report.streak_days().to_string()]),
            tr("ai_usage_streak_detail"),
        ));
        for (model, tokens) in report
            .models
            .iter()
            .filter(|(model, _)| !model.starts_with('<'))
            .take(3)
        {
            view.rows.push((model.clone(), format_tokens(*tokens)));
        }
        for (tool, tokens) in &report.tools {
            view.rows
                .push((tool.label().to_string(), format_tokens(*tokens)));
        }
        for (date, day) in report.days.iter().rev().skip(1).take(6) {
            view.rows.push((
                date.clone(),
                tr_args(
                    "ai_usage_day_detail",
                    &[&format_tokens(day.tokens), &day.sessions.to_string()],
                ),
            ));
        }
    });
}

fn draw_ai_usage_page(painter: Painter<'_>, grid: Rect, colors: &SemanticColors) {
    AI_USAGE.with(|cell| {
        let view = cell.borrow();
        let empty = match view.key {
            Some((false, _)) | None => Some("ai_usage_off_hint"),
            Some((true, _)) if view.rows.len() <= 1 => Some("ai_usage_empty_hint"),
            _ => None,
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
        // The headline figures as tiles across the top, the rest as a list below.
        const TILES: usize = 4;
        let tile_height = 56.0;
        let gap = space::S2;
        let tile_width = (grid.width() - gap * (TILES as f32 - 1.0)) / TILES as f32;
        for (index, (title, detail)) in view.rows.iter().take(TILES).enumerate() {
            let tile = Rect::from_xywh(
                grid.left + index as f32 * (tile_width + gap),
                grid.top,
                tile_width,
                tile_height,
            );
            painter.fill_round_rect(
                tile,
                Radius::uniform(radius::ROW),
                colors.surface_interactive,
            );
            draw_text(
                painter,
                title,
                typography::BODY,
                Rect::from_xywh(tile.left + 10.0, tile.top + 8.0, tile.width() - 20.0, 20.0),
                Align::Start,
                colors.text_primary,
            );
            draw_text(
                painter,
                detail,
                typography::CAPTION,
                Rect::from_xywh(tile.left + 10.0, tile.top + 30.0, tile.width() - 20.0, 16.0),
                Align::Start,
                colors.text_secondary,
            );
        }
        let list = Rect::from_ltrb(
            grid.left,
            grid.top + tile_height + space::S3,
            grid.right,
            grid.bottom,
        );
        for (index, (title, detail)) in view.rows.iter().skip(TILES).enumerate() {
            let row = list_row(list, index, true);
            if row.bounds.bottom > list.bottom {
                break;
            }
            draw_row(
                painter,
                &row,
                title,
                Some(detail.as_str()),
                colors,
                InteractionState::default(),
            );
        }
    });
}

/// Whether a palette or editor needs the island to take keyboard focus.
pub fn wants_keyboard() -> bool {
    palette::is_open()
        || editor::is_open()
        || tabs::wants_keyboard(PAGE.with(|page| page.borrow().active))
}

/// Offers a key to the visible tab first. `true` when the tab used it.
pub fn handle_tab_key(key: &winisland_platform::Key) -> bool {
    tabs::handle_key(PAGE.with(|page| page.borrow().active), key)
}

thread_local! {
    static LINKS: RefCell<Vec<(String, String)>> = const { RefCell::new(Vec::new()) };
}

/// Publishes the configured quick links.
pub fn set_links(links: &[(String, String)]) {
    LINKS.with(|cell| {
        let mut current = cell.borrow_mut();
        if current.as_slice() != links {
            *current = links.to_vec();
        }
    });
}

fn hit_link_row(layout: &DashboardLayout, point: Point) -> Option<usize> {
    if !layout.grid.contains(point) {
        return None;
    }
    let count = LINKS.with(|cell| cell.borrow().len());
    (0..count).find(|index| list_row(layout.grid, *index, true).bounds.contains(point))
}

fn open_link(index: usize) {
    let url = LINKS.with(|cell| cell.borrow().get(index).map(|link| link.1.clone()));
    if let Some(url) = url
        && let Err(error) = crate::platform::shell().open_url(&url)
    {
        log::warn!("Cannot open link: {error}");
    }
}

fn draw_links_page(
    painter: Painter<'_>,
    grid: Rect,
    hovered_row: Option<usize>,
    colors: &SemanticColors,
) {
    LINKS.with(|cell| {
        let links = cell.borrow();
        if links.is_empty() {
            draw_text(
                painter,
                &tr("links_empty_hint"),
                typography::BODY,
                grid,
                Align::Center,
                colors.text_secondary,
            );
            return;
        }
        for (index, (name, url)) in links.iter().enumerate() {
            let row = list_row(grid, index, true);
            if row.bounds.top >= grid.bottom {
                break;
            }
            // Show the destination domain before anything is opened.
            let domain = url
                .split("://")
                .nth(1)
                .and_then(|rest| rest.split('/').next())
                .unwrap_or(url);
            draw_row(
                painter,
                &row,
                name,
                Some(domain),
                colors,
                InteractionState {
                    hovered: hovered_row == Some(index),
                    ..InteractionState::default()
                },
            );
        }
    });
}

thread_local! {
    static WEATHER_ROWS: RefCell<Vec<(String, String)>> = const { RefCell::new(Vec::new()) };
}

/// Publishes the Weather page rows.
pub fn set_weather_rows(rows: Vec<(String, String)>) {
    WEATHER_ROWS.with(|cell| {
        let mut current = cell.borrow_mut();
        if *current != rows {
            *current = rows;
        }
    });
}

fn draw_weather_page(painter: Painter<'_>, grid: Rect, colors: &SemanticColors) {
    let hourly = WEATHER_HOURLY.with(|cell| cell.borrow().clone());
    let grid = if hourly.len() >= 2 {
        let chart = Rect::from_ltrb(
            grid.left,
            grid.top,
            grid.right,
            grid.top + WEATHER_CHART_HEIGHT,
        );
        draw_temperature_chart(painter, chart, &hourly, colors);
        Rect::from_ltrb(grid.left, chart.bottom + space::S2, grid.right, grid.bottom)
    } else {
        grid
    };
    WEATHER_ROWS.with(|cell| {
        let rows = cell.borrow();
        if rows.is_empty() {
            draw_text(
                painter,
                &tr("weather_turn_on"),
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
            draw_row(
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

/// Whether `destination` is the active dashboard page.
/// Whether the Play tab is open with a game running (it needs smooth frames).
pub fn play_animating() -> bool {
    showing(Destination::Play) && tabs::play::animating()
}

pub fn showing(destination: Destination) -> bool {
    PAGE.with(|page| page.borrow().active == destination)
}

thread_local! {
    static SCREEN_TIME_ROWS: RefCell<Vec<(String, String)>> = const { RefCell::new(Vec::new()) };
}

/// Publishes the Screen Time page rows.
pub fn set_screen_time_rows(rows: Vec<(String, String)>) {
    SCREEN_TIME_ROWS.with(|cell| {
        let mut current = cell.borrow_mut();
        if *current != rows {
            *current = rows;
        }
    });
}

/// Draws a simple `(title, detail)` list, or `empty_key` when it is empty.
fn draw_rows_page(
    rows: &'static std::thread::LocalKey<RefCell<Vec<(String, String)>>>,
    empty_key: &str,
    painter: Painter<'_>,
    grid: Rect,
    colors: &SemanticColors,
) {
    rows.with(|cell| {
        let rows = cell.borrow();
        if rows.is_empty() {
            draw_text(
                painter,
                &tr(empty_key),
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
            draw_row(
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
    static HIDDEN_MODULES: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

/// Publishes which More-page modules the user hid.
pub fn set_hidden_modules(keys: &[String]) {
    HIDDEN_MODULES.with(|cell| {
        let mut current = cell.borrow_mut();
        if current.as_slice() != keys {
            *current = keys.to_vec();
        }
    });
}

thread_local! {
    static AGENDA_ROWS: RefCell<Vec<(String, String)>> = const { RefCell::new(Vec::new()) };
    static AGENDA_REVISION: Cell<Option<u64>> = const { Cell::new(None) };
}

/// Publishes agenda rows, rebuilding them only when the feeds changed.
pub fn sync_agenda(revision: u64, rows: impl FnOnce() -> Vec<(String, String)>) {
    if AGENDA_REVISION.with(Cell::get) != Some(revision) {
        AGENDA_REVISION.with(|cell| cell.set(Some(revision)));
        let rows = rows();
        AGENDA_ROWS.with(|cell| *cell.borrow_mut() = rows);
    }
}

struct DropView {
    url: String,
    /// QR modules, row-major, `true` for dark.
    modules: Vec<bool>,
    width: usize,
    minutes_left: u64,
}

thread_local! {
    static DROP_VIEW: RefCell<Option<DropView>> = const { RefCell::new(None) };
}

/// Publishes the active Drop session as `(url, minutes left)`, or `None`.
pub fn set_drop(session: Option<(&str, u64)>) {
    DROP_VIEW.with(|cell| {
        let mut view = cell.borrow_mut();
        let Some((url, minutes_left)) = session else {
            *view = None;
            return;
        };
        if let Some(current) = view.as_mut().filter(|current| current.url == url) {
            current.minutes_left = minutes_left;
            return;
        }
        let Ok(code) = qrcode::QrCode::new(url.as_bytes()) else {
            *view = None;
            return;
        };
        let width = code.width();
        let modules = code
            .to_colors()
            .into_iter()
            .map(|color| color == qrcode::Color::Dark)
            .collect();
        *view = Some(DropView {
            url: url.to_string(),
            modules,
            width,
            minutes_left,
        });
    });
}

fn draw_drop_page(painter: Painter<'_>, grid: Rect, colors: &SemanticColors) {
    DROP_VIEW.with(|cell| {
        let view = cell.borrow();
        let Some(view) = view.as_ref() else {
            draw_text(
                painter,
                &tr("drop_unavailable"),
                typography::BODY,
                grid,
                Align::Center,
                colors.text_secondary,
            );
            return;
        };
        // White quiet zone and dark modules, sized to fit the grid.
        let side = grid.height().min(grid.width() * 0.5);
        let quiet = 2.0;
        let module = side / (view.width as f32 + quiet * 2.0);
        let qr = Rect::from_xywh(grid.left, grid.top, side, side);
        painter.fill_rect(qr, winisland_render::Rgba::WHITE);
        for (index, dark) in view.modules.iter().enumerate() {
            if *dark {
                let x = (index % view.width) as f32;
                let y = (index / view.width) as f32;
                painter.fill_rect(
                    Rect::from_xywh(
                        qr.left + (x + quiet) * module,
                        qr.top + (y + quiet) * module,
                        module.ceil(),
                        module.ceil(),
                    ),
                    winisland_render::Rgba::BLACK,
                );
            }
        }
        let text = Rect::from_ltrb(qr.right + space::S4, grid.top, grid.right, grid.bottom);
        let line = typography::BODY.size * 1.6;
        // The note is two sentences; one line each so neither gets cut off.
        let note = tr("drop_same_wifi");
        let (wifi, received) = note
            .split_once(". ")
            .map(|(first, rest)| (format!("{first}."), rest.to_string()))
            .or_else(|| {
                note.split_once('。')
                    .map(|(first, rest)| (format!("{first}。"), rest.to_string()))
            })
            .unwrap_or((note.clone(), String::new()));
        let lines = [
            (
                tr("drop_scan"),
                typography::SECTION_TITLE,
                colors.text_primary,
            ),
            (
                winisland_core::i18n::tr_args(
                    "drop_minutes_left",
                    &[&view.minutes_left.to_string()],
                ),
                typography::CAPTION,
                colors.text_secondary,
            ),
            (wifi, typography::CAPTION, colors.text_secondary),
            (received, typography::CAPTION, colors.text_secondary),
        ];
        for (index, (content, style, color)) in lines.iter().enumerate() {
            let top = text.top + index as f32 * line;
            draw_text(
                painter,
                content,
                *style,
                Rect::from_ltrb(text.left, top, text.right, top + line),
                Align::Start,
                *color,
            );
        }
    });
}

/// A ring at the right edge of `content` showing `fraction` (0–1) remaining.
fn draw_progress_ring(painter: Painter<'_>, content: Rect, fraction: f32, colors: &SemanticColors) {
    use winisland_render::{Angle, StrokeCap};
    let size = content.height().min(content.width() * 0.4).min(56.0);
    if size < 16.0 {
        return;
    }
    let stroke = (size * 0.1).max(3.0);
    let ring = Rect::from_xywh(
        content.right - size + stroke / 2.0,
        content.center_y() - size / 2.0 + stroke / 2.0,
        size - stroke,
        size - stroke,
    );
    painter.stroke_arc(
        ring,
        Angle::from_degrees(0.0),
        Angle::from_degrees(360.0),
        stroke,
        colors.border_strong,
        StrokeCap::Round,
    );
    let sweep = 360.0 * fraction.clamp(0.0, 1.0);
    if sweep > 0.5 {
        painter.stroke_arc(
            ring,
            Angle::from_degrees(0.0),
            Angle::from_degrees(sweep),
            stroke,
            colors.accent_primary,
            StrokeCap::Round,
        );
    }
}

/// CPU, RAM and disk usage as three labelled rings across `content`.
fn draw_system_rings(painter: Painter<'_>, content: Rect, colors: &SemanticColors) {
    use winisland_render::{Angle, StrokeCap};
    // The system watcher samples all three; the compact widget only samples what it shows.
    let snapshot = crate::core::system_watch::snapshot();
    let metrics = [
        ("CPU", snapshot.cpu),
        ("RAM", snapshot.memory),
        ("DISK", snapshot.disk),
    ];
    let palette = crate::ui::theme::current().chart_palette();
    let slot = content.width() / metrics.len() as f32;
    let label_h = typography::CAPTION.size + space::S1;
    let size = (content.height() - label_h).min(slot - space::S2).max(0.0);
    if size < 18.0 {
        return;
    }
    let stroke = (size * 0.12).max(3.0);
    for (index, &(label, value)) in metrics.iter().enumerate() {
        let center_x = content.left + slot * (index as f32 + 0.5);
        let ring = Rect::from_xywh(
            center_x - size / 2.0 + stroke / 2.0,
            content.top + stroke / 2.0,
            size - stroke,
            size - stroke,
        );
        painter.stroke_arc(
            ring,
            Angle::from_degrees(0.0),
            Angle::from_degrees(360.0),
            stroke,
            colors.border_strong,
            StrokeCap::Round,
        );
        let fraction = value.unwrap_or(0.0).clamp(0.0, 1.0);
        if fraction > 0.0 {
            painter.stroke_arc(
                ring,
                Angle::from_degrees(0.0),
                Angle::from_degrees(360.0 * fraction),
                stroke,
                palette[index % palette.len()],
                StrokeCap::Round,
            );
        }
        // Small rings drop the % sign and use caption type so the number always fits.
        let small = size < 52.0;
        let percent = value.map_or_else(
            || "—".to_string(),
            |value| {
                if small {
                    format!("{:.0}", value * 100.0)
                } else {
                    format!("{:.0}%", value * 100.0)
                }
            },
        );
        draw_text(
            painter,
            &percent,
            if small {
                typography::CAPTION
            } else {
                typography::LABEL
            },
            ring.inset(-4.0),
            Align::Center,
            colors.text_primary,
        );
        draw_text(
            painter,
            label,
            typography::CAPTION,
            Rect::from_xywh(center_x - slot / 2.0, content.top + size, slot, label_h),
            Align::Center,
            colors.text_secondary,
        );
    }
}

const WEATHER_CHART_HEIGHT: f32 = 64.0;

thread_local! {
    static WEATHER_HOURLY: RefCell<Vec<f32>> = const { RefCell::new(Vec::new()) };
}

/// Publishes the next hours' temperatures for the Weather chart.
pub fn set_weather_hourly(temperatures: Vec<f32>) {
    WEATHER_HOURLY.with(|cell| {
        let mut current = cell.borrow_mut();
        if *current != temperatures {
            *current = temperatures;
        }
    });
}

/// A smooth temperature line with a soft fill, plus the high and low labels.
fn draw_temperature_chart(
    painter: Painter<'_>,
    rect: Rect,
    values: &[f32],
    colors: &SemanticColors,
) {
    use winisland_render::{GradientStop, PathBuilder, StrokeCap, StrokeJoin, TileMode};
    let (min, max) = values
        .iter()
        .fold((f32::MAX, f32::MIN), |(min, max), value| {
            (min.min(*value), max.max(*value))
        });
    let range = (max - min).max(1.0);
    let plot = rect.inset(space::S2);
    let step = plot.width() / (values.len() - 1) as f32;
    let points: Vec<Point> = values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            Point::new(
                plot.left + step * index as f32,
                plot.bottom - (value - min) / range * plot.height(),
            )
        })
        .collect();
    // Catmull-Rom style smoothing through the points.
    let mut line = PathBuilder::new();
    line.move_to(points[0]);
    for index in 1..points.len() {
        let previous = points[index - 1];
        let current = points[index];
        let before = points[index.saturating_sub(2)];
        let after = points[(index + 1).min(points.len() - 1)];
        let control1 = Point::new(
            previous.x + (current.x - before.x) / 6.0,
            previous.y + (current.y - before.y) / 6.0,
        );
        let control2 = Point::new(
            current.x - (after.x - previous.x) / 6.0,
            current.y - (after.y - previous.y) / 6.0,
        );
        line.cubic_to(control1, control2, current);
    }
    let stroke = line.detach();
    let mut area = PathBuilder::new();
    area.move_to(Point::new(points[0].x, plot.bottom));
    for point in &points {
        area.line_to(*point);
    }
    area.line_to(Point::new(points[points.len() - 1].x, plot.bottom));
    area.close();
    let accent = colors.accent_primary;
    painter.fill_path_with_gradient(
        &area.detach(),
        Point::new(plot.left, plot.top),
        Point::new(plot.left, plot.bottom),
        &[
            GradientStop {
                offset: 0.0,
                color: accent.with_alpha(70),
            },
            GradientStop {
                offset: 1.0,
                color: accent.with_alpha(0),
            },
        ],
        TileMode::Clamp,
    );
    painter.stroke_path(&stroke, 2.0, accent, StrokeCap::Round, StrokeJoin::Round);
    for (label, y) in [
        (format!("{max:.0}°"), plot.top),
        (format!("{min:.0}°"), plot.bottom - typography::CAPTION.size),
    ] {
        draw_text(
            painter,
            &label,
            typography::CAPTION,
            Rect::from_xywh(rect.right - 40.0, y, 40.0, typography::CAPTION.size + 2.0),
            Align::Start,
            colors.text_secondary,
        );
    }
}

const WEEK_CHART_HEIGHT: f32 = 70.0;

thread_local! {
    static SCREEN_TIME_WEEK: RefCell<Vec<(String, f32)>> = const { RefCell::new(Vec::new()) };
}

/// Publishes the last seven days of screen time, oldest first.
pub fn set_screen_time_week(days: Vec<(String, f32)>) {
    SCREEN_TIME_WEEK.with(|cell| {
        let mut current = cell.borrow_mut();
        if *current != days {
            *current = days;
        }
    });
}

/// Seven rounded bars with day labels; today (the last bar) uses the accent colour.
fn draw_week_chart(
    painter: Painter<'_>,
    rect: Rect,
    days: &[(String, f32)],
    colors: &SemanticColors,
) {
    if days.is_empty() {
        return;
    }
    let label_h = typography::CAPTION.size + space::S1;
    let plot = Rect::from_ltrb(rect.left, rect.top, rect.right, rect.bottom - label_h);
    let max = days
        .iter()
        .map(|(_, minutes)| *minutes)
        .fold(1.0f32, f32::max);
    let slot = plot.width() / days.len() as f32;
    let bar_width = (slot * 0.5).min(18.0);
    for (index, (label, minutes)) in days.iter().enumerate() {
        let center = plot.left + slot * (index as f32 + 0.5);
        let height = (minutes / max * plot.height()).max(3.0);
        let today = index + 1 == days.len();
        painter.fill_round_rect(
            Rect::from_xywh(
                center - bar_width / 2.0,
                plot.bottom - height,
                bar_width,
                height,
            ),
            Radius::uniform(bar_width / 2.0),
            if today {
                colors.accent_primary
            } else {
                colors.border_strong
            },
        );
        draw_text(
            painter,
            label,
            typography::CAPTION,
            Rect::from_xywh(center - slot / 2.0, plot.bottom + 2.0, slot, label_h),
            Align::Center,
            if today {
                colors.text_primary
            } else {
                colors.text_secondary
            },
        );
    }
}

/// Splits the clipboard grid into a list and, when wide enough, a preview pane.
fn clipboard_split(grid: Rect) -> (Rect, Option<Rect>) {
    if grid.width() < 440.0 {
        return (grid, None);
    }
    let list_width = (grid.width() * 0.48).round();
    (
        Rect::from_ltrb(grid.left, grid.top, grid.left + list_width, grid.bottom),
        Some(Rect::from_ltrb(
            grid.left + list_width + space::S3,
            grid.top,
            grid.right,
            grid.bottom,
        )),
    )
}

fn clipboard_preview(item: &winisland_core::clipboard::ClipboardItem) -> String {
    use winisland_core::clipboard::ClipboardBody;
    const MAX_PREVIEW_CHARS: usize = 1_200;
    match &item.body {
        ClipboardBody::Text(text) | ClipboardBody::Link(text) => {
            text.chars().take(MAX_PREVIEW_CHARS).collect()
        }
        ClipboardBody::Files(files) => files
            .iter()
            .map(|file| file.display().to_string())
            .collect::<Vec<_>>()
            .join("\n"),
        ClipboardBody::Image { width, height, .. } => format!("{width} × {height}"),
    }
}

/// A raised pane showing the item's text, wrapped by line, with a copy hint.
fn draw_clipboard_preview(painter: Painter<'_>, pane: Rect, text: &str, colors: &SemanticColors) {
    painter.fill_round_rect(pane, Radius::uniform(radius::CARD), colors.surface_raised);
    painter.stroke_round_rect(
        pane,
        Radius::uniform(radius::CARD),
        1.0,
        colors.border_subtle,
    );
    let inner = pane.inset(space::S3);
    let line_height = typography::BODY.size * 1.45;
    let hint_top = inner.bottom - typography::CAPTION.size;
    let save = painter.save();
    painter.clip_rect(Rect::from_ltrb(
        inner.left,
        inner.top,
        inner.right,
        hint_top - space::S1,
    ));
    let fonts = winisland_render::text::FontManager::global();
    let mut top = inner.top;
    'lines: for paragraph in text.lines() {
        // Greedy word wrap to the pane width.
        let mut line = String::new();
        for word in paragraph.split(' ') {
            let candidate = if line.is_empty() {
                word.to_string()
            } else {
                format!("{line} {word}")
            };
            let width = fonts.measure_text_cached(
                &candidate,
                typography::BODY.size,
                winisland_render::FontStyle::normal(),
            );
            if width > inner.width() && !line.is_empty() {
                draw_text(
                    painter,
                    &line,
                    typography::BODY,
                    Rect::from_xywh(inner.left, top, inner.width(), line_height),
                    Align::Start,
                    colors.text_primary,
                );
                top += line_height;
                if top > hint_top {
                    break 'lines;
                }
                line = word.to_string();
            } else {
                line = candidate;
            }
        }
        draw_text(
            painter,
            &line,
            typography::BODY,
            Rect::from_xywh(inner.left, top, inner.width(), line_height),
            Align::Start,
            colors.text_primary,
        );
        top += line_height;
        if top > hint_top {
            break;
        }
    }
    painter.restore_to(save);
    draw_text(
        painter,
        &tr("clipboard_click_to_copy"),
        typography::CAPTION,
        Rect::from_ltrb(inner.left, hint_top, inner.right, inner.bottom),
        Align::Start,
        colors.text_secondary,
    );
}

fn hit_clip_row(layout: &DashboardLayout, point: Point) -> Option<usize> {
    let (list, _) = clipboard_split(layout.grid);
    if !list.contains(point) {
        return None;
    }
    let count = CLIPBOARD_VIEW.with(|cell| cell.borrow().rows.len());
    (0..count).find(|index| list_row(list, *index, true).bounds.contains(point))
}

/// A clipboard row the user clicked, to copy again.
pub fn take_clipboard_copy() -> Option<usize> {
    CLIPBOARD_COPY.with(Cell::take)
}

/// Whether the Focus timer is counting, for the automatic compact live activity.
pub fn focus_running() -> bool {
    PAGE.with(|page| page.borrow().focus.state() == TimerState::Running)
}

/// Whether the user hid `destination` in Settings (Home is always shown).
pub(crate) fn module_hidden(destination: Destination) -> bool {
    if matches!(destination, Destination::Home | Destination::More) {
        return false;
    }
    let key = destination.label_key();
    HIDDEN_MODULES.with(|cell| cell.borrow().iter().any(|hidden| hidden == key))
        || (!ARRANGING.with(Cell::get) && !workspace_has(key))
}

/// Up to two compact rows with an icon and a title, inside a card. Returns false when
/// there is nothing to show, so the card falls back to its value text.
fn draw_card_rows(
    painter: Painter<'_>,
    content: Rect,
    rows: &[(String, String)],
    icon: crate::ui::components::icon::Icon,
    colors: &SemanticColors,
) -> bool {
    use crate::ui::components::icon::draw_icon;
    if rows.is_empty() {
        return false;
    }
    let row_height = (content.height() / 2.0).min(30.0);
    for (index, (title, detail)) in rows.iter().take(2).enumerate() {
        let row = Rect::from_xywh(
            content.left,
            content.top + index as f32 * row_height,
            content.width(),
            row_height - 3.0,
        );
        painter.fill_round_rect(
            row,
            Radius::uniform(radius::CONTROL),
            colors.surface_overlay,
        );
        let icon_size = 13.0;
        draw_icon(
            painter,
            icon,
            Rect::from_xywh(
                row.left + 8.0,
                row.center_y() - icon_size / 2.0,
                icon_size,
                icon_size,
            ),
            colors.text_secondary,
        );
        let detail_width = if detail.is_empty() { 0.0 } else { 44.0 };
        draw_text(
            painter,
            title,
            typography::LABEL,
            Rect::from_ltrb(
                row.left + 28.0,
                row.top,
                row.right - detail_width - 6.0,
                row.bottom,
            ),
            Align::Start,
            colors.text_primary,
        );
        if !detail.is_empty() {
            draw_text(
                painter,
                detail,
                typography::CAPTION,
                Rect::from_ltrb(
                    row.right - detail_width,
                    row.top,
                    row.right - 6.0,
                    row.bottom,
                ),
                Align::Start,
                colors.text_secondary,
            );
        }
    }
    true
}

fn draw_recent_clips(painter: Painter<'_>, content: Rect, colors: &SemanticColors) -> bool {
    let rows: Vec<(String, String)> = CLIPBOARD_VIEW.with(|view| {
        view.borrow()
            .rows
            .iter()
            .take(2)
            .map(|(title, _)| (title.clone(), String::new()))
            .collect()
    });
    draw_card_rows(
        painter,
        content,
        &rows,
        crate::ui::components::icon::Icon::Clipboard,
        colors,
    )
}

fn draw_recent_notes(painter: Painter<'_>, content: Rect, colors: &SemanticColors) -> bool {
    let rows: Vec<(String, String)> = NOTE_ROWS.with(|cell| {
        cell.borrow()
            .iter()
            .take(2)
            .map(|(_, title, detail)| (title.clone(), detail.clone()))
            .collect()
    });
    draw_card_rows(
        painter,
        content,
        &rows,
        crate::ui::components::icon::Icon::Notes,
        colors,
    )
}

/// Seven small bars along the bottom right of the Screen Time card.
fn draw_mini_week(painter: Painter<'_>, content: Rect, colors: &SemanticColors) {
    let week = SCREEN_TIME_WEEK.with(|cell| cell.borrow().clone());
    if !week.iter().any(|(_, minutes)| *minutes > 0.0) {
        return;
    }
    let max = week
        .iter()
        .map(|(_, minutes)| *minutes)
        .fold(1.0f32, f32::max);
    let bar_width = 5.0;
    let gap = 6.0;
    let height = (content.height() * 0.45).min(26.0);
    let total = week.len() as f32 * (bar_width + gap) - gap;
    let left = content.right - total;
    for (index, (_, minutes)) in week.iter().enumerate() {
        let bar_height = (minutes / max * height).max(3.0);
        let today = index + 1 == week.len();
        painter.fill_round_rect(
            Rect::from_xywh(
                left + index as f32 * (bar_width + gap),
                content.bottom - bar_height,
                bar_width,
                bar_height,
            ),
            Radius::uniform(bar_width / 2.0),
            if today {
                colors.accent_primary
            } else {
                colors.border_strong
            },
        );
    }
}

/// Reset, play/pause and skip buttons centred near the bottom of the Focus page.
fn focus_buttons(grid: Rect) -> [Rect; 3] {
    let size = 40.0;
    let main = 52.0;
    let gap = space::S5;
    let center_y = grid.bottom - main / 2.0 - space::S1;
    let center_x = grid.center_x();
    [
        Rect::from_xywh(
            center_x - main / 2.0 - gap - size,
            center_y - size / 2.0,
            size,
            size,
        ),
        Rect::from_xywh(center_x - main / 2.0, center_y - main / 2.0, main, main),
        Rect::from_xywh(
            center_x + main / 2.0 + gap,
            center_y - size / 2.0,
            size,
            size,
        ),
    ]
}

fn draw_focus_page(
    painter: Painter<'_>,
    grid: Rect,
    focus: Option<&FocusView>,
    hovered: Option<u8>,
    colors: &SemanticColors,
) {
    use crate::ui::components::icon::{Icon, draw_icon};
    let Some(view) = focus else {
        return;
    };
    let buttons = focus_buttons(grid);
    let label = match view.state {
        TimerState::Paused => tr("focus_paused"),
        _ => tr(view.label_key),
    };
    draw_text(
        painter,
        &label,
        typography::LABEL,
        Rect::from_xywh(grid.left, grid.top, grid.width(), 20.0),
        Align::Center,
        colors.accent_primary,
    );
    let time = match view.remaining {
        Some(remaining) => format_clock(remaining, true),
        None => format_clock(view.elapsed, false),
    };
    let big = winisland_render::tokens::TextStyle {
        size: (buttons[1].top - grid.top - 40.0).clamp(28.0, 64.0),
        weight: 600,
        tabular: true,
    };
    let time_rect = Rect::from_ltrb(
        grid.left,
        grid.top + 18.0,
        grid.right,
        buttons[1].top - 14.0,
    );
    draw_text(
        painter,
        &time,
        big,
        time_rect,
        Align::Center,
        colors.text_primary,
    );
    if let Some(progress) = view.progress {
        let track = Rect::from_xywh(grid.center_x() - 90.0, time_rect.bottom - 4.0, 180.0, 4.0);
        painter.fill_round_rect(track, Radius::uniform(2.0), colors.border_strong);
        painter.fill_round_rect(
            Rect::from_xywh(
                track.left,
                track.top,
                track.width() * progress.clamp(0.0, 1.0),
                track.height(),
            ),
            Radius::uniform(2.0),
            colors.accent_primary,
        );
    }
    let running = view.state == TimerState::Running;
    for (index, rect) in buttons.iter().enumerate() {
        let primary = index == 1;
        let fill = if primary {
            colors.accent_primary
        } else if hovered == Some(index as u8) {
            colors.surface_overlay
        } else {
            colors.surface_raised
        };
        painter.fill_round_rect(*rect, Radius::uniform(rect.height() / 2.0), fill);
        let icon_color = if primary {
            colors.surface_base
        } else {
            colors.text_primary
        };
        let inner = rect.inset(rect.width() * 0.3);
        match index {
            0 => draw_icon(painter, Icon::Focus, inner, icon_color),
            1 if running => {
                // Pause: two rounded bars.
                let bar = inner.width() * 0.26;
                for x in [
                    inner.left + inner.width() * 0.18,
                    inner.right - inner.width() * 0.18 - bar,
                ] {
                    painter.fill_round_rect(
                        Rect::from_xywh(x, inner.top, bar, inner.height()),
                        Radius::uniform(bar / 2.0),
                        icon_color,
                    );
                }
            }
            1 => {
                // Play: a triangle nudged right for optical centring.
                let mut path = winisland_render::PathBuilder::new();
                path.move_to(Point::new(inner.left + inner.width() * 0.25, inner.top))
                    .line_to(Point::new(inner.right, inner.center_y()))
                    .line_to(Point::new(inner.left + inner.width() * 0.25, inner.bottom))
                    .close();
                painter.fill_path(&path.detach(), icon_color);
            }
            _ => draw_icon(painter, Icon::More, inner, icon_color),
        }
    }
}

/// Room the battery chip takes in the header, divider included.
const BATTERY_CHIP_WIDTH: f32 = 84.0;

thread_local! {
    /// Battery charge and AC state for the header chip, or `None` without a battery.
    static BATTERY: Cell<Option<(u8, bool)>> = const { Cell::new(None) };
}

/// Publishes the battery state shown in the dashboard header.
pub fn set_battery(state: Option<(u8, bool)>) {
    BATTERY.with(|cell| cell.set(state));
}

/// "74% [▮▮▮ ]" ending at `right`, followed by a thin divider before the header buttons.
fn draw_battery_chip(painter: Painter<'_>, header: Rect, right: f32, colors: &SemanticColors) {
    let Some((level, charging)) = BATTERY.with(Cell::get) else {
        return;
    };
    let fill_color = if charging {
        winisland_render::Rgba::from_argb(255, 52, 199, 89)
    } else if level <= 15 {
        winisland_render::Rgba::from_argb(255, 255, 69, 58)
    } else {
        colors.text_primary
    };
    painter.fill_rect(
        Rect::from_xywh(right - 1.0, header.center_y() - 8.0, 1.0, 16.0),
        colors.border_strong,
    );
    let body = Rect::from_xywh(
        right - space::S2 - 26.0,
        header.center_y() - 6.0,
        22.0,
        12.0,
    );
    painter.stroke_round_rect(body, Radius::uniform(3.5), 1.2, colors.text_secondary);
    painter.fill_round_rect(
        Rect::from_xywh(body.right + 1.0, body.center_y() - 2.5, 2.0, 5.0),
        Radius::uniform(1.0),
        colors.text_secondary,
    );
    let inner = Rect::from_xywh(
        body.left + 2.0,
        body.top + 2.0,
        ((body.width() - 4.0) * f32::from(level.min(100)) / 100.0).max(2.0),
        body.height() - 4.0,
    );
    painter.fill_round_rect(inner, Radius::uniform(2.0), fill_color);
    let label = format!("{level}%");
    let label_width = winisland_render::text::FontManager::global().measure_text_cached(
        &label,
        typography::LABEL.size,
        winisland_render::FontStyle::normal(),
    );
    draw_text(
        painter,
        &label,
        typography::LABEL,
        Rect::from_xywh(
            body.left - space::S1 - label_width - 2.0,
            header.top,
            label_width + 2.0,
            header.height(),
        ),
        Align::Start,
        colors.text_primary,
    );
}

thread_local! {
    /// Workspaces as `(name, module keys)`, mirrored from the config.
    static WORKSPACES: RefCell<Vec<(String, Vec<String>, Vec<(String, usize)>)>> =
        const { RefCell::new(Vec::new()) };
    static ACTIVE_WORKSPACE: Cell<usize> = const { Cell::new(0) };
    static ARRANGING: Cell<bool> = const { Cell::new(false) };
    /// Set when the user changed workspaces here and the app should save them.
    static WORKSPACES_DIRTY: Cell<bool> = const { Cell::new(false) };
}

/// Publishes the configured workspaces. Ignored while local edits wait to be saved.
pub fn set_workspaces(workspaces: &[winisland_core::config::Workspace], active: usize) {
    if WORKSPACES_DIRTY.with(Cell::get) {
        return;
    }
    WORKSPACES.with(|cell| {
        let incoming: Vec<(String, Vec<String>, Vec<(String, usize)>)> = workspaces
            .iter()
            .map(|space| {
                (
                    space.name.clone(),
                    space.modules.clone(),
                    space.cards.clone(),
                )
            })
            .collect();
        if *cell.borrow() != incoming {
            *cell.borrow_mut() = incoming;
        }
    });
    ACTIVE_WORKSPACE.with(|cell| cell.set(active.min(workspaces.len().saturating_sub(1))));
}

/// Workspaces edited in the dashboard, once, for the app to save to the config.
pub fn take_workspace_change() -> Option<(Vec<winisland_core::config::Workspace>, usize)> {
    if !WORKSPACES_DIRTY.with(|cell| cell.replace(false)) {
        return None;
    }
    let spaces = WORKSPACES.with(|cell| {
        cell.borrow()
            .iter()
            .map(|(name, modules, cards)| winisland_core::config::Workspace {
                name: name.clone(),
                modules: modules.clone(),
                cards: cards.clone(),
            })
            .collect()
    });
    Some((spaces, ACTIVE_WORKSPACE.with(Cell::get)))
}

fn workspace_has(key: &str) -> bool {
    WORKSPACES.with(|cell| {
        let spaces = cell.borrow();
        let active = ACTIVE_WORKSPACE.with(Cell::get);
        spaces
            .get(active)
            .is_none_or(|(_, modules, _)| modules.iter().any(|module| module == key))
    })
}

/// Header geometry: `(rect, hit)` for each workspace tab, then "+", then Arrange/Done.
fn workspace_tabs(header: Rect, active: Destination) -> Vec<(Rect, DashboardHit)> {
    // Workspaces belong to Home; module pages keep the header for their own title.
    if active != Destination::Home {
        return Vec::new();
    }
    let fonts = winisland_render::text::FontManager::global();
    let icon_size = 18.0;
    let title = tr(active.label_key());
    let title_width = fonts.measure_text_cached(
        &title,
        typography::SECTION_TITLE.size,
        winisland_render::FontStyle::bold(),
    );
    let mut x = if active == Destination::Home {
        // After the robot in the corner.
        header.left + header.height().min(28.0) + space::S2 - 8.0
    } else {
        header.left + icon_size + space::S2 + title_width + space::S3 * 2.0 + 1.0
    };
    let mut tabs = Vec::new();
    let names: Vec<String> = WORKSPACES.with(|cell| {
        cell.borrow()
            .iter()
            .map(|(name, ..)| name.clone())
            .collect()
    });
    // One chip for the current mode; a click moves on to the next one.
    if !names.is_empty() {
        let current = ACTIVE_WORKSPACE.with(Cell::get).min(names.len() - 1);
        let width = fonts.measure_text_cached(
            &names[current],
            typography::SECTION_TITLE.size,
            winisland_render::FontStyle::bold(),
        ) + 40.0;
        tabs.push((
            Rect::from_xywh(x + 8.0, header.top + 4.0, width, header.height() - 8.0),
            DashboardHit::Workspace(((current + 1) % names.len()) as u8),
        ));
        x += width + 12.0;
    }
    tabs.push((
        Rect::from_xywh(x, header.top + 3.0, 24.0, header.height() - 6.0),
        DashboardHit::AddWorkspace,
    ));
    x += 28.0;
    let arrange_label = if ARRANGING.with(Cell::get) {
        "Done"
    } else {
        "Arrange"
    };
    let width = fonts.measure_text_cached(
        arrange_label,
        typography::LABEL.size,
        winisland_render::FontStyle::bold(),
    ) + 18.0;
    tabs.push((
        Rect::from_xywh(x, header.center_y() - 11.0, width, 22.0),
        DashboardHit::Arrange,
    ));
    tabs
}

const WORKSPACE_ACCENT: winisland_render::Rgba = winisland_render::Rgba::from_rgb(255, 176, 32);

fn draw_workspace_tabs(
    painter: Painter<'_>,
    header: Rect,
    active: Destination,
    hovered: Option<DashboardHit>,
    colors: &SemanticColors,
) {
    let names: Vec<String> = WORKSPACES.with(|cell| {
        cell.borrow()
            .iter()
            .map(|(name, ..)| name.clone())
            .collect()
    });
    let current = ACTIVE_WORKSPACE.with(Cell::get);
    let arranging = ARRANGING.with(Cell::get);
    for (rect, hit) in workspace_tabs(header, active) {
        let hover = hovered == Some(hit);
        match hit {
            DashboardHit::Workspace(_) => {
                // The mode chip: the current mode and a small chevron, on a raised pill.
                let name = names.get(current).map_or("", String::as_str);
                painter.fill_round_rect(
                    rect,
                    Radius::uniform(rect.height() / 2.0),
                    if hover {
                        colors.surface_overlay
                    } else {
                        colors.surface_raised
                    },
                );
                painter.stroke_round_rect(
                    rect,
                    Radius::uniform(rect.height() / 2.0),
                    1.0,
                    colors.border_subtle,
                );
                draw_text(
                    painter,
                    name,
                    typography::SECTION_TITLE,
                    Rect::from_ltrb(rect.left + 12.0, rect.top, rect.right - 24.0, rect.bottom),
                    Align::Start,
                    colors.text_primary,
                );
                let chevron = Point::new(rect.right - 15.0, rect.center_y());
                for side in [-1.0f32, 1.0] {
                    painter.stroke_line(
                        Point::new(chevron.x + side * 3.5, chevron.y - 1.8),
                        Point::new(chevron.x, chevron.y + 1.8),
                        1.5,
                        colors.text_secondary,
                        winisland_render::StrokeCap::Round,
                    );
                }
            }
            DashboardHit::AddWorkspace => {
                if hover {
                    painter.fill_round_rect(rect, Radius::uniform(7.0), colors.surface_overlay);
                }
                crate::ui::components::icon::draw_icon(
                    painter,
                    crate::ui::components::icon::Icon::Plus,
                    rect.inset(5.0),
                    colors.text_secondary,
                );
            }
            DashboardHit::Arrange => {
                let (fill, text) = if arranging {
                    (WORKSPACE_ACCENT, winisland_render::Rgba::BLACK)
                } else if hover {
                    (colors.surface_overlay, colors.text_primary)
                } else {
                    (winisland_render::Rgba::TRANSPARENT, colors.text_disabled)
                };
                painter.fill_round_rect(rect, Radius::uniform(rect.height() / 2.0), fill);
                draw_text(
                    painter,
                    if arranging { "Done" } else { "Arrange" },
                    typography::LABEL,
                    rect.inset(9.0),
                    Align::Start,
                    text,
                );
            }
            _ => {}
        }
    }
}

/// In arrange mode, marks each dock module: a filled check when it is in the workspace, a
/// "+" when it can be added. Clicking toggles it.
fn draw_arrange_badges(painter: Painter<'_>, items: &[Rect], destinations: &[Destination]) {
    if !ARRANGING.with(Cell::get) {
        return;
    }
    for (rect, destination) in items.iter().zip(destinations) {
        if matches!(destination, Destination::Home | Destination::More) {
            continue;
        }
        let member = workspace_has(destination.label_key());
        let badge = Rect::from_xywh(rect.right - 13.0, rect.top - 2.0, 15.0, 15.0);
        if member {
            painter.fill_circle(
                Point::new(badge.center_x(), badge.center_y()),
                7.5,
                WORKSPACE_ACCENT,
            );
        } else {
            painter.stroke_circle(
                Point::new(badge.center_x(), badge.center_y()),
                7.0,
                1.5,
                WORKSPACE_ACCENT,
            );
        }
        crate::ui::components::icon::draw_icon(
            painter,
            if member {
                crate::ui::components::icon::Icon::Check
            } else {
                crate::ui::components::icon::Icon::Plus
            },
            badge.inset(3.0),
            if member {
                winisland_render::Rgba::BLACK
            } else {
                WORKSPACE_ACCENT
            },
        );
    }
}

/// Applies a workspace-related click. Returns `true` when handled.
fn apply_workspace_hit(hit: DashboardHit, page: &mut PageState) -> bool {
    match hit {
        DashboardHit::Workspace(index) => {
            ACTIVE_WORKSPACE.with(|cell| cell.set(usize::from(index)));
            page.active = Destination::Home;
            page.shown_at = Instant::now();
        }
        DashboardHit::AddWorkspace => {
            WORKSPACES.with(|cell| {
                let mut spaces = cell.borrow_mut();
                let name = format!("Space {}", spaces.len() + 1);
                spaces.push((
                    name,
                    vec!["dashboard_agenda".into(), "dashboard_notes".into()],
                    Vec::new(),
                ));
                ACTIVE_WORKSPACE.with(|active| active.set(spaces.len() - 1));
            });
            ARRANGING.with(|cell| cell.set(true));
        }
        DashboardHit::Arrange => {
            ARRANGING.with(|cell| cell.set(!cell.get()));
            return true;
        }
        DashboardHit::Destination(destination)
            if ARRANGING.with(Cell::get)
                && !matches!(destination, Destination::Home | Destination::More) =>
        {
            let key = destination.label_key();
            WORKSPACES.with(|cell| {
                let mut spaces = cell.borrow_mut();
                let active = ACTIVE_WORKSPACE.with(Cell::get);
                if let Some((_, modules, _)) = spaces.get_mut(active) {
                    if let Some(at) = modules.iter().position(|module| module == key) {
                        modules.remove(at);
                    } else {
                        modules.push(key.to_string());
                    }
                }
            });
        }
        _ => return false,
    }
    WORKSPACES_DIRTY.with(|cell| cell.set(true));
    true
}

thread_local! {
    /// The Home cards for the active workspace. Leaked once per layout change so callers can
    /// keep borrowing a `'static` slice; layouts change only on user edits.
    static HOME_LAYOUT: RefCell<(Vec<(String, usize)>, &'static [SampleCard])> =
        const { RefCell::new((Vec::new(), &HOME_CARDS)) };
}

fn card_key(kind: CardKind) -> &'static str {
    match kind {
        CardKind::ScreenTime => "screen_time",
        CardKind::Weather => "weather",
        CardKind::Focus => "focus",
        CardKind::Clipboard => "clipboard",
        CardKind::Notes => "notes",
        CardKind::System => "system",
    }
}

/// The active workspace's card spec, or the default cards when it has none.
fn active_card_spec() -> Vec<(String, usize)> {
    let spec = WORKSPACES.with(|cell| {
        let spaces = cell.borrow();
        spaces
            .get(ACTIVE_WORKSPACE.with(Cell::get))
            .map(|(_, _, cards)| cards.clone())
            .unwrap_or_default()
    });
    if spec.is_empty() {
        HOME_CARDS
            .iter()
            .map(|card| (card_key(card.kind).to_string(), card.column_span))
            .collect()
    } else {
        spec
    }
}

fn home_cards() -> &'static [SampleCard] {
    let spec = active_card_spec();
    HOME_LAYOUT.with(|cell| {
        let mut layout = cell.borrow_mut();
        if layout.0 != spec {
            let cards: Vec<SampleCard> = spec
                .iter()
                .filter_map(|(key, span)| {
                    HOME_CARDS
                        .iter()
                        .find(|card| card_key(card.kind) == key)
                        .map(|card| SampleCard {
                            column_span: (*span).clamp(1, 2),
                            ..*card
                        })
                })
                .collect();
            layout.1 = Box::leak(cards.into_boxed_slice());
            layout.0 = spec;
        }
        layout.1
    })
}

/// Badge rectangles on a card in arrange mode: move (top left), resize and remove (top right).
fn card_badges(card: Rect) -> [(Rect, u8); 3] {
    let size = 20.0;
    [
        (
            Rect::from_xywh(card.left + 6.0, card.top + 6.0, size, size),
            0,
        ),
        (
            Rect::from_xywh(card.right - 2.0 * size - 10.0, card.top + 6.0, size, size),
            1,
        ),
        (
            Rect::from_xywh(card.right - size - 6.0, card.top + 6.0, size, size),
            2,
        ),
    ]
}

fn hit_card_badge(cards: &[Rect], point: Point) -> Option<DashboardHit> {
    if !ARRANGING.with(Cell::get) {
        return None;
    }
    for (index, card) in cards.iter().enumerate() {
        for (rect, kind) in card_badges(*card) {
            if rect.contains(point) {
                return Some(match kind {
                    0 => DashboardHit::CardMove(index),
                    1 => DashboardHit::CardResize(index),
                    _ => DashboardHit::CardRemove(index),
                });
            }
        }
    }
    None
}

fn draw_card_badges(painter: Painter<'_>, cards: &[Rect]) {
    if !ARRANGING.with(Cell::get) {
        return;
    }
    for card in cards {
        painter.stroke_round_rect(
            *card,
            Radius::uniform(16.0),
            1.5,
            WORKSPACE_ACCENT.with_alpha(160),
        );
        for (rect, kind) in card_badges(*card) {
            let fill = if kind == 2 {
                winisland_render::Rgba::from_rgb(255, 69, 58)
            } else {
                winisland_render::Rgba::from_rgb(58, 58, 60)
            };
            painter.fill_circle(
                Point::new(rect.center_x(), rect.center_y()),
                rect.width() / 2.0,
                fill,
            );
            let label = match kind {
                0 => "←",
                1 => "↔",
                _ => "✕",
            };
            draw_text(
                painter,
                label,
                typography::LABEL,
                rect,
                Align::Center,
                winisland_render::Rgba::WHITE,
            );
        }
    }
}

/// Applies a card badge: edits the active workspace's card list and marks it for saving.
fn apply_card_badge(hit: DashboardHit) -> bool {
    let (index, op) = match hit {
        DashboardHit::CardMove(index) => (index, 0),
        DashboardHit::CardResize(index) => (index, 1),
        DashboardHit::CardRemove(index) => (index, 2),
        _ => return false,
    };
    let mut spec = active_card_spec();
    if index >= spec.len() {
        return true;
    }
    match op {
        0 if index > 0 => spec.swap(index, index - 1),
        1 => spec[index].1 = if spec[index].1 >= 2 { 1 } else { 2 },
        2 if spec.len() > 1 => {
            spec.remove(index);
        }
        _ => {}
    }
    WORKSPACES.with(|cell| {
        if let Some((_, _, cards)) = cell.borrow_mut().get_mut(ACTIVE_WORKSPACE.with(Cell::get)) {
            *cards = spec;
        }
    });
    WORKSPACES_DIRTY.with(|cell| cell.set(true));
    true
}

/// `2123.38` as `2,123.38`.
fn thousands(value: f64) -> String {
    let text = format!("{value:.2}");
    let (whole, cents) = text.split_once('.').unwrap_or((&text, "00"));
    let mut grouped = String::new();
    for (index, digit) in whole.chars().enumerate() {
        if index > 0 && (whole.len() - index) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    format!("{grouped}.{cents}")
}

/// The mode that fits now: Code while an editor or terminal is in front, Work on
/// weekdays from 9 to 6 or while a meeting or mail app is in front, Home otherwise.
pub fn mode_for(foreground_executable: &str, weekday: u16, hour: u16) -> &'static str {
    const CODE: [&str; 9] = [
        "code",
        "cursor",
        "devenv",
        "idea64",
        "windowsterminal",
        "wt",
        "powershell",
        "pwsh",
        "cmd",
    ];
    const WORK: [&str; 5] = ["ms-teams", "teams", "olk", "outlook", "zoom"];
    let app = foreground_executable
        .trim_end_matches(".exe")
        .to_ascii_lowercase();
    if CODE.contains(&app.as_str()) {
        "code"
    } else if WORK.contains(&app.as_str())
        || ((1..=5).contains(&weekday) && (9..18).contains(&hour))
    {
        "work"
    } else {
        "home"
    }
}

/// The current track for the Now playing page, or `None` when nothing is loaded.
pub fn set_media(media: Option<&crate::core::smtc::MediaInfo>) {
    MEDIA.with(|cell| *cell.borrow_mut() = media.cloned());
}

/// A Now playing button press: 0 previous, 1 play or pause, 2 next.
pub fn take_media_request() -> Option<u8> {
    MEDIA_REQUEST.with(Cell::take)
}

/// A click on the progress bar, as thousandths of the track.
pub fn take_media_seek() -> Option<u16> {
    MEDIA_SEEK.with(Cell::take)
}

/// Where everything on the Now playing page sits.
struct MediaLayout {
    /// The music card and the week card beside it.
    player: Rect,
    week: Rect,
    art: Rect,
    /// Title, artist and controls sit right of the art.
    text_left: f32,
    bar: Rect,
    buttons: [Rect; 3],
}

fn media_layout(grid: Rect) -> MediaLayout {
    const PAD: f32 = 16.0;
    let split = grid.left + grid.width() * 0.64;
    let player = Rect::from_ltrb(grid.left, grid.top, split - space::S2, grid.bottom);
    let week = Rect::from_ltrb(split + space::S2, grid.top, grid.right, grid.bottom);
    let side = (player.height() - PAD * 2.0).clamp(64.0, 132.0);
    let art = Rect::from_xywh(
        player.left + PAD,
        player.center_y() - side / 2.0,
        side,
        side,
    );
    let text_left = art.right + PAD + 4.0;
    let text_right = player.right - PAD - 4.0;
    let bar = Rect::from_ltrb(text_left, art.top + 64.0, text_right, art.top + 68.0);
    const BUTTON: f32 = 36.0;
    let row_center = (text_left + text_right) / 2.0;
    let row_top = (bar.bottom + 28.0).min(player.bottom - PAD - BUTTON);
    let buttons = [-1.0f32, 0.0, 1.0].map(|step| {
        let size = if step == 0.0 { BUTTON + 4.0 } else { BUTTON };
        Rect::from_xywh(
            row_center + step * 56.0 - size / 2.0,
            row_top + (BUTTON - size) / 2.0,
            size,
            size,
        )
    });
    MediaLayout {
        player,
        week,
        art,
        text_left,
        bar,
        buttons,
    }
}

/// `m:ss` for a position in milliseconds.
fn track_time(ms: u64) -> String {
    let seconds = ms / 1000;
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

/// A plain card surface: raised fill and a hairline.
fn page_card(painter: Painter<'_>, rect: Rect, colors: &SemanticColors) {
    let shape = Radius::uniform(radius::CARD);
    painter.fill_round_rect(rect, shape, colors.surface_raised);
    painter.stroke_round_rect(rect, shape, 1.0, colors.border_subtle);
}

/// Artwork, title, artist in the accent, a progress bar with times and the controls, with
/// this week and what is next beside it.
fn draw_media_page(painter: Painter<'_>, grid: Rect, colors: &SemanticColors) {
    let layout = media_layout(grid);
    let media = MEDIA.with(|cell| cell.borrow().clone());
    page_card(painter, layout.player, colors);
    page_card(painter, layout.week, colors);
    draw_week_card(painter, layout.week, colors);
    let Some(media) = media.filter(|media| !media.title.is_empty()) else {
        draw_text(
            painter,
            &tr("media_nothing_playing"),
            typography::BODY,
            layout.player,
            Align::Center,
            colors.text_secondary,
        );
        return;
    };

    // Artwork, centre-cropped into a rounded square; a quiet tile when there is none.
    let art_shape = Radius::uniform(radius::CARD - 2.0);
    match crate::ui::expanded::music_view::get_cached_media_image(&media) {
        Some(image) => {
            let edge = (image.width() as f32).min(image.height() as f32);
            let source = Rect::from_xywh(
                (image.width() as f32 - edge) / 2.0,
                (image.height() as f32 - edge) / 2.0,
                edge,
                edge,
            );
            painter.save();
            painter.clip_round_rect(layout.art, art_shape);
            painter.draw_image(
                &image,
                layout.art,
                &winisland_render::ImageOptions::default()
                    .with_sampling(winisland_render::Sampling::LinearLinear)
                    .with_src(source),
            );
            painter.restore();
        }
        None => {
            painter.fill_round_rect(layout.art, art_shape, colors.surface_interactive);
            crate::ui::components::icon::draw_icon(
                painter,
                crate::ui::components::icon::Icon::Music,
                layout.art.inset(layout.art.width() * 0.34),
                colors.text_secondary,
            );
        }
    }

    let text_width = layout.bar.right - layout.text_left;
    draw_text(
        painter,
        &media.title,
        typography::SECTION_TITLE,
        Rect::from_xywh(layout.text_left, layout.art.top + 4.0, text_width, 22.0),
        Align::Start,
        colors.text_primary,
    );
    draw_text(
        painter,
        &media.artist,
        typography::BODY,
        Rect::from_xywh(layout.text_left, layout.art.top + 28.0, text_width, 20.0),
        Align::Start,
        colors.accent_primary,
    );

    // Progress, moving on between updates while playing.
    let duration = media.duration_ms.max(1);
    let position = if media.is_playing {
        media.position_ms + media.last_update.elapsed().as_millis() as u64
    } else {
        media.position_ms
    }
    .min(duration);
    let fraction = position as f32 / duration as f32;
    let track = Radius::uniform(layout.bar.height() / 2.0);
    painter.fill_round_rect(layout.bar, track, colors.surface_interactive);
    painter.fill_round_rect(
        Rect::from_xywh(
            layout.bar.left,
            layout.bar.top,
            layout.bar.width() * fraction,
            layout.bar.height(),
        ),
        track,
        colors.text_primary,
    );
    let times_top = layout.bar.bottom + 4.0;
    draw_text(
        painter,
        &track_time(position),
        typography::CAPTION,
        Rect::from_xywh(layout.bar.left, times_top, layout.bar.width(), 16.0),
        Align::Start,
        colors.text_secondary,
    );
    let total = track_time(duration);
    let total_width = winisland_render::text::FontManager::global().measure_text_cached(
        &total,
        typography::CAPTION.size,
        winisland_render::FontStyle::normal(),
    );
    draw_text(
        painter,
        &total,
        typography::CAPTION,
        Rect::from_xywh(
            layout.bar.right - total_width - 2.0,
            times_top,
            total_width + 4.0,
            16.0,
        ),
        Align::Start,
        colors.text_secondary,
    );

    // Previous, play or pause, next: the middle one is the solid primary.
    let glyphs = [
        "M7 6v12M18 6l-8 6 8 6z",
        if media.is_playing {
            "M9 6v12M15 6v12"
        } else {
            "M9 6l9 6-9 6z"
        },
        "M17 6v12M6 6l8 6-8 6z",
    ];
    for (index, (rect, svg)) in layout.buttons.iter().zip(glyphs).enumerate() {
        let main = index == 1;
        if main {
            painter.fill_circle(
                Point::new(rect.center_x(), rect.center_y()),
                rect.width() / 2.0,
                colors.text_primary,
            );
        }
        let ink = if main {
            colors.surface_base
        } else {
            colors.text_primary
        };
        if let Some(path) = winisland_render::Path::from_svg(svg) {
            let size = 16.0;
            painter.save();
            painter.translate(Vec2::new(
                rect.center_x() - size / 2.0,
                rect.center_y() - size / 2.0,
            ));
            painter.scale(Vec2::new(size / 24.0, size / 24.0));
            painter.fill_path(&path, ink);
            painter.stroke_path(
                &path,
                2.2,
                ink,
                winisland_render::StrokeCap::Round,
                winisland_render::StrokeJoin::Round,
            );
            painter.restore();
        }
    }
}

/// Days since 1970-01-01 back to `(year, month, day)`.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let mp = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let month = i64::from(month);
    let day_of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// This week, Monday to Sunday, with today circled in the accent, and the next event or a
/// calm empty line.
fn draw_week_card(painter: Painter<'_>, card: Rect, colors: &SemanticColors) {
    const PAD: f32 = 16.0;
    let now = crate::platform::shell().local_datetime();
    let today = days_from_civil(
        i64::from(now.year),
        u32::from(now.month),
        u32::from(now.day),
    );
    let from_monday = (i64::from(now.day_of_week) + 6) % 7;
    draw_text(
        painter,
        &tr("media_this_week"),
        typography::LABEL,
        Rect::from_xywh(
            card.left + PAD,
            card.top + PAD - 4.0,
            card.width() - PAD * 2.0,
            18.0,
        ),
        Align::Start,
        colors.text_secondary,
    );
    const INITIALS: [&str; 7] = ["M", "T", "W", "T", "F", "S", "S"];
    let cell = (card.width() - PAD * 2.0) / 7.0;
    let row = card.top + PAD + 22.0;
    for index in 0..7 {
        let (_, _, day) = civil_from_days(today - from_monday + index);
        let center = card.left + PAD + cell * (index as f32 + 0.5);
        let is_today = index == from_monday;
        draw_text(
            painter,
            INITIALS[index as usize],
            typography::CAPTION,
            Rect::from_xywh(center - cell / 2.0, row, cell, 14.0),
            Align::Center,
            colors.text_secondary,
        );
        if is_today {
            painter.fill_circle(Point::new(center, row + 29.0), 12.0, colors.accent_primary);
        }
        draw_text(
            painter,
            &day.to_string(),
            typography::LABEL,
            Rect::from_xywh(center - cell / 2.0, row + 20.0, cell, 18.0),
            Align::Center,
            if is_today {
                colors.surface_base
            } else {
                colors.text_primary
            },
        );
    }
    let next = AGENDA_ROWS.with(|cell| cell.borrow().first().cloned());
    let (title, detail) = next.unwrap_or_else(|| (tr("agenda_free_day"), String::new()));
    let lines = Rect::from_xywh(card.left + PAD, row + 52.0, card.width() - PAD * 2.0, 20.0);
    draw_text(
        painter,
        &title,
        typography::BODY,
        lines,
        Align::Start,
        colors.text_primary,
    );
    draw_text(
        painter,
        &detail,
        typography::CAPTION,
        Rect::from_xywh(lines.left, lines.bottom + 2.0, lines.width(), 16.0),
        Align::Start,
        colors.text_secondary,
    );
}

thread_local! {
    /// Today's total and the apps behind it, in minutes, most used first.
    static SCREEN_TIME_APPS: RefCell<Option<(u64, Vec<(String, u64)>)>> = const { RefCell::new(None) };
}

/// Today's screen time for the Screen time page; `None` while tracking is off.
pub fn set_screen_time_apps(today: Option<(u64, Vec<(String, u64)>)>) {
    SCREEN_TIME_APPS.with(|cell| *cell.borrow_mut() = today);
}

/// `2h 31m`, or `45m` under an hour.
fn hours_minutes(minutes: u64) -> String {
    if minutes >= 60 {
        format!("{}h {}m", minutes / 60, minutes % 60)
    } else {
        format!("{minutes}m")
    }
}

/// Today's total and the week on the left, the apps behind it on the right.
fn draw_screen_time_page(painter: Painter<'_>, grid: Rect, colors: &SemanticColors) {
    const PAD: f32 = 16.0;
    let Some((total, apps)) = SCREEN_TIME_APPS.with(|cell| cell.borrow().clone()) else {
        draw_text(
            painter,
            &tr("screen_time_turn_on"),
            typography::BODY,
            grid,
            Align::Center,
            colors.text_secondary,
        );
        return;
    };
    let split = grid.left + grid.width() * 0.56;
    let today = Rect::from_ltrb(grid.left, grid.top, split - space::S2, grid.bottom);
    let by_app = Rect::from_ltrb(split + space::S2, grid.top, grid.right, grid.bottom);
    page_card(painter, today, colors);
    page_card(painter, by_app, colors);

    draw_text(
        painter,
        &hours_minutes(total),
        typography::DISPLAY,
        Rect::from_xywh(
            today.left + PAD,
            today.top + PAD - 2.0,
            today.width() / 2.0,
            34.0,
        ),
        Align::Start,
        colors.text_primary,
    );
    draw_text(
        painter,
        &winisland_core::i18n::tr_args("screen_time_apps_used", &[&apps.len().to_string()]),
        typography::CAPTION,
        Rect::from_xywh(
            today.left + PAD,
            today.top + PAD + 32.0,
            today.width() - PAD * 2.0,
            16.0,
        ),
        Align::Start,
        colors.text_secondary,
    );
    let week = SCREEN_TIME_WEEK.with(|cell| cell.borrow().clone());
    if week.iter().any(|(_, minutes)| *minutes > 0.0) {
        let chart = Rect::from_ltrb(
            today.left + PAD,
            (today.bottom - PAD - WEEK_CHART_HEIGHT).max(today.top + PAD + 56.0),
            today.right - PAD,
            today.bottom - PAD,
        );
        draw_week_chart(painter, chart, &week, colors);
    }

    draw_text(
        painter,
        &tr("media_by_app"),
        typography::LABEL,
        Rect::from_xywh(
            by_app.left + PAD,
            by_app.top + PAD - 4.0,
            by_app.width() - PAD * 2.0,
            18.0,
        ),
        Align::Start,
        colors.text_secondary,
    );
    // Each app: its name and time, then a thin bar for its share of the day.
    const ROW: f32 = 34.0;
    let rows_top = by_app.top + PAD + 22.0;
    let fits = ((by_app.bottom - PAD - rows_top) / ROW).floor().max(0.0) as usize;
    let width = by_app.width() - PAD * 2.0;
    for (index, (name, minutes)) in apps.iter().take(fits).enumerate() {
        let top = rows_top + index as f32 * ROW;
        let time = hours_minutes(*minutes);
        let time_width = winisland_render::text::FontManager::global().measure_text_cached(
            &time,
            typography::LABEL.size,
            winisland_render::FontStyle::normal(),
        );
        draw_text(
            painter,
            name,
            typography::LABEL,
            Rect::from_xywh(by_app.left + PAD, top, width - time_width - space::S2, 18.0),
            Align::Start,
            colors.text_primary,
        );
        draw_text(
            painter,
            &time,
            typography::LABEL,
            Rect::from_xywh(
                by_app.right - PAD - time_width - 2.0,
                top,
                time_width + 4.0,
                18.0,
            ),
            Align::Start,
            colors.text_secondary,
        );
        let share = if total == 0 {
            0.0
        } else {
            (*minutes as f32 / total as f32).clamp(0.0, 1.0)
        };
        let bar = Rect::from_xywh(by_app.left + PAD, top + 21.0, width, 3.0);
        painter.fill_round_rect(bar, Radius::uniform(1.5), colors.surface_interactive);
        painter.fill_round_rect(
            Rect::from_xywh(bar.left, bar.top, bar.width() * share, bar.height()),
            Radius::uniform(1.5),
            if index == 0 {
                colors.accent_primary
            } else {
                colors.text_secondary
            },
        );
    }
}

/// A round play button at the right of a card, for a timer that is not running.
fn draw_start_button(painter: Painter<'_>, content: Rect, colors: &SemanticColors) {
    let size = content.height().min(44.0);
    if size < 20.0 {
        return;
    }
    let center = Point::new(content.right - size / 2.0, content.bottom - size / 2.0);
    painter.fill_circle(center, size / 2.0, colors.surface_interactive);
    let half = size * 0.18;
    if let Some(path) = winisland_render::Path::from_svg(&format!(
        "M{} {} L{} {} L{} {} Z",
        center.x - half * 0.7,
        center.y - half,
        center.x + half,
        center.y,
        center.x - half * 0.7,
        center.y + half
    )) {
        painter.fill_path(&path, colors.text_primary);
    }
}
