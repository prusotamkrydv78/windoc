//! Shelf tab: files parked by dropping them on the island, in two panes.
//!
//! Left: filter tabs (All · Images · Docs · Other) or a search field, then the files grouped
//! by when they were parked (Today / Yesterday / Earlier) with thumbnails or type badges.
//! Right: the selected file's preview, details and actions (Open, Show in folder, Copy,
//! Convert, Remove), plus the shelf switcher. Shelf data lives in the app's `ShelfService`;
//! this tab reads the snapshot the frame loop publishes (`sync_shelf`), removes entries
//! through the dashboard's one-per-frame removal cell, and queues adds, new shelves and
//! shelf switches through `core::shelf_store::request_*`.

pub(super) mod clip;
pub(super) mod kit;
pub(super) mod thumbs;

use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use winisland_platform::Key;
use winisland_render::tokens::{SemanticColors, TextStyle, typography};
use winisland_render::{ImageFit, ImageOptions, Painter, Point, Radius, Rect, Sampling};

use super::TabContext;
use crate::ui::dashboard::{DashboardHit, Destination};
use kit::{format_bytes, style, text, text_center, text_width};

const ID_ADD: u16 = 600;
const ID_SEARCH: u16 = 601;
const ID_SEARCH_CLOSE: u16 = 602;
const ID_SHELF: u16 = 603;
const ID_NEW_SHELF: u16 = 604;
const ID_REMOVE: u16 = 605;
const ID_OPEN: u16 = 606;
const ID_REVEAL: u16 = 607;
const ID_COPY: u16 = 608;
const ID_CONVERT: u16 = 609;
const ID_FILTER: u16 = 610;
/// Swallows clicks on empty parts of the page so the legacy page underneath never acts.
const ID_INERT: u16 = 619;
const ID_ROW: u16 = 620;
const MAX_ROW_IDS: usize = 80;

const TAB_ROW: f32 = 28.0;
const ROW_HEIGHT: f32 = 42.0;
const HEADER_HEIGHT: f32 = 22.0;
const BUTTON: f32 = 28.0;
const FOOTER: f32 = 32.0;
const OPEN_WIDTH: f32 = 78.0;
const FEEDBACK: Duration = Duration::from_millis(1600);

const FILTERS: [&str; 4] = ["All", "Images", "Docs", "Other"];
const TAB_LABEL: TextStyle = typography::BODY;
const TAB_COUNT: TextStyle = typography::LABEL;
const PILL: TextStyle = style(13.0, 600);

#[derive(Clone)]
struct Item {
    id: u64,
    name: String,
    path: PathBuf,
    exists: bool,
    is_dir: bool,
    bytes: u64,
    added: Option<SystemTime>,
    /// Position on the shelf (the order files were added in).
    order: usize,
    category: usize,
}

#[derive(Default)]
struct State {
    revision: Option<u64>,
    name: String,
    items: Vec<Item>,
    /// `(id, name, item count)` for every shelf, in the store's order.
    shelves: Vec<(u64, String, usize)>,
    active: usize,
    filter: usize,
    query: Option<String>,
    selected: Option<u64>,
    scroll: f32,
    /// Entries still to remove, one per frame.
    pending_removals: Vec<u64>,
    copied: Option<(u64, Instant)>,
    picker: Option<Receiver<Vec<PathBuf>>>,
    grid: Option<Rect>,
    /// Local clock offset from UTC in seconds, and when it was measured.
    offset: Option<(i64, Instant)>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> R {
    STATE.with(|cell| f(&mut cell.borrow_mut()))
}

// ---------------------------------------------------------------------------------------
// Data
// ---------------------------------------------------------------------------------------

/// 1 = Images, 2 = Docs, 3 = Other (folders and everything else).
fn category(path: &std::path::Path, is_dir: bool) -> usize {
    if is_dir {
        return 3;
    }
    if kit::is_image(path) || kit::extension(path) == "svg" {
        return 1;
    }
    match kit::kind(path).label.as_str() {
        "PDF" | "DOC" | "XLS" | "PPT" | "TXT" => 2,
        _ => 3,
    }
}

/// Rebuilds the items when the app published a new shelf snapshot.
fn refresh(state: &mut State) {
    let revision = crate::ui::dashboard::SHELF.with(|cell| cell.borrow().revision);
    if revision.is_none() || revision == state.revision {
        return;
    }
    let (name, entries) = crate::ui::dashboard::SHELF.with(|cell| {
        let view = cell.borrow();
        (view.name.clone(), view.entries.clone())
    });
    state.revision = revision;
    let switched = state.name != name;
    state.name = name;
    let snapshot = crate::core::json_store::read::<winisland_core::shelf::ShelfSnapshot>(
        crate::core::shelf_store::FILE_NAME,
        4 * 1024 * 1024,
    )
    .ok()
    .flatten();
    let added_at = |id: u64| {
        snapshot.as_ref().and_then(|snapshot| {
            snapshot
                .shelves
                .iter()
                .flat_map(|shelf| shelf.entries())
                .find(|entry| entry.id == id)
                .map(|entry| entry.added_at)
        })
    };
    state.items = entries
        .into_iter()
        .enumerate()
        .map(|(order, (id, name, path, exists))| {
            let meta = if exists {
                std::fs::metadata(&path).ok()
            } else {
                None
            };
            let is_dir = meta.as_ref().is_some_and(std::fs::Metadata::is_dir);
            Item {
                id,
                name,
                exists,
                is_dir,
                bytes: meta
                    .as_ref()
                    .filter(|meta| meta.is_file())
                    .map_or(0, std::fs::Metadata::len),
                added: added_at(id),
                order,
                category: category(&path, is_dir),
                path,
            }
        })
        .collect();
    // Newest first; files without a known time keep their shelf order.
    state
        .items
        .sort_by(|a, b| b.added.cmp(&a.added).then_with(|| b.order.cmp(&a.order)));
    state.shelves = snapshot
        .as_ref()
        .map(|snapshot| {
            snapshot
                .shelves
                .iter()
                .map(|shelf| (shelf.id, shelf.name.clone(), shelf.entries().len()))
                .collect()
        })
        .unwrap_or_default();
    match state
        .shelves
        .iter()
        .position(|(_, name, _)| *name == state.name)
    {
        Some(index) => state.active = index,
        None => {
            state.shelves = vec![(0, state.name.clone(), 0)];
            state.active = 0;
        }
    }
    let count = state.items.len();
    if let Some(active) = state.shelves.get_mut(state.active) {
        active.2 = count;
    }
    if switched {
        state.selected = None;
        state.scroll = 0.0;
        state.query = None;
    }
    if state
        .selected
        .is_some_and(|id| !state.items.iter().any(|item| item.id == id))
    {
        state.selected = None;
    }
}

#[derive(Clone, Copy)]
enum Line {
    Header(&'static str),
    Item(usize),
}

struct Model {
    /// Indices into `State::items`, in display order.
    shown: Vec<usize>,
    lines: Vec<(Line, f32, f32)>,
    content_height: f32,
    counts: [usize; 4],
    /// Position in `shown` of the selected file (the first one when nothing is picked).
    selected: Option<usize>,
}

fn build_model(state: &mut State) -> Model {
    let mut counts = [state.items.len(), 0, 0, 0];
    for item in &state.items {
        counts[item.category] += 1;
    }
    let query = state
        .query
        .as_deref()
        .map(|query| query.trim().to_lowercase())
        .filter(|query| !query.is_empty());
    let shown: Vec<usize> = state
        .items
        .iter()
        .enumerate()
        .filter(|(_, item)| query.is_some() || state.filter == 0 || item.category == state.filter)
        .filter(|(_, item)| {
            query
                .as_deref()
                .is_none_or(|query| item.name.to_lowercase().contains(query))
        })
        .map(|(index, _)| index)
        .collect();
    let offset = local_offset(state);
    let today = local_day(SystemTime::now(), offset);
    let mut lines = Vec::with_capacity(shown.len() + 3);
    let mut y = 0.0;
    let mut group = None;
    for (position, index) in shown.iter().enumerate() {
        let name = match state.items[*index]
            .added
            .map(|time| local_day(time, offset))
        {
            Some(day) if day >= today => "Today",
            Some(day) if day == today - 1 => "Yesterday",
            _ => "Earlier",
        };
        if group != Some(name) {
            group = Some(name);
            lines.push((Line::Header(name), y, HEADER_HEIGHT));
            y += HEADER_HEIGHT;
        }
        lines.push((Line::Item(position), y, ROW_HEIGHT));
        y += ROW_HEIGHT;
    }
    let selected = state
        .selected
        .and_then(|id| shown.iter().position(|index| state.items[*index].id == id))
        .or_else(|| (!shown.is_empty()).then_some(0));
    Model {
        shown,
        lines,
        content_height: y,
        counts,
        selected,
    }
}

impl Model {
    fn item<'a>(&self, state: &'a State, position: usize) -> Option<&'a Item> {
        self.shown
            .get(position)
            .and_then(|index| state.items.get(*index))
    }
}

// ---------------------------------------------------------------------------------------
// Time
// ---------------------------------------------------------------------------------------

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

fn unix_seconds(time: SystemTime) -> i64 {
    match time.duration_since(UNIX_EPOCH) {
        Ok(elapsed) => elapsed.as_secs() as i64,
        Err(error) => -(error.duration().as_secs() as i64),
    }
}

/// The local clock's offset from UTC in seconds, re-measured once a minute.
fn local_offset(state: &mut State) -> i64 {
    if let Some((offset, at)) = state.offset
        && at.elapsed() < Duration::from_secs(60)
    {
        return offset;
    }
    let local = crate::platform::shell().local_datetime();
    let local_seconds = crate::ui::dashboard::days_from_civil(
        i64::from(local.year),
        u32::from(local.month),
        u32::from(local.day),
    ) * 86_400
        + i64::from(local.hour) * 3600
        + i64::from(local.minute) * 60
        + i64::from(local.second);
    let offset = local_seconds - unix_seconds(SystemTime::now());
    let offset = (offset as f64 / 900.0).round() as i64 * 900;
    state.offset = Some((offset, Instant::now()));
    offset
}

fn local_day(time: SystemTime, offset: i64) -> i64 {
    (unix_seconds(time) + offset).div_euclid(86_400)
}

/// "now", "4m ago", "23h ago", "3d ago", then "Oct 28".
fn ago(time: SystemTime, offset: i64) -> String {
    let seconds = unix_seconds(SystemTime::now()) - unix_seconds(time);
    if seconds < 60 {
        "now".to_string()
    } else if seconds < 3600 {
        format!("{}m ago", seconds / 60)
    } else if seconds < 86_400 {
        format!("{}h ago", seconds / 3600)
    } else if seconds < 604_800 {
        format!("{}d ago", seconds / 86_400)
    } else {
        month_day(time, offset)
    }
}

fn month_day(time: SystemTime, offset: i64) -> String {
    let (_, month, day) = crate::ui::dashboard::civil_from_days(local_day(time, offset));
    format!(
        "{} {day}",
        MONTHS[(month as usize).saturating_sub(1).min(11)]
    )
}

/// "Oct 28, 10:13 PM".
fn stamp(time: SystemTime, offset: i64) -> String {
    let seconds = (unix_seconds(time) + offset).rem_euclid(86_400);
    let (hour, minute) = (seconds / 3600, seconds / 60 % 60);
    let suffix = if hour < 12 { "AM" } else { "PM" };
    let hour12 = match hour % 12 {
        0 => 12,
        other => other,
    };
    format!("{}, {hour12}:{minute:02} {suffix}", month_day(time, offset))
}

fn size_label(item: &Item) -> String {
    if !item.exists {
        "Missing".to_string()
    } else if item.is_dir {
        "Folder".to_string()
    } else {
        format_bytes(item.bytes)
    }
}

// ---------------------------------------------------------------------------------------
// Layout
// ---------------------------------------------------------------------------------------

struct Layout {
    list: Rect,
    divider_x: f32,
    pane: Rect,
    tabs_row: Rect,
    search_button: Rect,
    add_button: Rect,
    view: Rect,
}

fn layout(grid: Rect) -> Layout {
    let list_width = (grid.width() * 0.5).round();
    let list = Rect::from_ltrb(grid.left, grid.top, grid.left + list_width, grid.bottom);
    let tabs_row = Rect::from_xywh(list.left, list.top + 2.0, list.width(), TAB_ROW);
    let add_button = Rect::from_xywh(list.right - BUTTON, tabs_row.top, BUTTON, BUTTON);
    Layout {
        list,
        divider_x: list.right + 12.0,
        pane: Rect::from_ltrb(list.right + 25.0, grid.top + 2.0, grid.right, grid.bottom),
        tabs_row,
        search_button: Rect::from_xywh(
            add_button.left - BUTTON - 2.0,
            tabs_row.top,
            BUTTON,
            BUTTON,
        ),
        add_button,
        view: Rect::from_ltrb(list.left, tabs_row.bottom + 6.0, list.right, list.bottom),
    }
}

fn tab_rects(row: Rect, counts: &[usize; 4]) -> [Rect; 4] {
    let mut left = row.left + 4.0;
    let mut rects = [Rect::from_xywh(0.0, 0.0, 0.0, 0.0); 4];
    for (index, rect) in rects.iter_mut().enumerate() {
        let width = text_width(FILTERS[index], TAB_LABEL)
            + 5.0
            + text_width(&counts[index].to_string(), TAB_COUNT);
        *rect = Rect::from_xywh(left, row.top, width, row.height());
        left += width + 14.0;
    }
    rects
}

/// The search field spans the tab row up to the add button.
fn search_rect(layout: &Layout) -> Rect {
    Rect::from_ltrb(
        layout.tabs_row.left,
        layout.tabs_row.top,
        layout.add_button.left - 6.0,
        layout.tabs_row.bottom,
    )
}

struct PaneLayout {
    shelf: Rect,
    new_shelf: Rect,
    remove: Rect,
    preview: Rect,
    name: Rect,
    path: Rect,
    footer: Rect,
    open: Rect,
    copy: Rect,
    reveal: Rect,
    convert: Rect,
}

fn pane_layout(pane: Rect, shelf_label_w: f32) -> PaneLayout {
    let remove = Rect::from_xywh(pane.right - BUTTON, pane.top, BUTTON, BUTTON);
    let shelf_w = (shelf_label_w + 34.0).min(pane.width() - BUTTON * 2.0 - 12.0);
    let shelf = Rect::from_xywh(pane.left, pane.top, shelf_w.max(40.0), BUTTON);
    let new_shelf = Rect::from_xywh(shelf.right + 2.0, pane.top, BUTTON, BUTTON);
    let footer = Rect::from_ltrb(pane.left, pane.bottom - FOOTER, pane.right, pane.bottom);
    let open = Rect::from_xywh(
        footer.right - OPEN_WIDTH,
        footer.top + 1.0,
        OPEN_WIDTH,
        FOOTER - 2.0,
    );
    let copy = Rect::from_xywh(open.left - 6.0 - BUTTON, footer.top + 2.0, BUTTON, BUTTON);
    let reveal = Rect::from_xywh(copy.left - 2.0 - BUTTON, footer.top + 2.0, BUTTON, BUTTON);
    let convert = Rect::from_xywh(reveal.left - 2.0 - BUTTON, footer.top + 2.0, BUTTON, BUTTON);
    let path = Rect::from_ltrb(pane.left, footer.top - 20.0, pane.right, footer.top - 4.0);
    let name = Rect::from_ltrb(pane.left, path.top - 20.0, pane.right, path.top);
    let preview = Rect::from_ltrb(pane.left, shelf.bottom + 8.0, pane.right, name.top - 6.0);
    PaneLayout {
        shelf,
        new_shelf,
        remove,
        preview,
        name,
        path,
        footer,
        open,
        copy,
        reveal,
        convert,
    }
}

fn shelf_label(state: &State) -> String {
    if state.name.is_empty() {
        "Shelf".to_string()
    } else {
        state.name.clone()
    }
}

/// The empty page: a drop-zone card with an "Add files" button.
fn empty_card(grid: Rect) -> Rect {
    grid.inset(4.0)
}

fn empty_add(card: Rect) -> Rect {
    let width = text_width("Add files", PILL) + 46.0;
    Rect::from_xywh(
        card.center_x() - width / 2.0,
        card.center_y() + 36.0,
        width,
        30.0,
    )
}

fn max_scroll(model: &Model, view: Rect) -> f32 {
    (model.content_height - view.height()).max(0.0)
}

/// Scrolls so the selected row (and its group header) is visible.
fn reveal(state: &mut State) {
    let Some(grid) = state.grid else {
        return;
    };
    let model = build_model(state);
    let view = layout(grid).view;
    let Some(selected) = model.selected else {
        state.scroll = 0.0;
        return;
    };
    let Some((_, top, height)) = model
        .lines
        .iter()
        .find(|(line, _, _)| matches!(line, Line::Item(position) if *position == selected))
        .copied()
    else {
        return;
    };
    let context = ROW_HEIGHT * 0.6;
    let mut scroll = state.scroll;
    if top - context < scroll {
        scroll = top - context;
    }
    if top + height + context > scroll + view.height() {
        scroll = top + height + context - view.height();
    }
    state.scroll = scroll.clamp(0.0, max_scroll(&model, view));
}

fn line_at(model: &Model, scroll: f32, view: Rect, point: Point) -> Option<Line> {
    if !view.contains(point) {
        return None;
    }
    let y = point.y - view.top + scroll;
    model
        .lines
        .iter()
        .find(|(_, top, height)| y >= *top && y < top + height)
        .map(|(line, _, _)| *line)
}

// ---------------------------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------------------------

/// Draws the tab. Return `false` to fall back to the legacy page.
pub fn draw(ctx: &TabContext<'_>) -> bool {
    let grid = ctx.grid;
    let hovered = |id: u16| ctx.hover == Some(DashboardHit::Tab(id));
    with_state(|state| {
        state.grid = Some(grid);
        refresh(state);
        if state.items.is_empty() {
            draw_empty(ctx.painter, grid, state, &hovered, ctx.colors);
            return;
        }
        let model = build_model(state);
        let layout = layout(grid);
        state.scroll = state.scroll.clamp(0.0, max_scroll(&model, layout.view));
        draw_list_header(ctx.painter, &layout, state, &model, &hovered, ctx.colors);
        draw_list(ctx.painter, &layout, state, &model, &hovered, ctx.colors);
        ctx.painter.fill_rect(
            Rect::from_ltrb(
                layout.divider_x,
                grid.top + 4.0,
                layout.divider_x + 1.0,
                grid.bottom - 4.0,
            ),
            ctx.colors.border_subtle,
        );
        draw_pane(
            ctx.painter,
            &layout,
            state,
            &model,
            &hovered,
            ctx.now,
            ctx.colors,
        );
    });
    true
}

fn draw_empty(
    painter: Painter<'_>,
    grid: Rect,
    state: &State,
    hovered: &dyn Fn(u16) -> bool,
    colors: &SemanticColors,
) {
    let card = empty_card(grid);
    painter.fill_round_rect(
        card,
        Radius::uniform(16.0),
        colors.surface_raised.with_alpha(110),
    );
    kit::dashed_round_rect(painter, card, 16.0, colors.border_strong);
    let center = card.center_y();
    let tile = kit::square(Point::new(card.center_x(), center - 40.0), 44.0);
    painter.fill_round_rect(tile, Radius::uniform(12.0), colors.surface_overlay);
    kit::glyph(
        painter,
        kit::TRAY,
        kit::square(Point::new(tile.center_x(), tile.center_y()), 22.0),
        colors.text_secondary,
    );
    let title = if state.shelves.len() > 1 {
        format!("{} is empty", shelf_label(state))
    } else {
        "Your shelf is empty".to_string()
    };
    text_center(
        painter,
        &title,
        typography::SECTION_TITLE,
        Rect::from_xywh(card.left, center - 8.0, card.width(), 22.0),
        colors.text_primary,
    );
    text_center(
        painter,
        "Drag files onto the notch to park them here · originals never move",
        typography::LABEL,
        Rect::from_xywh(card.left + 16.0, center + 14.0, card.width() - 32.0, 18.0),
        colors.text_secondary,
    );
    let add = empty_add(card);
    accent_pill(
        painter,
        add,
        kit::PLUS,
        "Add files",
        hovered(ID_ADD),
        true,
        colors,
    );
    // With several shelves, keep the switcher reachable from the empty page.
    if state.shelves.len() > 1 {
        let label = shelf_label(state);
        let width = text_width(&label, typography::LABEL) + 34.0;
        let chip = Rect::from_xywh(card.left + 10.0, card.top + 10.0, width, 26.0);
        shelf_chip(painter, chip, &label, hovered(ID_SHELF), colors);
    }
}

fn shelf_chip(painter: Painter<'_>, rect: Rect, label: &str, hover: bool, colors: &SemanticColors) {
    painter.fill_round_rect(
        rect,
        Radius::uniform(rect.height() / 2.0),
        if hover {
            colors.surface_interactive
        } else {
            colors.surface_overlay
        },
    );
    text(
        painter,
        label,
        typography::LABEL,
        Rect::from_ltrb(rect.left + 12.0, rect.top, rect.right - 22.0, rect.bottom),
        colors.text_primary,
    );
    kit::glyph(
        painter,
        kit::CHEVRON_DOWN,
        kit::square(Point::new(rect.right - 13.0, rect.center_y() + 1.0), 13.0),
        colors.text_secondary,
    );
}

/// A filled accent pill with a glyph and a label.
fn accent_pill(
    painter: Painter<'_>,
    rect: Rect,
    svg: &'static str,
    label: &str,
    hover: bool,
    enabled: bool,
    colors: &SemanticColors,
) {
    let (fill, ink) = if !enabled {
        (colors.surface_interactive, colors.text_disabled)
    } else if hover {
        (colors.accent_primary.with_alpha(220), colors.surface_base)
    } else {
        (colors.accent_primary, colors.surface_base)
    };
    painter.fill_round_rect(rect, Radius::uniform(rect.height() / 2.0), fill);
    let content = 14.0 + 6.0 + text_width(label, PILL);
    let left = rect.center_x() - content / 2.0;
    kit::glyph(
        painter,
        svg,
        Rect::from_xywh(left, rect.center_y() - 7.0, 14.0, 14.0),
        ink,
    );
    text(
        painter,
        label,
        PILL,
        Rect::from_ltrb(left + 20.0, rect.top, rect.right, rect.bottom),
        ink,
    );
}

fn draw_list_header(
    painter: Painter<'_>,
    layout: &Layout,
    state: &State,
    model: &Model,
    hovered: &dyn Fn(u16) -> bool,
    colors: &SemanticColors,
) {
    kit::icon_button(
        painter,
        layout.add_button,
        kit::PLUS,
        hovered(ID_ADD),
        state.picker.is_none(),
        colors,
    );
    if let Some(query) = &state.query {
        let field = search_rect(layout);
        kit::search_field(painter, field, query, "Search shelf", colors);
        let close = kit::search_close(field);
        kit::glyph(
            painter,
            kit::CLOSE,
            kit::square(Point::new(close.center_x(), close.center_y()), 14.0),
            if hovered(ID_SEARCH_CLOSE) {
                colors.text_primary
            } else {
                colors.text_secondary
            },
        );
        return;
    }
    for (index, rect) in tab_rects(layout.tabs_row, &model.counts).iter().enumerate() {
        if rect.right > layout.search_button.left - 4.0 {
            break;
        }
        let active = state.filter == index;
        let hover = hovered(ID_FILTER + index as u16);
        let label_color = if active || hover {
            colors.text_primary
        } else {
            colors.text_secondary
        };
        let label_width = text_width(FILTERS[index], TAB_LABEL);
        let text_rect = Rect::from_ltrb(rect.left, rect.top, rect.right, rect.bottom - 4.0);
        text(painter, FILTERS[index], TAB_LABEL, text_rect, label_color);
        text(
            painter,
            &model.counts[index].to_string(),
            TAB_COUNT,
            Rect::from_ltrb(
                rect.left + label_width + 5.0,
                text_rect.top + 1.0,
                rect.right + 2.0,
                text_rect.bottom + 1.0,
            ),
            colors.text_secondary,
        );
        if active {
            painter.fill_round_rect(
                Rect::from_xywh(rect.left, rect.bottom - 2.0, rect.width(), 2.0),
                Radius::uniform(1.0),
                colors.text_primary,
            );
        }
    }
    kit::icon_button(
        painter,
        layout.search_button,
        kit::SEARCH,
        hovered(ID_SEARCH),
        true,
        colors,
    );
}

fn draw_list(
    painter: Painter<'_>,
    layout: &Layout,
    state: &mut State,
    model: &Model,
    hovered: &dyn Fn(u16) -> bool,
    colors: &SemanticColors,
) {
    let view = layout.view;
    if model.shown.is_empty() {
        let message = if state
            .query
            .as_deref()
            .is_some_and(|query| !query.trim().is_empty())
        {
            "No matching files"
        } else {
            match state.filter {
                1 => "No images on this shelf",
                2 => "No documents on this shelf",
                _ => "Nothing else on this shelf",
            }
        };
        text(
            painter,
            message,
            typography::BODY,
            Rect::from_xywh(view.left + 8.0, view.top + 8.0, view.width() - 16.0, 22.0),
            colors.text_secondary,
        );
        return;
    }
    let offset = local_offset(state);
    let scroll = state.scroll;
    let save = painter.save();
    painter.clip_rect(view);
    for (line, top, height) in &model.lines {
        let y = view.top + top - scroll;
        if y + height < view.top || y > view.bottom {
            continue;
        }
        match *line {
            Line::Header(name) => text(
                painter,
                name,
                typography::LABEL,
                Rect::from_xywh(view.left + 8.0, y + 2.0, view.width() - 16.0, height - 2.0),
                colors.text_secondary,
            ),
            Line::Item(position) => {
                let Some(item) = model.item(state, position) else {
                    continue;
                };
                let rect = Rect::from_xywh(view.left, y + 1.0, view.width() - 6.0, height - 2.0);
                let selected = model.selected == Some(position);
                let hover = position < MAX_ROW_IDS && hovered(ID_ROW + position as u16);
                if selected {
                    painter.fill_round_rect(
                        rect,
                        Radius::uniform(10.0),
                        colors.surface_interactive,
                    );
                } else if hover {
                    painter.fill_round_rect(rect, Radius::uniform(10.0), colors.surface_raised);
                }
                let tile = Rect::from_xywh(rect.left + 8.0, rect.center_y() - 15.0, 30.0, 30.0);
                draw_tile(painter, tile, item, colors);
                let text_left = tile.right + 10.0;
                text(
                    painter,
                    &item.name,
                    typography::BODY,
                    Rect::from_ltrb(
                        text_left,
                        rect.top + 3.0,
                        rect.right - 10.0,
                        rect.top + 22.0,
                    ),
                    if item.exists {
                        colors.text_primary
                    } else {
                        colors.text_disabled
                    },
                );
                let mut subtitle = size_label(item);
                if let Some(time) = item.added {
                    subtitle.push_str(" · ");
                    subtitle.push_str(&ago(time, offset));
                }
                text(
                    painter,
                    &subtitle,
                    typography::CAPTION,
                    Rect::from_ltrb(
                        text_left,
                        rect.top + 22.0,
                        rect.right - 10.0,
                        rect.bottom - 3.0,
                    ),
                    if item.exists {
                        colors.text_secondary
                    } else {
                        colors.state_error
                    },
                );
            }
        }
    }
    painter.restore_to(save);
    let max = max_scroll(model, view);
    if max > 0.0 {
        let track = view.height() - 8.0;
        let thumb = (track * view.height() / model.content_height).max(18.0);
        let top = view.top + 4.0 + (track - thumb) * (scroll / max);
        painter.fill_round_rect(
            Rect::from_xywh(view.right - 3.0, top, 3.0, thumb),
            Radius::uniform(1.5),
            colors.border_strong,
        );
    }
}

/// A row's 30 px tile: the image thumbnail, a folder, or a coloured type badge.
fn draw_tile(painter: Painter<'_>, tile: Rect, item: &Item, colors: &SemanticColors) {
    if item.is_dir
        || (item.exists && kit::is_image(&item.path) && thumbs::thumbnail(&item.path).is_some())
    {
        kit::file_icon(painter, tile, &item.path, item.is_dir, !item.exists, colors);
    } else {
        kit::badge(painter, tile, &kit::kind(&item.path), !item.exists);
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_pane(
    painter: Painter<'_>,
    layout: &Layout,
    state: &mut State,
    model: &Model,
    hovered: &dyn Fn(u16) -> bool,
    now: Instant,
    colors: &SemanticColors,
) {
    let label = shelf_label(state);
    let parts = pane_layout(layout.pane, text_width(&label, typography::LABEL));
    shelf_chip(painter, parts.shelf, &label, hovered(ID_SHELF), colors);
    kit::icon_button(
        painter,
        parts.new_shelf,
        kit::FOLDER_PLUS,
        hovered(ID_NEW_SHELF),
        true,
        colors,
    );
    let offset = local_offset(state);
    let Some(item) = model
        .selected
        .and_then(|position| model.item(state, position))
        .cloned()
    else {
        text_center(
            painter,
            "Pick a file to see it here",
            typography::BODY,
            Rect::from_xywh(
                layout.pane.left,
                parts.preview.center_y() - 10.0,
                layout.pane.width(),
                20.0,
            ),
            colors.text_secondary,
        );
        return;
    };
    // Remove from shelf (never deletes the original).
    let remove_hover = hovered(ID_REMOVE);
    if remove_hover {
        painter.fill_round_rect(
            parts.remove,
            Radius::uniform(parts.remove.height() / 2.0),
            colors.surface_overlay,
        );
    }
    kit::glyph(
        painter,
        kit::TRASH,
        kit::square(
            Point::new(parts.remove.center_x(), parts.remove.center_y()),
            16.0,
        ),
        if remove_hover {
            colors.state_error
        } else {
            colors.text_secondary
        },
    );

    draw_preview(painter, parts.preview, &item, colors);

    text(
        painter,
        &item.name,
        style(14.0, 600),
        parts.name,
        if item.exists {
            colors.text_primary
        } else {
            colors.text_disabled
        },
    );
    let folder = item
        .path
        .parent()
        .map(|parent| parent.display().to_string())
        .unwrap_or_default();
    text(
        painter,
        &if item.exists {
            folder
        } else {
            format!("Missing · was in {folder}")
        },
        typography::CAPTION,
        parts.path,
        if item.exists {
            colors.text_secondary
        } else {
            colors.state_error
        },
    );

    // Footer: size and date, the icon actions and the Open pill.
    let image = item.exists && kit::is_image(&item.path);
    let actions_left = if image {
        parts.convert.left
    } else {
        parts.reveal.left
    };
    let mut meta = size_label(&item);
    if image && let Some((width, height)) = thumbs::dimensions(&item.path) {
        meta = format!("{width} × {height} · {meta}");
    }
    if let Some(time) = item.added {
        meta.push_str(" · ");
        meta.push_str(&stamp(time, offset));
    }
    text(
        painter,
        &meta,
        typography::LABEL,
        Rect::from_ltrb(
            parts.footer.left,
            parts.footer.top,
            actions_left - 8.0,
            parts.footer.bottom,
        ),
        colors.text_secondary,
    );
    let copied = state
        .copied
        .is_some_and(|(id, at)| id == item.id && now.saturating_duration_since(at) < FEEDBACK);
    kit::icon_button(
        painter,
        parts.copy,
        if copied { kit::CHECK } else { kit::COPY },
        hovered(ID_COPY),
        item.exists,
        colors,
    );
    kit::icon_button(
        painter,
        parts.reveal,
        kit::REVEAL,
        hovered(ID_REVEAL),
        item.exists,
        colors,
    );
    if image {
        kit::icon_button(
            painter,
            parts.convert,
            kit::CONVERT,
            hovered(ID_CONVERT),
            true,
            colors,
        );
    }
    accent_pill(
        painter,
        parts.open,
        kit::OPEN,
        "Open",
        hovered(ID_OPEN),
        item.exists,
        colors,
    );
}

/// The large preview: the image fitted inside a soft frame, or a big file tile.
fn draw_preview(painter: Painter<'_>, area: Rect, item: &Item, colors: &SemanticColors) {
    if area.height() < 24.0 {
        return;
    }
    painter.fill_round_rect(area, Radius::uniform(12.0), colors.surface_raised);
    let thumbnail = (item.exists && kit::is_image(&item.path))
        .then(|| thumbs::thumbnail(&item.path))
        .flatten();
    if let Some(image) = thumbnail {
        let inner = area.inset(8.0);
        let (iw, ih) = (image.width().max(1) as f32, image.height().max(1) as f32);
        let scale = (inner.width() / iw).min(inner.height() / ih);
        let (w, h) = (iw * scale, ih * scale);
        let frame = Rect::from_xywh(inner.center_x() - w / 2.0, inner.center_y() - h / 2.0, w, h);
        let save = painter.save();
        painter.clip_round_rect(frame, Radius::uniform(8.0));
        painter.draw_image(
            &image,
            frame,
            &ImageOptions {
                fit: ImageFit::Contain,
                sampling: Sampling::LinearNone,
                ..ImageOptions::default()
            },
        );
        painter.restore_to(save);
        return;
    }
    let size = (area.height() - 16.0).min(96.0);
    let icon = kit::square(Point::new(area.center_x(), area.center_y()), size);
    kit::file_icon(painter, icon, &item.path, item.is_dir, !item.exists, colors);
}

/// `true` when this tab draws its own cards instead of sitting on the shared page card.
pub fn custom_layout() -> bool {
    true
}

// ---------------------------------------------------------------------------------------
// Input
// ---------------------------------------------------------------------------------------

/// Hit test in page coordinates. Use `DashboardHit::Tab(id)` with ids private to this tab.
pub fn hit(grid: Rect, point: Point) -> Option<DashboardHit> {
    if !grid.contains(point) {
        return None;
    }
    let tab = |id: u16| Some(DashboardHit::Tab(id));
    with_state(|state| {
        state.grid = Some(grid);
        refresh(state);
        if state.items.is_empty() {
            let card = empty_card(grid);
            if empty_add(card).contains(point) {
                return tab(ID_ADD);
            }
            if state.shelves.len() > 1 {
                let width = text_width(&shelf_label(state), typography::LABEL) + 34.0;
                if Rect::from_xywh(card.left + 10.0, card.top + 10.0, width, 26.0).contains(point) {
                    return tab(ID_SHELF);
                }
            }
            return tab(ID_INERT);
        }
        let model = build_model(state);
        let layout = layout(grid);
        if layout.add_button.contains(point) {
            return tab(ID_ADD);
        }
        if state.query.is_some() {
            if kit::search_close(search_rect(&layout)).contains(point) {
                return tab(ID_SEARCH_CLOSE);
            }
        } else {
            if layout.search_button.contains(point) {
                return tab(ID_SEARCH);
            }
            if let Some(index) = tab_rects(layout.tabs_row, &model.counts)
                .iter()
                .position(|rect| {
                    rect.right <= layout.search_button.left - 4.0 && rect.contains(point)
                })
            {
                return tab(ID_FILTER + index as u16);
            }
        }
        if let Some(Line::Item(position)) = line_at(&model, state.scroll, layout.view, point)
            && position < MAX_ROW_IDS
        {
            return tab(ID_ROW + position as u16);
        }
        let parts = pane_layout(
            layout.pane,
            text_width(&shelf_label(state), typography::LABEL),
        );
        if parts.shelf.contains(point) {
            return tab(ID_SHELF);
        }
        if parts.new_shelf.contains(point) {
            return tab(ID_NEW_SHELF);
        }
        if let Some(item) = model
            .selected
            .and_then(|position| model.item(state, position))
        {
            let image = item.exists && kit::is_image(&item.path);
            if parts.remove.contains(point) {
                return tab(ID_REMOVE);
            }
            if item.exists {
                for (rect, id) in [
                    (parts.open, ID_OPEN),
                    (parts.copy, ID_COPY),
                    (parts.reveal, ID_REVEAL),
                ] {
                    if rect.contains(point) {
                        return tab(id);
                    }
                }
            }
            if image && parts.convert.contains(point) {
                return tab(ID_CONVERT);
            }
        }
        tab(ID_INERT)
    })
}

/// Scrolls the file list by wheel `lines` (positive = towards the top, like `WM_MOUSEWHEEL`).
/// `true` when the list moved or the point was over it.
pub fn scroll(grid: Rect, point: Point, lines: f32) -> bool {
    with_state(|state| {
        state.grid = Some(grid);
        if state.items.is_empty() {
            return false;
        }
        let layout = layout(grid);
        if !layout.view.contains(point) {
            return false;
        }
        let model = build_model(state);
        let max = max_scroll(&model, layout.view);
        state.scroll = (state.scroll - lines * ROW_HEIGHT).clamp(0.0, max);
        true
    })
}

/// Applies a click from [`hit`]. `true` when handled.
/// The file under a row hit, if it can be dragged out of the shelf (it still exists).
pub fn drag_paths(hit: DashboardHit) -> Option<Vec<PathBuf>> {
    let DashboardHit::Tab(id) = hit else {
        return None;
    };
    if id < ID_ROW || usize::from(id - ID_ROW) >= MAX_ROW_IDS {
        return None;
    }
    with_state(|state| {
        let model = build_model(state);
        model
            .item(state, usize::from(id - ID_ROW))
            .filter(|item| item.exists)
            .map(|item| vec![item.path.clone()])
    })
}

pub fn apply(hit: DashboardHit) -> bool {
    let id = match hit {
        DashboardHit::Tab(id) if (600..700).contains(&id) => id,
        _ => return false,
    };
    let mut convert = false;
    with_state(|state| {
        let model = build_model(state);
        let selected = model
            .selected
            .and_then(|position| model.item(state, position))
            .cloned();
        match id {
            ID_ADD => open_picker(state),
            ID_SEARCH => {
                state.query = Some(String::new());
                state.scroll = 0.0;
            }
            ID_SEARCH_CLOSE => {
                state.query = None;
                reveal(state);
            }
            ID_SHELF => {
                if state.shelves.len() > 1 {
                    let next = (state.active + 1) % state.shelves.len();
                    crate::core::shelf_store::request_activate(state.shelves[next].0);
                }
            }
            ID_NEW_SHELF => crate::core::shelf_store::request_new_shelf(),
            ID_REMOVE => {
                if let Some(item) = selected {
                    let position = model.selected.unwrap_or(0);
                    let next = model
                        .item(state, position + 1)
                        .or_else(|| position.checked_sub(1).and_then(|p| model.item(state, p)))
                        .map(|item| item.id);
                    state.pending_removals.push(item.id);
                    state.selected = next;
                    reveal(state);
                }
            }
            ID_OPEN => {
                if let Some(item) = selected {
                    open_item(&item);
                }
            }
            ID_REVEAL => {
                if let Some(item) = selected {
                    reveal_in_folder(&item);
                }
            }
            ID_COPY => {
                if let Some(item) = selected.filter(|item| item.exists) {
                    match clip::set_files(std::slice::from_ref(&item.path)) {
                        Ok(()) => state.copied = Some((item.id, Instant::now())),
                        Err(error) => log::warn!("Cannot copy shelf file: {error}"),
                    }
                }
            }
            ID_CONVERT => convert = true,
            _ if (ID_FILTER..ID_FILTER + 4).contains(&id) => {
                state.filter = usize::from(id - ID_FILTER);
                state.scroll = 0.0;
                state.selected = None;
            }
            _ if id >= ID_ROW && usize::from(id - ID_ROW) < MAX_ROW_IDS => {
                let position = usize::from(id - ID_ROW);
                if let Some(item) = model.item(state, position).cloned() {
                    if model.selected == Some(position) && state.selected == Some(item.id) {
                        // A second click on the selected row opens it (or drops a missing one).
                        if item.exists {
                            open_item(&item);
                        }
                    } else {
                        state.selected = Some(item.id);
                    }
                    reveal(state);
                }
            }
            _ => {}
        }
    });
    if convert {
        super::converter::import_shelf();
        crate::ui::dashboard::show(Destination::Converter);
    }
    true
}

fn open_item(item: &Item) {
    if let Err(error) = crate::platform::shell().open_url(&item.path.to_string_lossy()) {
        log::warn!("Cannot open shelf file: {error}");
    }
}

fn reveal_in_folder(item: &Item) {
    use std::os::windows::process::CommandExt;
    let result = std::process::Command::new("explorer.exe")
        .raw_arg(format!("/select,\"{}\"", item.path.display()))
        .spawn();
    if let Err(error) = result {
        log::warn!("Cannot show shelf file in folder: {error}");
    }
}

/// Opens the system file picker on its own thread; `poll` parks the chosen files.
fn open_picker(state: &mut State) {
    if state.picker.is_some() {
        return;
    }
    let (sender, receiver) = channel();
    let spawned = std::thread::Builder::new()
        .name("shelf-picker".into())
        .spawn(move || {
            let files = rfd::FileDialog::new()
                .set_title("Add files to the shelf")
                .pick_files()
                .unwrap_or_default();
            let _ = sender.send(files);
        });
    if spawned.is_ok() {
        state.picker = Some(receiver);
    }
}

fn select_offset(state: &mut State, delta: isize) {
    let model = build_model(state);
    if model.shown.is_empty() {
        return;
    }
    let current = model.selected.unwrap_or(0) as isize;
    let next = (current + delta).clamp(0, model.shown.len() as isize - 1) as usize;
    state.selected = model.item(state, next).map(|item| item.id);
    reveal(state);
}

pub fn wants_keyboard() -> bool {
    with_state(|state| state.query.is_some())
}

pub fn handle_key(key: &Key) -> bool {
    with_state(|state| {
        let Some(query) = state.query.as_mut() else {
            return false;
        };
        match key {
            Key::Escape => {
                state.query = None;
                reveal(state);
            }
            Key::Backspace => {
                query.pop();
                state.scroll = 0.0;
                state.selected = None;
            }
            Key::Character(typed) => {
                if query.chars().count() < 80 {
                    query.extend(typed.chars().filter(|character| !character.is_control()));
                }
                state.scroll = 0.0;
                state.selected = None;
            }
            Key::ArrowDown => select_offset(state, 1),
            Key::ArrowUp => select_offset(state, -1),
            Key::Enter => {
                let model = build_model(state);
                if let Some(item) = model
                    .selected
                    .and_then(|position| model.item(state, position))
                {
                    open_item(item);
                }
            }
            _ => {}
        }
        true
    })
}

/// Every frame, visible or not.
pub fn poll(_now: Instant) {
    thumbs::pump();
    with_state(|state| {
        refresh(state);
        if let Some(receiver) = &state.picker {
            match receiver.try_recv() {
                Ok(files) => {
                    state.picker = None;
                    if !files.is_empty() {
                        state.filter = 0;
                        state.query = None;
                        state.selected = None;
                        state.scroll = 0.0;
                        crate::core::shelf_store::request_add(files);
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => state.picker = None,
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }
        if !state.pending_removals.is_empty() {
            crate::ui::dashboard::SHELF_REMOVAL.with(|cell| {
                if cell.get().is_none() {
                    cell.set(state.pending_removals.pop());
                }
            });
        }
    });
}
