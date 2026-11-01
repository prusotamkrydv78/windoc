//! Clipboard tab: filterable, searchable history on the left, a preview with actions on the
//! right.
//!
//! Rows come from the dashboard's `CLIPBOARD_VIEW` (published by `sync_clipboard`). The
//! view carries text only, so capture times, pins and kinds are read once from the saved
//! history and then tracked here as new items arrive. Copying goes through the existing
//! copy-again request; pin and delete have no app hook yet, so they apply to this view for
//! the current session only.

pub mod kit;

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant, SystemTime};

use winisland_platform::Key;
use winisland_render::tokens::{SemanticColors, TextStyle, typography};
use winisland_render::{Painter, Point, Radius, Rect, Rgba};

use super::TabContext;
use crate::ui::dashboard::DashboardHit;
use kit::glyphs;

const ID_FILTER: u16 = 400;
const ID_SEARCH: u16 = 404;
const ID_SEARCH_CLOSE: u16 = 405;
const ID_PIN: u16 = 406;
const ID_DELETE: u16 = 407;
const ID_COPY: u16 = 408;
const ID_ROW: u16 = 410;
const MAX_ROW_IDS: usize = 80;
/// Claims the rest of the page so the legacy row hit test never fires under this tab.
const ID_INERT: u16 = 499;

const TAB_ROW: f32 = 28.0;
const ROW_HEIGHT: f32 = 42.0;
const HEADER_HEIGHT: f32 = 22.0;
const BUTTON: f32 = 28.0;
const COPY_WIDTH: f32 = 86.0;
const FOOTER: f32 = 32.0;
/// Below this grid width the preview pane is dropped and a click copies directly.
const SPLIT_MIN_WIDTH: f32 = 460.0;
const COPIED_FOR: Duration = Duration::from_millis(1600);

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

#[derive(Clone, Copy, Debug)]
struct Meta {
    captured: Option<SystemTime>,
    pinned: bool,
    kind: Option<Kind>,
}

#[derive(Default)]
struct State {
    filter: usize,
    /// `Some` while the search field is open.
    query: Option<String>,
    /// Key of the selected item, so selection survives new captures.
    selected: Option<String>,
    scroll: f32,
    meta: HashMap<String, Meta>,
    revision: Option<u64>,
    first_key: Option<String>,
    /// Items removed from this view (session only; history has no delete hook).
    hidden: HashSet<String>,
    /// Pin changes made here (session only; history has no pin hook).
    pins: HashMap<String, bool>,
    copied: Option<(String, Instant)>,
    /// Last drawn grid, so clicks and keys can keep the selection in view.
    grid: Option<Rect>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

#[derive(Clone)]
struct Entry {
    /// Index in history (newest first), for the copy-again request.
    index: usize,
    key: String,
    title: String,
    source: String,
    preview: String,
    kind: Kind,
    captured: Option<SystemTime>,
    pinned: bool,
}

#[derive(Clone, Copy)]
enum Line {
    Header(&'static str),
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
    copied: Option<String>,
}

fn key_of(title: &str, preview: &str) -> String {
    format!("{title}\u{1f}{preview}")
}

fn kind_of(body: &winisland_core::clipboard::ClipboardBody) -> Kind {
    use winisland_core::clipboard::ClipboardBody;
    match body {
        ClipboardBody::Text(_) => Kind::Text,
        ClipboardBody::Link(_) => Kind::Link,
        ClipboardBody::Image { .. } => Kind::Image,
        ClipboardBody::Files(_) => Kind::Files,
    }
}

/// Kind of a row captured after the saved history was read, from its text alone.
fn guess_kind(title: &str, preview: &str) -> Kind {
    let trimmed = preview.trim();
    let lower = trimmed.to_ascii_lowercase();
    if !trimmed.contains(char::is_whitespace)
        && (lower.starts_with("http://") || lower.starts_with("https://"))
    {
        return Kind::Link;
    }
    if let Some((width, height)) = trimmed.split_once(" × ")
        && width.parse::<u32>().is_ok()
        && height.parse::<u32>().is_ok()
        && title
            == winisland_core::i18n::tr_args("clipboard_image", &[&format!("{width}×{height}")])
    {
        return Kind::Image;
    }
    let lines: Vec<&str> = trimmed.lines().collect();
    let looks_like_path = |line: &str| {
        let bytes = line.as_bytes();
        (bytes.len() > 2 && bytes[1] == b':' && (bytes[2] == b'\\' || bytes[2] == b'/'))
            || line.starts_with("\\\\")
    };
    if !lines.is_empty() && lines.iter().all(|line| looks_like_path(line)) {
        let named = lines.len() == 1
            && std::path::Path::new(lines[0])
                .file_name()
                .is_some_and(|name| name.to_string_lossy() == title);
        let counted =
            title == winisland_core::i18n::tr_args("clipboard_files", &[&lines.len().to_string()]);
        if named || counted {
            return Kind::Files;
        }
    }
    Kind::Text
}

/// Capture times, pins and kinds from the saved (encrypted) history, keyed like the view.
fn load_saved_meta() -> HashMap<String, Meta> {
    use winisland_core::clipboard::ClipboardSnapshot;
    const MAX_FILE_BYTES: u64 = 96 * 1024 * 1024;
    // Must match `core::clipboard_history`; it binds the file to the app, not a secret.
    const ENTROPY: &[u8] = b"winisland-next/clipboard/v1";
    let bytes = match crate::core::json_store::read_bytes(
        crate::core::clipboard_history::FILE_NAME,
        MAX_FILE_BYTES,
    ) {
        Ok(Some(bytes)) => bytes,
        _ => return HashMap::new(),
    };
    let Ok(plain) = winisland_platform_windows::dpapi::unprotect(&bytes, Some(ENTROPY)) else {
        return HashMap::new();
    };
    let Ok(snapshot) = serde_json::from_slice::<ClipboardSnapshot>(&plain) else {
        return HashMap::new();
    };
    snapshot
        .items
        .iter()
        .map(|item| {
            let (title, _) = crate::ui::dashboard::clipboard_row(item);
            let preview = crate::ui::dashboard::clipboard_preview(item);
            (
                key_of(&title, &preview),
                Meta {
                    captured: Some(item.captured_at),
                    pinned: item.pinned,
                    kind: Some(kind_of(&item.body)),
                },
            )
        })
        .collect()
}

fn filter_matches(filter: usize, kind: Kind) -> bool {
    match filter {
        1 => kind == Kind::Text,
        2 => kind == Kind::Image,
        3 => kind == Kind::Link,
        _ => true,
    }
}

fn build_model(state: &State) -> Model {
    let (synced, enabled, all) = crate::ui::dashboard::CLIPBOARD_VIEW.with(|view| {
        let view = view.borrow();
        let entries: Vec<Entry> = view
            .rows
            .iter()
            .zip(view.previews.iter())
            .enumerate()
            .filter_map(|(index, ((title, source), preview))| {
                let key = key_of(title, preview);
                if state.hidden.contains(&key) {
                    return None;
                }
                let meta = state.meta.get(&key);
                let kind = meta
                    .and_then(|meta| meta.kind)
                    .unwrap_or_else(|| guess_kind(title, preview));
                let pinned = state
                    .pins
                    .get(&key)
                    .copied()
                    .unwrap_or_else(|| meta.is_some_and(|meta| meta.pinned));
                Some(Entry {
                    index,
                    title: if kind == Kind::Image {
                        "Image".to_string()
                    } else {
                        title.clone()
                    },
                    key,
                    source: source.clone(),
                    preview: preview.clone(),
                    kind,
                    captured: meta.and_then(|meta| meta.captured),
                    pinned,
                })
            })
            .collect();
        (view.synced, view.enabled, entries)
    });
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
    let matching: Vec<Entry> = all
        .into_iter()
        .filter(|entry| filter_matches(state.filter, entry.kind))
        .filter(|entry| {
            needle.as_ref().is_none_or(|needle| {
                entry.title.to_lowercase().contains(needle)
                    || entry.preview.to_lowercase().contains(needle)
                    || entry.source.to_lowercase().contains(needle)
            })
        })
        .collect();
    // Pinned, then today, then earlier; history order inside each group.
    let today = kit::today();
    let group = |entry: &Entry| {
        if entry.pinned {
            0
        } else if entry
            .captured
            .is_some_and(|time| kit::local_day(time) == today)
        {
            1
        } else {
            2
        }
    };
    let mut shown = matching;
    shown.sort_by_key(|entry| (group(entry), entry.index));
    let mut lines = Vec::new();
    let mut top = 0.0;
    let mut current = None;
    for (position, entry) in shown.iter().enumerate() {
        let entry_group = group(entry);
        if current != Some(entry_group) {
            current = Some(entry_group);
            let name = ["Pinned", "Today", "Earlier"][entry_group];
            lines.push((Line::Header(name), top, HEADER_HEIGHT));
            top += HEADER_HEIGHT;
        }
        lines.push((Line::Item(position), top, ROW_HEIGHT));
        top += ROW_HEIGHT;
    }
    let selected = state
        .selected
        .as_ref()
        .and_then(|key| shown.iter().position(|entry| &entry.key == key))
        .or_else(|| (!shown.is_empty()).then_some(0));
    let copied = state
        .copied
        .as_ref()
        .filter(|(_, at)| at.elapsed() < COPIED_FOR)
        .map(|(key, _)| key.clone());
    Model {
        synced,
        enabled,
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
    PaneLayout {
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
    let Some((_, top, height)) = model
        .lines
        .iter()
        .find(|(line, _, _)| matches!(line, Line::Item(position) if *position == selected))
        .copied()
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
        .map(|(line, _, _)| *line)
}

pub fn draw(ctx: &TabContext<'_>) -> bool {
    let (painter, grid, colors) = (ctx.painter, ctx.grid, ctx.colors);
    let hovered = |id: u16| ctx.hover == Some(DashboardHit::Tab(id));
    let model = STATE.with(|state| {
        let mut state = state.borrow_mut();
        state.grid = Some(grid);
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

fn meta_line(entry: &Entry) -> String {
    let what = match entry.kind {
        Kind::Text => kit::plural(kit::word_count(&entry.preview), "word", "words"),
        Kind::Image => entry.preview.clone(),
        Kind::Link => host(&entry.preview).to_string(),
        Kind::Files => kit::plural(entry.preview.lines().count().max(1), "file", "files"),
    };
    match entry.captured {
        Some(time) => format!("{what} · {}", kit::ago(time)),
        None if !entry.source.is_empty() => format!("{what} · {}", entry.source),
        None => what,
    }
}

fn host(url: &str) -> &str {
    let rest = url
        .split_once("://")
        .map_or(url, |(_, rest)| rest)
        .trim_start_matches("www.");
    rest.split(['/', '?', '#']).next().unwrap_or(rest)
}

fn kind_tile(painter: Painter<'_>, tile: Rect, kind: Kind, colors: &SemanticColors) {
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
        match *line {
            Line::Header(name) => kit::label(
                painter,
                name,
                typography::LABEL,
                Rect::from_xywh(view.left + 8.0, y + 2.0, view.width() - 16.0, height - 2.0),
                colors.text_secondary,
            ),
            Line::Item(position) => {
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
                kind_tile(painter, tile, entry.kind, colors);
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
    let mut heading = entry.kind.name().to_string();
    if let Some(time) = entry.captured {
        heading.push_str(&format!(" · {}", kit::ago(time)));
    }
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
            let frame = Rect::from_xywh(
                parts.body.left,
                parts.body.top,
                parts.body.width().min(220.0),
                parts.body.height().min(110.0),
            );
            painter.fill_round_rect(frame, Radius::uniform(12.0), colors.surface_raised);
            kit::glyph(
                painter,
                glyphs::IMAGE,
                Rect::from_xywh(frame.center_x() - 16.0, frame.center_y() - 24.0, 32.0, 32.0),
                colors.text_secondary,
            );
            crate::ui::components::text::draw_text(
                painter,
                &format!("{} pixels", entry.preview),
                typography::LABEL,
                Rect::from_xywh(frame.left, frame.center_y() + 12.0, frame.width(), 18.0),
                crate::ui::components::text::Align::Center,
                colors.text_secondary,
            );
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
        Kind::Image => "Images can't be copied back yet".to_string(),
        Kind::Files => kit::plural(entry.preview.lines().count().max(1), "file", "files"),
    };
    kit::label(
        painter,
        &counts,
        typography::LABEL,
        Rect::from_ltrb(
            parts.footer.left,
            parts.footer.top,
            parts.copy.left - 10.0,
            parts.footer.bottom,
        ),
        colors.text_secondary,
    );
    let can_copy = entry.kind != Kind::Image;
    let copied = model.copied.as_deref() == Some(entry.key.as_str());
    let (fill, ink) = if !can_copy {
        (colors.surface_interactive, colors.text_disabled)
    } else if hovered(ID_COPY) {
        (colors.text_primary.with_alpha(225), colors.surface_base)
    } else {
        (colors.text_primary, colors.surface_base)
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
    }
    tab(ID_INERT)
}

/// Asks the app to put history item `index` back on the clipboard.
fn request_copy(state: &mut State, entry: &Entry) {
    if entry.kind == Kind::Image {
        return;
    }
    crate::ui::dashboard::CLIPBOARD_COPY.with(|cell| cell.set(Some(entry.index)));
    state.copied = Some((entry.key.clone(), Instant::now()));
}

fn select_offset(state: &mut State, delta: isize) {
    let model = build_model(state);
    if model.shown.is_empty() {
        return;
    }
    let current = model.selected.unwrap_or(0) as isize;
    let next = (current + delta).clamp(0, model.shown.len() as isize - 1) as usize;
    state.selected = Some(model.shown[next].key.clone());
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
                    state.pins.insert(entry.key.clone(), !entry.pinned);
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
                        .map(|entry| entry.key.clone());
                    state.hidden.insert(entry.key);
                    state.selected = next;
                    reveal(&mut state);
                }
            }
            ID_COPY => {
                if let Some(entry) = selected {
                    request_copy(&mut state, &entry);
                }
            }
            _ if id >= ID_ROW && usize::from(id - ID_ROW) < MAX_ROW_IDS => {
                if let Some(entry) = model.shown.get(usize::from(id - ID_ROW)) {
                    let narrow = state.grid.is_some_and(|grid| layout(grid).pane.is_none());
                    state.selected = Some(entry.key.clone());
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

/// Tracks capture times and kinds as history changes.
pub fn poll(_now: Instant) {
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        crate::ui::dashboard::CLIPBOARD_VIEW.with(|view| {
            let view = view.borrow();
            if !view.synced || state.revision == Some(view.revision) {
                return;
            }
            let first_time = state.revision.is_none();
            state.revision = Some(view.revision);
            let saved = if first_time && view.enabled {
                load_saved_meta()
            } else {
                HashMap::new()
            };
            let now = SystemTime::now();
            for (index, ((title, _), preview)) in
                view.rows.iter().zip(view.previews.iter()).enumerate()
            {
                let key = key_of(title, preview);
                if first_time {
                    let meta = saved.get(&key).copied().unwrap_or(Meta {
                        captured: None,
                        pinned: false,
                        kind: None,
                    });
                    state.meta.insert(key, meta);
                } else if !state.meta.contains_key(&key) {
                    state.meta.insert(
                        key,
                        Meta {
                            captured: Some(now),
                            pinned: false,
                            kind: None,
                        },
                    );
                } else if index == 0 && state.first_key.as_deref() != Some(key.as_str()) {
                    // Copied again: it moved back to the top.
                    if let Some(meta) = state.meta.get_mut(&key) {
                        meta.captured = Some(now);
                    }
                    state.hidden.remove(&key);
                }
            }
            state.first_key = view
                .rows
                .first()
                .zip(view.previews.first())
                .map(|((title, _), preview)| key_of(title, preview));
        });
    });
}
