//! Clipboard tab: filterable, searchable history on the left, a preview with actions on the
//! right.
//!
//! Items come from `core::clipboard_history::published()`, which carries ids, capture times
//! and pins. Pin, delete and copy are queued back to the history service by id, which applies
//! and saves them on its next poll. Rows are rebuilt only when the published revision moves.
//! Images are decoded off the UI thread by `images` and drawn from its cache.

mod images;
pub mod kit;

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant, SystemTime};

use winisland_core::clipboard::{ClipboardBody, ClipboardItem};
use winisland_platform::Key;
use winisland_render::tokens::{SemanticColors, TextStyle, typography};
use winisland_render::{ImageFit, ImageOptions, Painter, Point, Radius, Rect, Rgba, Sampling};

use super::TabContext;
use crate::core::clipboard_history::{self as history, ClipboardRequest};
use crate::ui::dashboard::DashboardHit;
use kit::glyphs;

const ID_FILTER: u16 = 400;
const ID_SEARCH: u16 = 404;
const ID_SEARCH_CLOSE: u16 = 405;
const ID_PIN: u16 = 406;
const ID_DELETE: u16 = 407;
const ID_COPY: u16 = 408;
const ID_SAVE: u16 = 409;
const ID_ROW: u16 = 410;
const MAX_ROW_IDS: usize = 80;
/// Claims the rest of the page so the legacy row hit test never fires under this tab.
const ID_INERT: u16 = 499;

const TAB_ROW: f32 = 28.0;
const ROW_HEIGHT: f32 = 46.0;
const HEADER_HEIGHT: f32 = 24.0;
const BUTTON: f32 = 28.0;
const COPY_WIDTH: f32 = 86.0;
const FOOTER: f32 = 32.0;
/// Below this grid width the preview pane is dropped and a click copies directly.
const SPLIT_MIN_WIDTH: f32 = 460.0;
const COPIED_FOR: Duration = Duration::from_millis(1600);
/// Preview text is cut here; the wrapped body never shows more than a screenful.
const MAX_PREVIEW_CHARS: usize = 4000;

const FILTERS: [&str; 4] = ["All", "Text", "Images", "Links"];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Text,
    Image,
    Link,
    Files,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Self::Text => "Text",
            Self::Image => "Image",
            Self::Link => "Link",
            Self::Files => "Files",
        }
    }

    fn glyph(self) -> &'static str {
        match self {
            Self::Text => glyphs::TEXT_DOC,
            Self::Image => glyphs::IMAGE,
            Self::Link => glyphs::LINK,
            Self::Files => glyphs::FOLDER,
        }
    }
}

#[derive(Default)]
struct State {
    filter: usize,
    /// `Some` while the search field is open.
    query: Option<String>,
    /// Id of the selected item, so selection survives new captures.
    selected: Option<u64>,
    scroll: f32,
    /// Rows built from the last published history.
    entries: Rc<Vec<Entry>>,
    synced: bool,
    enabled: bool,
    revision: Option<u64>,
    /// Pin and delete requests not yet reflected by a publish, so the view reacts at once.
    pending_pins: HashMap<u64, bool>,
    pending_deletes: Vec<u64>,
    copied: Option<(u64, Instant)>,
    /// Last drawn grid, so clicks and keys can keep the selection in view.
    grid: Option<Rect>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

#[derive(Clone)]
struct Entry {
    id: u64,
    title: String,
    source: String,
    /// Full text (cut to [`MAX_PREVIEW_CHARS`]) for text, links and file lists.
    preview: String,
    kind: Kind,
    /// Pixel size and blob for images.
    size: Option<(u32, u32)>,
    blob_id: Option<String>,
    captured: SystemTime,
    pinned: bool,
}

#[derive(Clone)]
enum Line {
    Header(String),
    /// Position in `Model::shown`.
    Item(usize),
}

struct Model {
    synced: bool,
    enabled: bool,
    total: usize,
    counts: [usize; 4],
    shown: Vec<Entry>,
    /// Lines with their top offset and height inside the list.
    lines: Vec<(Line, f32, f32)>,
    content_height: f32,
    selected: Option<usize>,
    filter: usize,
    query: Option<String>,
    scroll: f32,
    copied: Option<u64>,
}

fn entry_of(item: &ClipboardItem) -> Entry {
    let blob_id = match &item.body {
        ClipboardBody::Image { blob_id, .. } => Some(blob_id.clone()),
        _ => None,
    };
    let (kind, title, preview, size) = match &item.body {
        ClipboardBody::Text(text) => (
            Kind::Text,
            text.lines()
                .map(str::trim)
                .find(|line| !line.is_empty())
                .unwrap_or_default()
                .chars()
                .take(160)
                .collect(),
            text.chars().take(MAX_PREVIEW_CHARS).collect(),
            None,
        ),
        ClipboardBody::Link(url) => (
            Kind::Link,
            url.chars().take(160).collect(),
            url.chars().take(MAX_PREVIEW_CHARS).collect(),
            None,
        ),
        ClipboardBody::Image { width, height, .. } => (
            Kind::Image,
            "Image".to_string(),
            String::new(),
            Some((*width, *height)),
        ),
        ClipboardBody::Files(files) => (
            Kind::Files,
            match files.as_slice() {
                [file] => file
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| file.display().to_string()),
                _ => kit::plural(files.len(), "file", "files"),
            },
            files
                .iter()
                .map(|file| file.display().to_string())
                .collect::<Vec<_>>()
                .join("\n")
                .chars()
                .take(MAX_PREVIEW_CHARS)
                .collect(),
            None,
        ),
    };
    Entry {
        id: item.id,
        title,
        source: item.source_app.clone().unwrap_or_default(),
        preview,
        kind,
        size,
        blob_id,
        captured: item.captured_at,
        pinned: item.pinned,
    }
}

/// Rebuilds rows when the history service published a new revision.
fn sync(state: &mut State) {
    let Some(published) = history::published() else {
        return;
    };
    state.synced = true;
    state.enabled = published.enabled;
    if state.revision == Some(published.revision) {
        return;
    }
    state.revision = Some(published.revision);
    state.entries = Rc::new(published.items.iter().map(entry_of).collect());
    // The publish now carries every request made before it.
    state.pending_pins.clear();
    state.pending_deletes.clear();
}

fn filter_matches(filter: usize, kind: Kind) -> bool {
    match filter {
        1 => kind == Kind::Text,
        2 => kind == Kind::Image,
        3 => kind == Kind::Link,
        _ => true,
    }
}

/// Group of an entry: pinned first, then one group per local day.
fn group_of(entry: &Entry, today: i64) -> (u8, i64) {
    if entry.pinned {
        (0, 0)
    } else {
        (1, today - kit::local_day(entry.captured))
    }
}

fn group_name(group: (u8, i64), entry: &Entry) -> String {
    match group {
        (0, _) => "Pinned".to_string(),
        (_, days) if days <= 0 => "Today".to_string(),
        (_, 1) => "Yesterday".to_string(),
        _ => kit::month_day(entry.captured),
    }
}

fn build_model(state: &State) -> Model {
    let all: Vec<Entry> = state
        .entries
        .iter()
        .filter(|entry| !state.pending_deletes.contains(&entry.id))
        .map(|entry| {
            let mut entry = entry.clone();
            if let Some(&pinned) = state.pending_pins.get(&entry.id) {
                entry.pinned = pinned;
            }
            entry
        })
        .collect();
    let mut counts = [all.len(), 0, 0, 0];
    for entry in &all {
        for (filter, count) in counts.iter_mut().enumerate().skip(1) {
            if filter_matches(filter, entry.kind) {
                *count += 1;
            }
        }
    }
    let needle = state
        .query
        .as_deref()
        .map(|query| query.trim().to_lowercase())
        .filter(|query| !query.is_empty());
    let total = all.len();
    let today = kit::today();
    let mut shown: Vec<(usize, Entry)> = all
        .into_iter()
        .enumerate()
        .filter(|(_, entry)| filter_matches(state.filter, entry.kind))
        .filter(|(_, entry)| {
            needle.as_ref().is_none_or(|needle| {
                entry.title.to_lowercase().contains(needle)
                    || entry.preview.to_lowercase().contains(needle)
                    || entry.source.to_lowercase().contains(needle)
            })
        })
        .collect();
    // History order (newest first) inside each group.
    shown.sort_by_key(|(index, entry)| (group_of(entry, today), *index));
    let shown: Vec<Entry> = shown.into_iter().map(|(_, entry)| entry).collect();
    let mut lines = Vec::new();
    let mut top = 0.0;
    let mut current = None;
    for (position, entry) in shown.iter().enumerate() {
        let group = group_of(entry, today);
        if current != Some(group) {
            current = Some(group);
            lines.push((Line::Header(group_name(group, entry)), top, HEADER_HEIGHT));
            top += HEADER_HEIGHT;
        }
        lines.push((Line::Item(position), top, ROW_HEIGHT));
        top += ROW_HEIGHT;
    }
    let selected = state
        .selected
        .and_then(|id| shown.iter().position(|entry| entry.id == id))
        .or_else(|| (!shown.is_empty()).then_some(0));
    let copied = state
        .copied
        .filter(|(_, at)| at.elapsed() < COPIED_FOR)
        .map(|(id, _)| id);
    Model {
        synced: state.synced,
        enabled: state.enabled,
        total,
        counts,
        shown,
        lines,
        content_height: top,
        selected,
        filter: state.filter,
        query: state.query.clone(),
        scroll: state.scroll,
        copied,
    }
}

struct Layout {
    list: Rect,
    divider_x: f32,
    pane: Option<Rect>,
    tabs_row: Rect,
    search_button: Rect,
    view: Rect,
}

fn layout(grid: Rect) -> Layout {
    let split = grid.width() >= SPLIT_MIN_WIDTH;
    let list_width = if split {
        (grid.width() * 0.47).round()
    } else {
        grid.width()
    };
    let list = Rect::from_ltrb(grid.left, grid.top, grid.left + list_width, grid.bottom);
    let pane =
        split.then(|| Rect::from_ltrb(list.right + 25.0, grid.top + 2.0, grid.right, grid.bottom));
    let tabs_row = Rect::from_xywh(list.left, list.top + 2.0, list.width(), TAB_ROW);
    Layout {
        list,
        divider_x: list.right + 12.0,
        pane,
        tabs_row,
        search_button: Rect::from_xywh(list.right - BUTTON, tabs_row.top, BUTTON, BUTTON),
        view: Rect::from_ltrb(list.left, tabs_row.bottom + 6.0, list.right, list.bottom),
    }
}

const TAB_LABEL: TextStyle = typography::BODY;
const TAB_COUNT: TextStyle = typography::LABEL;

fn tab_rects(row: Rect, counts: &[usize; 4]) -> [Rect; 4] {
    let mut left = row.left + 4.0;
    let mut rects = [Rect::from_xywh(0.0, 0.0, 0.0, 0.0); 4];
    for (index, rect) in rects.iter_mut().enumerate() {
        let width = kit::text_width(FILTERS[index], TAB_LABEL)
            + 5.0
            + kit::text_width(&counts[index].to_string(), TAB_COUNT);
        *rect = Rect::from_xywh(left, row.top, width, row.height());
        left += width + 16.0;
    }
    rects
}

struct PaneLayout {
    header: Rect,
    pin: Rect,
    delete: Rect,
    body: Rect,
    footer: Rect,
    copy: Rect,
    save: Rect,
}

fn pane_layout(pane: Rect) -> PaneLayout {
    let header = Rect::from_xywh(pane.left, pane.top, pane.width(), BUTTON);
    let delete = Rect::from_xywh(pane.right - BUTTON, pane.top, BUTTON, BUTTON);
    let pin = Rect::from_xywh(delete.left - BUTTON - 4.0, pane.top, BUTTON, BUTTON);
    let footer = Rect::from_ltrb(pane.left, pane.bottom - FOOTER, pane.right, pane.bottom);
    let copy = Rect::from_xywh(
        footer.right - COPY_WIDTH,
        footer.top + 1.0,
        COPY_WIDTH,
        FOOTER - 2.0,
    );
    let save = Rect::from_xywh(
        copy.left - BUTTON - 6.0,
        footer.center_y() - BUTTON / 2.0,
        BUTTON,
        BUTTON,
    );
    PaneLayout {
        save,
        header,
        pin,
        delete,
        body: Rect::from_ltrb(
            pane.left,
            header.bottom + 10.0,
            pane.right,
            footer.top - 8.0,
        ),
        footer,
        copy,
    }
}

fn max_scroll(model: &Model, view: Rect) -> f32 {
    (model.content_height - view.height()).max(0.0)
}

/// Scrolls so the selected row and a little of its neighbours are visible.
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
    let Some(&(_, top, height)) = model
        .lines
        .iter()
        .find(|(line, _, _)| matches!(line, Line::Item(position) if *position == selected))
    else {
        return;
    };
    // Keep the group header visible above the first row of a group.
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

fn line_at(model: &Model, view: Rect, point: Point) -> Option<Line> {
    if !view.contains(point) {
        return None;
    }
    let y = point.y - view.top + model.scroll;
    model
        .lines
        .iter()
        .find(|(_, top, height)| y >= *top && y < top + height)
        .map(|(line, _, _)| line.clone())
}

pub fn draw(ctx: &TabContext<'_>) -> bool {
    let (painter, grid, colors) = (ctx.painter, ctx.grid, ctx.colors);
    let hovered = |id: u16| ctx.hover == Some(DashboardHit::Tab(id));
    let model = STATE.with(|state| {
        let mut state = state.borrow_mut();
        state.grid = Some(grid);
        sync(&mut state);
        let model = build_model(&state);
        let view = layout(grid).view;
        state.scroll = state.scroll.clamp(0.0, max_scroll(&model, view));
        build_model(&state)
    });
    if !model.synced || !model.enabled || model.total == 0 {
        draw_empty(painter, grid, &model, colors);
        return true;
    }
    let layout = layout(grid);
    draw_list_header(painter, &layout, &model, &hovered, colors);
    draw_list(painter, &layout, &model, &hovered, colors);
    if let Some(pane) = layout.pane {
        painter.fill_rect(
            Rect::from_ltrb(
                layout.divider_x,
                grid.top + 4.0,
                layout.divider_x + 1.0,
                grid.bottom - 4.0,
            ),
            colors.border_subtle,
        );
        match model.selected.and_then(|index| model.shown.get(index)) {
            Some(entry) => draw_pane(painter, pane, entry, &model, &hovered, colors),
            None => kit::label(
                painter,
                "Nothing selected",
                typography::BODY,
                Rect::from_xywh(pane.left, pane.center_y() - 10.0, pane.width(), 20.0),
                colors.text_secondary,
            ),
        }
    }
    true
}

fn draw_empty(painter: Painter<'_>, grid: Rect, model: &Model, colors: &SemanticColors) {
    let (title, detail) = if !model.enabled {
        (
            "Clipboard history is off",
            "Turn it on in Settings to keep what you copy, ready to paste again.",
        )
    } else {
        (
            "Nothing copied yet",
            "Copy text, links or images and they will appear here.",
        )
    };
    let center = grid.center_y();
    let tile = Rect::from_xywh(grid.center_x() - 22.0, center - 58.0, 44.0, 44.0);
    painter.fill_round_rect(tile, Radius::uniform(12.0), colors.surface_raised);
    kit::glyph(
        painter,
        glyphs::COPY,
        kit::centered(tile, 22.0),
        colors.text_secondary,
    );
    crate::ui::components::text::draw_text(
        painter,
        title,
        typography::SECTION_TITLE,
        Rect::from_xywh(grid.left, center - 4.0, grid.width(), 22.0),
        crate::ui::components::text::Align::Center,
        colors.text_primary,
    );
    crate::ui::components::text::draw_text(
        painter,
        detail,
        typography::LABEL,
        Rect::from_xywh(grid.left, center + 20.0, grid.width(), 18.0),
        crate::ui::components::text::Align::Center,
        colors.text_secondary,
    );
}

fn draw_list_header(
    painter: Painter<'_>,
    layout: &Layout,
    model: &Model,
    hovered: &dyn Fn(u16) -> bool,
    colors: &SemanticColors,
) {
    if let Some(query) = &model.query {
        let field = layout.tabs_row;
        kit::search_field(painter, field, query, "Search clipboard", colors);
        let close = kit::search_close(field);
        let color = if hovered(ID_SEARCH_CLOSE) {
            colors.text_primary
        } else {
            colors.text_secondary
        };
        kit::glyph(painter, glyphs::CLOSE, kit::centered(close, 14.0), color);
        return;
    }
    for (index, rect) in tab_rects(layout.tabs_row, &model.counts).iter().enumerate() {
        if rect.right > layout.search_button.left - 4.0 {
            break;
        }
        let active = model.filter == index;
        let hover = hovered(ID_FILTER + index as u16);
        let label_color = if active || hover {
            colors.text_primary
        } else {
            colors.text_secondary
        };
        let label_width = kit::text_width(FILTERS[index], TAB_LABEL);
        let text_rect = Rect::from_ltrb(rect.left, rect.top, rect.right, rect.bottom - 4.0);
        kit::label(painter, FILTERS[index], TAB_LABEL, text_rect, label_color);
        kit::label(
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
        glyphs::SEARCH,
        hovered(ID_SEARCH),
        false,
        colors.text_secondary,
        colors,
    );
}

/// "503 × 164", for images.
fn dimensions(entry: &Entry) -> String {
    entry
        .size
        .map(|(width, height)| format!("{width} × {height}"))
        .unwrap_or_default()
}

/// Row subtitle: what it is (size, host, source app) and how long ago it was copied.
fn meta_line(entry: &Entry) -> String {
    let what = match entry.kind {
        Kind::Image => dimensions(entry),
        Kind::Link => host(&entry.preview).to_string(),
        Kind::Text if !entry.source.is_empty() => entry.source.clone(),
        Kind::Text => kit::plural(kit::word_count(&entry.preview), "word", "words"),
        Kind::Files => kit::plural(entry.preview.lines().count().max(1), "file", "files"),
    };
    format!("{what} · {}", kit::ago(entry.captured))
}

fn host(url: &str) -> &str {
    let rest = url
        .split_once("://")
        .map_or(url, |(_, rest)| rest)
        .trim_start_matches("www.");
    rest.split(['/', '?', '#']).next().unwrap_or(rest)
}

/// Draws `image` into `frame` (cover or contain) with rounded corners.
fn rounded_image(
    painter: Painter<'_>,
    image: &winisland_render::Image,
    frame: Rect,
    radius: f32,
    fit: ImageFit,
    colors: &SemanticColors,
) {
    let save = painter.save();
    painter.clip_round_rect(frame, Radius::uniform(radius));
    painter.draw_image(
        image,
        frame,
        &ImageOptions {
            fit,
            sampling: Sampling::LinearNone,
            ..ImageOptions::default()
        },
    );
    painter.restore_to(save);
    painter.stroke_round_rect(frame, Radius::uniform(radius), 1.0, colors.border_subtle);
}

fn kind_tile(painter: Painter<'_>, tile: Rect, entry: &Entry, colors: &SemanticColors) {
    if let Some(thumb) = entry
        .blob_id
        .as_deref()
        .and_then(images::get)
        .and_then(|images| images.thumb)
    {
        rounded_image(painter, &thumb, tile, 8.0, ImageFit::Cover, colors);
        return;
    }
    let kind = entry.kind;
    let (fill, ink) = match kind {
        Kind::Image => (Rgba::from_rgb(64, 124, 196), Rgba::WHITE),
        Kind::Link => (colors.accent_soft, colors.accent_primary),
        Kind::Text | Kind::Files => (colors.surface_overlay, colors.text_secondary),
    };
    painter.fill_round_rect(tile, Radius::uniform(8.0), fill);
    kit::glyph(painter, kind.glyph(), kit::centered(tile, 16.0), ink);
}

fn draw_list(
    painter: Painter<'_>,
    layout: &Layout,
    model: &Model,
    hovered: &dyn Fn(u16) -> bool,
    colors: &SemanticColors,
) {
    let view = layout.view;
    if model.shown.is_empty() {
        let text = if model.query.as_deref().is_some_and(|q| !q.trim().is_empty()) {
            "No matches"
        } else {
            "Nothing here yet"
        };
        kit::label(
            painter,
            text,
            typography::BODY,
            Rect::from_xywh(view.left + 8.0, view.top + 8.0, view.width() - 16.0, 22.0),
            colors.text_secondary,
        );
        return;
    }
    let save = painter.save();
    painter.clip_rect(view);
    for (line, top, height) in &model.lines {
        let y = view.top + top - model.scroll;
        if y + height < view.top || y > view.bottom {
            continue;
        }
        match line {
            Line::Header(name) => kit::label(
                painter,
                name,
                typography::LABEL,
                Rect::from_xywh(view.left + 8.0, y + 2.0, view.width() - 16.0, height - 2.0),
                colors.text_secondary,
            ),
            &Line::Item(position) => {
                let entry = &model.shown[position];
                let rect = Rect::from_xywh(view.left, y + 1.0, view.width(), height - 2.0);
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
                let tile = Rect::from_xywh(rect.left + 8.0, rect.center_y() - 14.0, 28.0, 28.0);
                kind_tile(painter, tile, entry, colors);
                let right = if entry.pinned {
                    rect.right - 28.0
                } else {
                    rect.right - 10.0
                };
                kit::label(
                    painter,
                    &entry.title,
                    typography::BODY,
                    Rect::from_ltrb(tile.right + 10.0, rect.top + 3.0, right, rect.top + 22.0),
                    colors.text_primary,
                );
                kit::label(
                    painter,
                    &meta_line(entry),
                    typography::CAPTION,
                    Rect::from_ltrb(tile.right + 10.0, rect.top + 22.0, right, rect.bottom - 3.0),
                    colors.text_secondary,
                );
                if entry.pinned {
                    let pin = Rect::from_xywh(rect.right - 24.0, rect.center_y() - 7.0, 14.0, 14.0);
                    kit::glyph_fill(painter, glyphs::PIN_FILL, pin, colors.text_secondary);
                    kit::glyph(painter, glyphs::PIN, pin, colors.text_secondary);
                }
            }
        }
    }
    painter.restore_to(save);
    // A slim scroll indicator when the list overflows.
    let max = max_scroll(model, view);
    if max > 0.0 {
        let track = view.height() - 8.0;
        let thumb = (track * view.height() / model.content_height).max(18.0);
        let top = view.top + 4.0 + (track - thumb) * (model.scroll / max);
        painter.fill_round_rect(
            Rect::from_xywh(view.right - 3.0, top, 3.0, thumb),
            Radius::uniform(1.5),
            colors.border_strong,
        );
    }
}

fn draw_pane(
    painter: Painter<'_>,
    pane: Rect,
    entry: &Entry,
    model: &Model,
    hovered: &dyn Fn(u16) -> bool,
    colors: &SemanticColors,
) {
    let parts = pane_layout(pane);
    let mut heading = format!("{} · {}", entry.kind.name(), kit::ago(entry.captured));
    if !entry.source.is_empty() {
        heading.push_str(&format!(" · {}", entry.source));
    }
    kit::label(
        painter,
        &heading,
        typography::LABEL,
        Rect::from_ltrb(
            parts.header.left,
            parts.header.top,
            parts.pin.left - 8.0,
            parts.header.bottom,
        ),
        colors.text_secondary,
    );
    kit::icon_button(
        painter,
        parts.pin,
        glyphs::PIN,
        hovered(ID_PIN),
        entry.pinned,
        if entry.pinned {
            colors.text_primary
        } else {
            colors.text_secondary
        },
        colors,
    );
    if entry.pinned {
        kit::glyph_fill(
            painter,
            glyphs::PIN_FILL,
            kit::centered(parts.pin, 16.0),
            colors.text_primary,
        );
    }
    kit::icon_button(
        painter,
        parts.delete,
        glyphs::TRASH,
        hovered(ID_DELETE),
        false,
        if hovered(ID_DELETE) {
            colors.state_error
        } else {
            colors.text_secondary
        },
        colors,
    );

    // Body.
    let save = painter.save();
    painter.clip_rect(parts.body);
    match entry.kind {
        Kind::Image => {
            let (width, height) = entry.size.unwrap_or((4, 3));
            let aspect = width.max(1) as f32 / height.max(1) as f32;
            // Scale to fit the body, never past the image's own pixel size.
            let fit_width = parts
                .body
                .width()
                .min(parts.body.height() * aspect)
                .min(width as f32)
                .max(24.0);
            let frame = Rect::from_xywh(
                parts.body.left,
                parts.body.top,
                fit_width,
                (fit_width / aspect).clamp(24.0, parts.body.height()),
            );
            match entry
                .blob_id
                .as_deref()
                .and_then(images::get)
                .and_then(|images| images.preview)
            {
                Some(preview) => {
                    rounded_image(painter, &preview, frame, 12.0, ImageFit::Fill, colors);
                }
                None => {
                    painter.fill_round_rect(frame, Radius::uniform(12.0), colors.surface_raised);
                    kit::glyph(
                        painter,
                        glyphs::IMAGE,
                        kit::centered(frame, 24.0_f32.min(frame.height() - 4.0)),
                        colors.text_secondary,
                    );
                }
            }
        }
        Kind::Link => {
            kit::draw_wrapped(
                painter,
                &entry.preview,
                typography::BODY,
                21.0,
                parts.body,
                colors.accent_primary,
            );
        }
        Kind::Text | Kind::Files => {
            kit::draw_wrapped(
                painter,
                &entry.preview,
                typography::BODY,
                21.0,
                parts.body,
                colors.text_primary,
            );
        }
    }
    painter.restore_to(save);

    // Footer: counts and the Copy pill.
    let counts = match entry.kind {
        Kind::Text => {
            let chars = entry.preview.chars().count();
            format!(
                "{} · {}",
                kit::plural(kit::word_count(&entry.preview), "word", "words"),
                kit::plural(chars, "char", "chars")
            )
        }
        Kind::Link => format!("Link · {}", host(&entry.preview)),
        Kind::Image => match entry
            .blob_id
            .as_deref()
            .and_then(images::get)
            .filter(|images| images.file_bytes > 0)
        {
            Some(images) => format!(
                "{} · {}",
                dimensions(entry),
                kit::byte_size(images.file_bytes)
            ),
            None => dimensions(entry),
        },
        Kind::Files => kit::plural(entry.preview.lines().count().max(1), "file", "files"),
    };
    kit::label(
        painter,
        &counts,
        typography::LABEL,
        Rect::from_ltrb(
            parts.footer.left,
            parts.footer.top,
            parts.save.left - 8.0,
            parts.footer.bottom,
        ),
        colors.text_secondary,
    );
    kit::icon_button(
        painter,
        parts.save,
        glyphs::DOWNLOAD,
        hovered(ID_SAVE),
        false,
        colors.text_secondary,
        colors,
    );
    let can_copy = entry.kind != Kind::Image || entry.blob_id.is_some();
    let copied = model.copied == Some(entry.id);
    let (fill, ink) = if !can_copy {
        (colors.surface_interactive, colors.text_disabled)
    } else if hovered(ID_COPY) {
        (colors.accent_primary.with_alpha(225), colors.surface_base)
    } else {
        (colors.accent_primary, colors.surface_base)
    };
    painter.fill_round_rect(parts.copy, Radius::uniform(parts.copy.height() / 2.0), fill);
    let text = if copied { "Copied" } else { "Copy" };
    let style = TextStyle {
        size: 13.0,
        weight: 600,
        tabular: false,
    };
    let content = 14.0 + 6.0 + kit::text_width(text, style);
    let left = parts.copy.center_x() - content / 2.0;
    kit::glyph(
        painter,
        if copied { glyphs::CHECK } else { glyphs::COPY },
        Rect::from_xywh(left, parts.copy.center_y() - 7.0, 14.0, 14.0),
        ink,
    );
    kit::label(
        painter,
        text,
        style,
        Rect::from_ltrb(
            left + 20.0,
            parts.copy.top,
            parts.copy.right,
            parts.copy.bottom,
        ),
        ink,
    );
}

pub fn custom_layout() -> bool {
    true
}

pub fn hit(grid: Rect, point: Point) -> Option<DashboardHit> {
    if !grid.contains(point) {
        return None;
    }
    let tab = |id: u16| Some(DashboardHit::Tab(id));
    let model = STATE.with(|state| build_model(&state.borrow()));
    if !model.synced || !model.enabled || model.total == 0 {
        return tab(ID_INERT);
    }
    let layout = layout(grid);
    if model.query.is_some() {
        if kit::search_close(layout.tabs_row).contains(point) {
            return tab(ID_SEARCH_CLOSE);
        }
    } else {
        if layout.search_button.contains(point) {
            return tab(ID_SEARCH);
        }
        if let Some(index) = tab_rects(layout.tabs_row, &model.counts)
            .iter()
            .position(|rect| rect.right <= layout.search_button.left - 4.0 && rect.contains(point))
        {
            return tab(ID_FILTER + index as u16);
        }
    }
    if let Some(Line::Item(position)) = line_at(&model, layout.view, point)
        && position < MAX_ROW_IDS
    {
        return tab(ID_ROW + position as u16);
    }
    if let Some(pane) = layout.pane
        && model.selected.is_some()
    {
        let parts = pane_layout(pane);
        if parts.pin.contains(point) {
            return tab(ID_PIN);
        }
        if parts.delete.contains(point) {
            return tab(ID_DELETE);
        }
        if parts.copy.contains(point) {
            return tab(ID_COPY);
        }
        if parts.save.contains(point) {
            return tab(ID_SAVE);
        }
    }
    tab(ID_INERT)
}

/// Asks the history service to put `entry` back on the clipboard.
fn request_copy(state: &mut State, entry: &Entry) {
    history::request(ClipboardRequest::Copy(entry.id));
    state.copied = Some((entry.id, Instant::now()));
}

fn select_offset(state: &mut State, delta: isize) {
    let model = build_model(state);
    if model.shown.is_empty() {
        return;
    }
    let current = model.selected.unwrap_or(0) as isize;
    let next = (current + delta).clamp(0, model.shown.len() as isize - 1) as usize;
    state.selected = Some(model.shown[next].id);
    reveal(state);
}

pub fn apply(hit: DashboardHit) -> bool {
    let id = match hit {
        DashboardHit::Tab(id) if (400..500).contains(&id) => id,
        // The legacy page's rows sit under this tab; never let them act.
        DashboardHit::ClipRow(_) => return true,
        _ => return false,
    };
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        let model = build_model(&state);
        let selected = model
            .selected
            .and_then(|index| model.shown.get(index))
            .cloned();
        match id {
            ID_FILTER..ID_SEARCH => {
                state.filter = usize::from(id - ID_FILTER);
                state.scroll = 0.0;
                state.selected = None;
            }
            ID_SEARCH => {
                state.query = Some(String::new());
                state.scroll = 0.0;
            }
            ID_SEARCH_CLOSE => {
                state.query = None;
                reveal(&mut state);
            }
            ID_PIN => {
                if let Some(entry) = selected {
                    history::request(ClipboardRequest::Pin(entry.id, !entry.pinned));
                    state.pending_pins.insert(entry.id, !entry.pinned);
                    reveal(&mut state);
                }
            }
            ID_DELETE => {
                if let Some(entry) = selected {
                    // Move the selection to the neighbour before the row disappears.
                    let position = model.selected.unwrap_or(0);
                    let next = model
                        .shown
                        .get(position + 1)
                        .or_else(|| position.checked_sub(1).and_then(|p| model.shown.get(p)))
                        .map(|entry| entry.id);
                    history::request(ClipboardRequest::Delete(entry.id));
                    state.pending_deletes.push(entry.id);
                    state.selected = next;
                    reveal(&mut state);
                }
            }
            ID_COPY => {
                if let Some(entry) = selected {
                    request_copy(&mut state, &entry);
                }
            }
            ID_SAVE => {
                if let Some(entry) = selected {
                    save_to_file(&entry);
                }
            }
            _ if id >= ID_ROW && usize::from(id - ID_ROW) < MAX_ROW_IDS => {
                if let Some(entry) = model.shown.get(usize::from(id - ID_ROW)) {
                    let narrow = state.grid.is_some_and(|grid| layout(grid).pane.is_none());
                    state.selected = Some(entry.id);
                    if narrow {
                        request_copy(&mut state, entry);
                    }
                    reveal(&mut state);
                }
            }
            _ => {}
        }
    });
    true
}

pub fn wants_keyboard() -> bool {
    STATE.with(|state| state.borrow().query.is_some())
}

pub fn handle_key(key: &Key) -> bool {
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        let Some(query) = state.query.as_mut() else {
            return false;
        };
        match key {
            Key::Escape => {
                state.query = None;
                reveal(&mut state);
            }
            Key::Backspace => {
                query.pop();
                state.scroll = 0.0;
                state.selected = None;
            }
            Key::Character(text) => {
                if query.chars().count() < 80 {
                    query.extend(text.chars().filter(|character| !character.is_control()));
                }
                state.scroll = 0.0;
                state.selected = None;
            }
            Key::ArrowDown => select_offset(&mut state, 1),
            Key::ArrowUp => select_offset(&mut state, -1),
            Key::Enter => {
                let model = build_model(&state);
                if let Some(entry) = model.selected.and_then(|index| model.shown.get(index)) {
                    request_copy(&mut state, entry);
                }
            }
            _ => {}
        }
        true
    })
}

/// Scrolls the history list by wheel `lines` (positive = wheel up, toward the newest rows).
/// Call from the panel's wheel handler while this tab is visible, with the same `grid` the
/// tab draws into and the pointer in page coordinates. `true` when the list took the wheel.
pub fn scroll(grid: Rect, point: Point, lines: f32) -> bool {
    let view = layout(grid).view;
    if !view.contains(point) {
        return false;
    }
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        let model = build_model(&state);
        state.scroll =
            (state.scroll - lines * ROW_HEIGHT).clamp(0.0, max_scroll(&model, view));
    });
    true
}

/// Asks where to save `entry` (a PNG for images, a text file otherwise) and writes it. The
/// dialog and file work run on their own thread so the panel keeps drawing.
fn save_to_file(entry: &Entry) {
    let blob_id = entry.blob_id.clone();
    let text = entry.preview.clone();
    let stem: String = entry
        .title
        .chars()
        .filter(|character| character.is_alphanumeric() || *character == ' ')
        .take(40)
        .collect::<String>()
        .trim()
        .to_string();
    let _ = std::thread::Builder::new()
        .name("clipboard-save".into())
        .spawn(move || {
            let (name, extension, label) = match &blob_id {
                Some(_) => ("Clipboard image".to_string(), "png", "PNG image"),
                None if stem.is_empty() => ("Clipboard".to_string(), "txt", "Text"),
                None => (stem, "txt", "Text"),
            };
            let Some(path) = rfd::FileDialog::new()
                .set_title("Save clipboard item")
                .set_file_name(format!("{name}.{extension}"))
                .add_filter(label, &[extension])
                .save_file()
            else {
                return;
            };
            let bytes = match &blob_id {
                Some(blob_id) => match history::read_image_blob(blob_id) {
                    Some(png) => png,
                    None => {
                        log::warn!("Cannot save clipboard image: its file is missing");
                        return;
                    }
                },
                None => text.into_bytes(),
            };
            if let Err(error) = std::fs::write(&path, bytes) {
                log::warn!("Cannot save clipboard item: {error}");
            }
        });
}

/// Picks up newly published history and finished image decodes.
pub fn poll(_now: Instant) {
    STATE.with(|state| sync(&mut state.borrow_mut()));
    images::pump();
}
