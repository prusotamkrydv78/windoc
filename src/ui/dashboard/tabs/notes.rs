//! Notes tab: grouped, searchable note list on the left, a formatted note with a toolbar
//! on the right.
//!
//! Notes live in the dashboard's `notes.json` store (`load_notes` / `NOTE_ROWS`). Bodies are
//! plain text with light Markdown: `# `/`## ` headings, `- ` bullets, `- [ ] `/`- [x] `
//! checklists and `**bold**`, `*italic*`, `<u>underline</u>`, `~~strike~~` spans. Reading
//! shows the formatted note; clicking the text edits the raw text with the shared
//! `TextArea`, and the toolbar inserts the markers at the caret. Edits save automatically.

use std::cell::RefCell;
use std::time::{Duration, Instant, SystemTime};

use winisland_core::notes::NoteStore;
use winisland_core::text_edit::{AreaKey, TextArea};
use winisland_platform::Key;
use winisland_render::text::FontManager;
use winisland_render::tokens::{TextStyle, typography};
use winisland_render::{FontStyle, Painter, Point, Radius, Rect, Rgba, StrokeCap, Vec2};

use super::TabContext;
use super::clipboard::kit::{self, glyphs};
use crate::ui::dashboard::DashboardHit;

const ID_BOLD: u16 = 500;
const ID_ITALIC: u16 = 501;
const ID_UNDERLINE: u16 = 502;
const ID_STRIKE: u16 = 503;
const ID_H1: u16 = 504;
const ID_H2: u16 = 505;
const ID_TEXT: u16 = 506;
const ID_BULLETS: u16 = 507;
const ID_CHECKLIST: u16 = 508;
const ID_COLOR: u16 = 509;
const ID_PIN: u16 = 510;
const ID_COPY: u16 = 511;
const ID_DELETE: u16 = 512;
const ID_SEARCH: u16 = 513;
const ID_SEARCH_CLOSE: u16 = 514;
const ID_NEW: u16 = 515;
const ID_ROW: u16 = 520;
const MAX_ROWS: usize = 40;
const ID_CHECK: u16 = 560;
const MAX_CHECKS: usize = 30;
const ID_BODY: u16 = 590;
const ID_INERT: u16 = 599;

const LIST_WIDTH: f32 = 220.0;
const ROW_HEIGHT: f32 = 44.0;
const GROUP_HEIGHT: f32 = 22.0;
const BAR: f32 = 28.0;
const MAX_NOTE_CHARS: usize = 64 * 1024;
const AUTOSAVE_AFTER: Duration = Duration::from_millis(1500);
const DELETE_ARMED_FOR: Duration = Duration::from_secs(3);

/// Note colours, stored as a `color-<name>` tag so they persist with the note.
const COLORS: [(&str, Rgba); 6] = [
    ("red", Rgba::from_rgb(255, 99, 88)),
    ("orange", Rgba::from_rgb(255, 159, 64)),
    ("yellow", Rgba::from_rgb(255, 207, 72)),
    ("green", Rgba::from_rgb(76, 200, 120)),
    ("blue", Rgba::from_rgb(84, 150, 255)),
    ("purple", Rgba::from_rgb(176, 120, 240)),
];

#[derive(Clone)]
struct NoteInfo {
    id: u64,
    title: String,
    body: String,
    pinned: bool,
    updated: SystemTime,
    checklist: Vec<(String, bool)>,
    tags: Vec<String>,
}

impl NoteInfo {
    fn color(&self) -> Option<usize> {
        self.tags.iter().find_map(|tag| {
            let name = tag.strip_prefix("color-")?;
            COLORS.iter().position(|(candidate, _)| *candidate == name)
        })
    }

    /// `(done, total)` across body checkboxes and the stored checklist.
    fn progress(&self) -> (usize, usize) {
        let mut done = self.checklist.iter().filter(|(_, done)| *done).count();
        let mut total = self.checklist.len();
        for line in self.body.lines() {
            match checkbox(line) {
                Some((true, _)) => {
                    done += 1;
                    total += 1;
                }
                Some((false, _)) => total += 1,
                None => {}
            }
        }
        (done, total)
    }
}

/// What a checkbox target toggles.
#[derive(Clone, Copy)]
enum Check {
    /// Line index in the body.
    Body(usize),
    /// Index in the stored checklist.
    Item(usize),
}

struct Editing {
    /// `None` until a new note is first saved.
    id: Option<u64>,
    area: TextArea,
    dirty: bool,
    last_edit: Instant,
}

#[derive(Default)]
struct State {
    notes: Vec<NoteInfo>,
    signature: Option<Vec<(u64, String, String)>>,
    selected: Option<u64>,
    /// A new, unsaved note is open.
    draft: bool,
    editing: Option<Editing>,
    query: Option<String>,
    scroll: f32,
    delete_armed: Option<Instant>,
    copied_at: Option<Instant>,
    /// Click targets from the last draw, topmost last.
    targets: Vec<(Rect, u16)>,
    checks: Vec<Check>,
    /// Note ids of the list rows drawn last, by slot.
    row_ids: Vec<u64>,
    /// Visual lines of the editor from the last draw: rect, first char index, chars.
    segments: Vec<(Rect, usize, Vec<char>)>,
    last_point: Option<Point>,
    grid: Option<Rect>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

// ---- Store access -----------------------------------------------------------------------

fn infos(store: &NoteStore) -> Vec<NoteInfo> {
    store
        .list()
        .into_iter()
        .map(|note| NoteInfo {
            id: note.id(),
            title: note.title().to_string(),
            body: note.body().to_string(),
            pinned: note.pinned(),
            updated: note.updated(),
            checklist: note
                .checklist()
                .iter()
                .map(|item| (item.text.clone(), item.done))
                .collect(),
            tags: note.tags().to_vec(),
        })
        .collect()
}

fn reload(state: &mut State) {
    let store = crate::ui::dashboard::load_notes();
    state.notes = infos(&store);
    state.signature = Some(crate::ui::dashboard::NOTE_ROWS.with(|rows| rows.borrow().clone()));
}

/// Changes the store, saves it and refreshes both this tab and the dashboard's rows.
fn with_store(state: &mut State, change: impl FnOnce(&mut NoteStore) -> bool) {
    let mut store = crate::ui::dashboard::load_notes();
    if !change(&mut store) {
        return;
    }
    if let Err(error) =
        crate::core::json_store::write(crate::ui::dashboard::NOTES_FILE, &store.snapshot())
    {
        log::warn!("Cannot save notes: {error}");
        return;
    }
    let rows = crate::ui::dashboard::note_rows(&store);
    crate::ui::dashboard::NOTE_ROWS.with(|cell| *cell.borrow_mut() = rows.clone());
    state.notes = infos(&store);
    state.signature = Some(rows);
}

/// Saves the open editor if it changed.
fn persist(state: &mut State) {
    let Some(editing) = state.editing.as_mut() else {
        return;
    };
    if !editing.dirty {
        return;
    }
    editing.dirty = false;
    let body = editing.area.text().to_string();
    let id = editing.id;
    let now = SystemTime::now();
    let mut created = None;
    match id {
        Some(id) => with_store(state, |store| {
            store.update_body(id, &body, now).unwrap_or_else(|error| {
                log::warn!("Cannot save note: {error}");
                false
            })
        }),
        None if body.trim().is_empty() => {}
        None => with_store(state, |store| match store.create(&body, now) {
            Ok(id) => {
                created = Some(id);
                true
            }
            Err(error) => {
                log::warn!("Cannot add note: {error}");
                false
            }
        }),
    }
    if let Some(id) = created {
        if let Some(editing) = state.editing.as_mut() {
            editing.id = Some(id);
        }
        state.selected = Some(id);
        state.draft = false;
    }
}

fn stop_editing(state: &mut State) {
    persist(state);
    state.editing = None;
    state.draft = false;
}

fn selected_note(state: &State) -> Option<&NoteInfo> {
    if state.draft {
        return None;
    }
    let id = state.selected?;
    state.notes.iter().find(|note| note.id == id)
}

/// A text area holding `text` with its caret at char `caret`.
fn area_at(text: &str, caret: usize) -> TextArea {
    let mut area = TextArea::with_text(text, MAX_NOTE_CHARS);
    let before: String = text.chars().take(caret).collect();
    let line = before.matches('\n').count();
    let column = before.chars().rev().take_while(|c| *c != '\n').count();
    let last_line = text.matches('\n').count();
    for _ in line..last_line {
        area.apply(AreaKey::Up);
    }
    area.apply(AreaKey::Home);
    for _ in 0..column {
        area.apply(AreaKey::Right);
    }
    area
}

fn caret_index(area: &TextArea) -> usize {
    let (line, column) = area.caret_line_column();
    area.text()
        .split('\n')
        .take(line)
        .map(|text| text.chars().count() + 1)
        .sum::<usize>()
        + column
}

/// Opens the editor for the selected note (or the draft) with the caret at the end.
fn ensure_editing(state: &mut State) {
    if state.editing.is_some() {
        return;
    }
    state.query = None;
    let (id, body) = match selected_note(state) {
        Some(note) => (Some(note.id), note.body.clone()),
        None => (None, String::new()),
    };
    if id.is_none() {
        state.draft = true;
    }
    state.editing = Some(Editing {
        id,
        area: TextArea::with_text(&body, MAX_NOTE_CHARS),
        dirty: false,
        last_edit: Instant::now(),
    });
}

fn edited(editing: &mut Editing) {
    editing.dirty = true;
    editing.last_edit = Instant::now();
}

/// Replaces the caret line's block prefix (heading, bullet, checkbox) with `prefix`.
fn set_line_prefix(editing: &mut Editing, prefix: &str) {
    let text = editing.area.text().to_string();
    let (line_index, column) = editing.area.caret_line_column();
    let mut lines: Vec<String> = text.split('\n').map(str::to_string).collect();
    let Some(line) = lines.get_mut(line_index) else {
        return;
    };
    let old = block_prefix(line);
    let old_chars = old.chars().count();
    let rest = line[old.len()..].to_string();
    // Toggling the same prefix again turns the line back into text.
    let new_prefix = if old == prefix { "" } else { prefix };
    *line = format!("{new_prefix}{rest}");
    let column = (column.saturating_sub(old_chars)) + new_prefix.chars().count();
    let start: usize = lines
        .iter()
        .take(line_index)
        .map(|line| line.chars().count() + 1)
        .sum();
    let joined = lines.join("\n");
    editing.area = area_at(&joined, start + column);
    edited(editing);
}

fn insert_pair(editing: &mut Editing, open: &str, close: &str) {
    editing.area.insert(open);
    editing.area.insert(close);
    for _ in 0..close.chars().count() {
        editing.area.apply(AreaKey::Left);
    }
    edited(editing);
}

// ---- Markdown-ish parsing ---------------------------------------------------------------

fn block_prefix(line: &str) -> &str {
    for prefix in ["- [ ] ", "- [x] ", "- [X] ", "## ", "# ", "- ", "* "] {
        if line.starts_with(prefix) {
            return &line[..prefix.len()];
        }
    }
    ""
}

/// `Some((done, text))` for a checklist line.
fn checkbox(line: &str) -> Option<(bool, &str)> {
    let trimmed = line.trim_start();
    if let Some(rest) = trimmed.strip_prefix("- [ ] ") {
        return Some((false, rest));
    }
    trimmed
        .strip_prefix("- [x] ")
        .or_else(|| trimmed.strip_prefix("- [X] "))
        .map(|rest| (true, rest))
}

const BOLD: u8 = 1;
const ITALIC: u8 = 2;
const UNDERLINE: u8 = 4;
const STRIKE: u8 = 8;

fn spans(line: &str) -> Vec<(String, u8)> {
    let mut out: Vec<(String, u8)> = Vec::new();
    let mut flags = 0u8;
    let mut current = String::new();
    let mut rest = line;
    while !rest.is_empty() {
        let toggle = if rest.starts_with("**") {
            Some((2, BOLD, None))
        } else if rest.starts_with("~~") {
            Some((2, STRIKE, None))
        } else if rest.starts_with("<u>") {
            Some((3, UNDERLINE, Some(true)))
        } else if rest.starts_with("</u>") {
            Some((4, UNDERLINE, Some(false)))
        } else if rest.starts_with('*') {
            Some((1, ITALIC, None))
        } else {
            None
        };
        if let Some((length, flag, set)) = toggle {
            if !current.is_empty() {
                out.push((std::mem::take(&mut current), flags));
            }
            flags = match set {
                Some(true) => flags | flag,
                Some(false) => flags & !flag,
                None => flags ^ flag,
            };
            rest = &rest[length..];
            continue;
        }
        let character = rest.chars().next().unwrap_or(' ');
        current.push(character);
        rest = &rest[character.len_utf8()..];
    }
    if !current.is_empty() {
        out.push((current, flags));
    }
    out
}

fn plain(line: &str) -> String {
    spans(line).into_iter().map(|(text, _)| text).collect()
}

/// Draws styled spans on one line from `x`, clipped at `right`.
fn draw_spans(
    painter: Painter<'_>,
    line: &str,
    base: TextStyle,
    rect: Rect,
    color: Rgba,
    struck: bool,
) {
    let fonts = FontManager::global();
    let baseline = rect.center_y() + base.size * 0.35;
    let mut x = rect.left;
    let save = painter.save();
    painter.clip_rect(rect);
    for (text, flags) in spans(line) {
        if x >= rect.right {
            break;
        }
        let bold = base.weight >= 600 || flags & BOLD != 0;
        let style = if bold {
            FontStyle::bold()
        } else {
            FontStyle::normal()
        };
        let width = fonts.measure_text_cached(&text, base.size, style);
        if flags & ITALIC != 0 {
            let inner = painter.save();
            painter.translate(Vec2::new(x, baseline));
            painter.skew(Vec2::new(-0.2, 0.0));
            fonts.draw_str(painter, &text, Point::new(0.0, 0.0), base.size, bold, color);
            painter.restore_to(inner);
        } else {
            fonts.draw_str(
                painter,
                &text,
                Point::new(x, baseline),
                base.size,
                bold,
                color,
            );
        }
        if flags & UNDERLINE != 0 {
            painter.stroke_line(
                Point::new(x, baseline + 2.5),
                Point::new(x + width, baseline + 2.5),
                1.0,
                color,
                StrokeCap::Butt,
            );
        }
        if struck || flags & STRIKE != 0 {
            let y = baseline - base.size * 0.3;
            painter.stroke_line(
                Point::new(x, y),
                Point::new(x + width, y),
                1.2,
                color,
                StrokeCap::Butt,
            );
        }
        x += width;
    }
    painter.restore_to(save);
}

// ---- Layout -----------------------------------------------------------------------------

struct Layout {
    list: Rect,
    header: Rect,
    search: Rect,
    new: Rect,
    view: Rect,
    divider_x: f32,
    pane: Rect,
    toolbar: Rect,
    content: Rect,
    footer: Rect,
}

fn layout(grid: Rect) -> Layout {
    let list_width = LIST_WIDTH.min(grid.width() * 0.4).round();
    let list = Rect::from_ltrb(grid.left, grid.top, grid.left + list_width, grid.bottom);
    let header = Rect::from_xywh(list.left, list.top + 2.0, list.width(), BAR);
    let new = Rect::from_xywh(list.right - BAR, header.top, BAR, BAR);
    let search = Rect::from_xywh(new.left - BAR - 2.0, header.top, BAR, BAR);
    let pane = Rect::from_ltrb(list.right + 25.0, grid.top + 2.0, grid.right, grid.bottom);
    let toolbar = Rect::from_xywh(pane.left, pane.top, pane.width(), BAR);
    let footer = Rect::from_ltrb(pane.left, pane.bottom - 16.0, pane.right, pane.bottom);
    Layout {
        list,
        header,
        search,
        new,
        view: Rect::from_ltrb(list.left, header.bottom + 6.0, list.right, list.bottom),
        divider_x: list.right + 12.0,
        pane,
        toolbar,
        content: Rect::from_ltrb(
            pane.left,
            toolbar.bottom + 12.0,
            pane.right,
            footer.top - 4.0,
        ),
        footer,
    }
}

#[derive(Clone, Copy)]
enum ListLine {
    Group(&'static str),
    /// Index into the shown notes.
    Note(usize),
}

fn shown_notes(state: &State) -> Vec<NoteInfo> {
    let needle = state
        .query
        .as_deref()
        .map(|query| query.trim().to_lowercase())
        .unwrap_or_default();
    state
        .notes
        .iter()
        .filter(|note| {
            needle.is_empty()
                || note.title.to_lowercase().contains(&needle)
                || note.body.to_lowercase().contains(&needle)
                || note
                    .checklist
                    .iter()
                    .any(|(text, _)| text.to_lowercase().contains(&needle))
        })
        .cloned()
        .collect()
}

fn list_lines(notes: &[NoteInfo]) -> (Vec<(ListLine, f32, f32)>, f32) {
    let today = kit::today();
    let group = |note: &NoteInfo| {
        if note.pinned {
            0
        } else if today - kit::local_day(note.updated) < 7 {
            1
        } else {
            2
        }
    };
    let mut order: Vec<usize> = (0..notes.len()).collect();
    order.sort_by_key(|index| (group(&notes[*index]), *index));
    let mut lines = Vec::new();
    let mut top = 0.0;
    let mut current = None;
    for index in order {
        let note_group = group(&notes[index]);
        if current != Some(note_group) {
            current = Some(note_group);
            let name = ["Pinned", "This week", "Earlier"][note_group];
            lines.push((ListLine::Group(name), top, GROUP_HEIGHT));
            top += GROUP_HEIGHT;
        }
        lines.push((ListLine::Note(index), top, ROW_HEIGHT));
        top += ROW_HEIGHT;
    }
    (lines, top)
}

/// Scrolls the list so the selected note is visible.
fn reveal(state: &mut State) {
    let Some(grid) = state.grid else {
        return;
    };
    let view = layout(grid).view;
    let shown = shown_notes(state);
    let (lines, content) = list_lines(&shown);
    let max = (content - view.height()).max(0.0);
    if let Some(selected) = state.selected
        && let Some((_, top, height)) = lines.iter().find(
            |(line, _, _)| matches!(line, ListLine::Note(index) if shown[*index].id == selected),
        )
    {
        let context = ROW_HEIGHT * 0.5;
        if top - context < state.scroll {
            state.scroll = top - context;
        }
        if top + height + context > state.scroll + view.height() {
            state.scroll = top + height + context - view.height();
        }
    }
    state.scroll = state.scroll.clamp(0.0, max);
}

// ---- Drawing ----------------------------------------------------------------------------

pub fn draw(ctx: &TabContext<'_>) -> bool {
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        if state.signature.is_none() {
            reload(&mut state);
        }
        state.grid = Some(ctx.grid);
        if state.selected.is_none() && !state.draft {
            state.selected = state.notes.first().map(|note| note.id);
        }
        state.targets.clear();
        state.checks.clear();
        state.row_ids.clear();
        state.segments.clear();
        let layout = layout(ctx.grid);
        draw_list(ctx, &mut state, &layout);
        ctx.painter.fill_rect(
            Rect::from_ltrb(
                layout.divider_x,
                ctx.grid.top + 4.0,
                layout.divider_x + 1.0,
                ctx.grid.bottom - 4.0,
            ),
            ctx.colors.border_subtle,
        );
        draw_pane(ctx, &mut state, &layout);
    });
    true
}

fn hovered(ctx: &TabContext<'_>, id: u16) -> bool {
    ctx.hover == Some(DashboardHit::Tab(id))
}

fn draw_list(ctx: &TabContext<'_>, state: &mut State, layout: &Layout) {
    let (painter, colors) = (ctx.painter, ctx.colors);
    if let Some(query) = &state.query {
        kit::search_field(painter, layout.header, query, "Search notes", colors);
        let close = kit::search_close(layout.header);
        kit::glyph(
            painter,
            glyphs::CLOSE,
            kit::centered(close, 14.0),
            if hovered(ctx, ID_SEARCH_CLOSE) {
                colors.text_primary
            } else {
                colors.text_secondary
            },
        );
        state.targets.push((close, ID_SEARCH_CLOSE));
    } else {
        kit::label(
            painter,
            "All notes",
            typography::BODY,
            Rect::from_ltrb(
                layout.header.left + 8.0,
                layout.header.top,
                layout.search.left - 4.0,
                layout.header.bottom,
            ),
            colors.text_secondary,
        );
        for (rect, svg, id) in [
            (layout.search, glyphs::SEARCH, ID_SEARCH),
            (layout.new, glyphs::PLUS, ID_NEW),
        ] {
            kit::icon_button(
                painter,
                rect,
                svg,
                hovered(ctx, id),
                false,
                colors.text_primary,
                colors,
            );
            state.targets.push((rect, id));
        }
    }
    let view = layout.view;
    let shown = shown_notes(state);
    if shown.is_empty() {
        let text = if state.query.is_some() {
            "No matches"
        } else {
            "No notes yet"
        };
        kit::label(
            painter,
            text,
            typography::BODY,
            Rect::from_xywh(view.left + 8.0, view.top + 6.0, view.width() - 16.0, 22.0),
            colors.text_secondary,
        );
        return;
    }
    let (lines, content) = list_lines(&shown);
    state.scroll = state.scroll.clamp(0.0, (content - view.height()).max(0.0));
    let save = painter.save();
    painter.clip_rect(view);
    let mut row_slot = 0usize;
    for (line, top, height) in lines {
        let y = view.top + top - state.scroll;
        let visible = y + height > view.top && y < view.bottom;
        match line {
            ListLine::Group(name) => {
                if visible {
                    kit::label(
                        painter,
                        name,
                        typography::LABEL,
                        Rect::from_xywh(
                            view.left + 8.0,
                            y + 2.0,
                            view.width() - 16.0,
                            height - 2.0,
                        ),
                        colors.text_secondary,
                    );
                }
            }
            ListLine::Note(index) => {
                if !visible || row_slot >= MAX_ROWS {
                    continue;
                }
                let note = &shown[index];
                let id = ID_ROW + row_slot as u16;
                row_slot += 1;
                let rect = Rect::from_xywh(view.left, y + 1.0, view.width(), height - 2.0);
                if let Some(clipped) = rect.intersect(view) {
                    state.targets.push((clipped, id));
                }
                state.row_ids.push(note.id);
                let selected = !state.draft && state.selected == Some(note.id);
                if selected {
                    painter.fill_round_rect(
                        rect,
                        Radius::uniform(10.0),
                        colors.surface_interactive,
                    );
                    painter.fill_round_rect(
                        Rect::from_xywh(rect.left, rect.top + 10.0, 3.0, rect.height() - 20.0),
                        Radius::uniform(1.5),
                        colors.text_primary,
                    );
                } else if hovered(ctx, id) {
                    painter.fill_round_rect(rect, Radius::uniform(10.0), colors.surface_raised);
                }
                let tile = Rect::from_xywh(rect.left + 10.0, rect.center_y() - 13.0, 26.0, 26.0);
                let (done, total) = note.progress();
                let svg = if total > 0 {
                    glyphs::CHECK_BOX
                } else {
                    glyphs::NOTE
                };
                let ink = note
                    .color()
                    .map_or(colors.text_secondary, |index| COLORS[index].1);
                kit::glyph(painter, svg, kit::centered(tile, 18.0), ink);
                let text_left = tile.right + 10.0;
                let title = if note.title.is_empty() {
                    "Untitled".to_string()
                } else {
                    plain(&note.title)
                };
                kit::label(
                    painter,
                    &title,
                    TextStyle {
                        size: 14.0,
                        weight: 600,
                        tabular: false,
                    },
                    Rect::from_ltrb(text_left, rect.top + 4.0, rect.right - 8.0, rect.top + 23.0),
                    colors.text_primary,
                );
                let mut meta_left = text_left;
                if note.pinned {
                    let pin = Rect::from_xywh(text_left - 1.0, rect.top + 25.0, 12.0, 12.0);
                    kit::glyph_fill(painter, glyphs::PIN_FILL, pin, colors.text_secondary);
                    kit::glyph(painter, glyphs::PIN, pin, colors.text_secondary);
                    meta_left += 15.0;
                }
                let detail = if total > 0 {
                    format!("{done}/{total} done")
                } else {
                    note.body
                        .lines()
                        .map(str::trim)
                        .filter(|line| !line.is_empty())
                        .nth(1)
                        .map(|line| plain(&line[block_prefix(line).len()..]))
                        .unwrap_or_else(|| kit::clock(note.updated))
                };
                kit::label(
                    painter,
                    &format!("{} · {detail}", kit::month_day(note.updated)),
                    typography::CAPTION,
                    Rect::from_ltrb(
                        meta_left,
                        rect.top + 23.0,
                        rect.right - 8.0,
                        rect.bottom - 3.0,
                    ),
                    colors.text_secondary,
                );
            }
        }
    }
    painter.restore_to(save);
    let max = (content - view.height()).max(0.0);
    if max > 0.0 {
        let track = view.height() - 8.0;
        let thumb = (track * view.height() / content).max(18.0);
        let top = view.top + 4.0 + (track - thumb) * (state.scroll / max);
        painter.fill_round_rect(
            Rect::from_xywh(view.right - 3.0, top, 3.0, thumb),
            Radius::uniform(1.5),
            colors.border_strong,
        );
    }
}

/// Note ids shown in list slots, captured during the last draw.
fn slot_note(state: &State, slot: usize) -> Option<u64> {
    state.row_ids.get(slot).copied()
}

enum Tool {
    Text(&'static str, u16, bool),
    Glyph(&'static str, u16),
    Gap,
}

fn draw_pane(ctx: &TabContext<'_>, state: &mut State, layout: &Layout) {
    let (painter, colors) = (ctx.painter, ctx.colors);
    let note = selected_note(state).cloned();
    let has_note = note.is_some() || state.draft;
    if !has_note {
        crate::ui::components::text::draw_text(
            painter,
            "Press + to start a note",
            typography::BODY,
            Rect::from_xywh(
                layout.pane.left,
                layout.pane.center_y() - 10.0,
                layout.pane.width(),
                20.0,
            ),
            crate::ui::components::text::Align::Center,
            colors.text_secondary,
        );
        state.targets.push((layout.pane, ID_NEW));
        return;
    }

    // Toolbar: formatting on the left, note actions on the right.
    let tools = [
        Tool::Text("B", ID_BOLD, true),
        Tool::Text("I", ID_ITALIC, false),
        Tool::Text("U", ID_UNDERLINE, false),
        Tool::Text("S", ID_STRIKE, false),
        Tool::Gap,
        Tool::Text("H1", ID_H1, true),
        Tool::Text("H2", ID_H2, true),
        Tool::Text("T", ID_TEXT, false),
        Tool::Gap,
        Tool::Glyph(glyphs::BULLETS, ID_BULLETS),
        Tool::Glyph(glyphs::CHECK_BOX, ID_CHECKLIST),
        Tool::Gap,
    ];
    let bar = layout.toolbar;
    let mut x = bar.left - 4.0;
    let caret_prefix = state.editing.as_ref().map(|editing| {
        let (line, _) = editing.area.caret_line_column();
        let text = editing.area.text();
        block_prefix(text.split('\n').nth(line).unwrap_or_default()).to_string()
    });
    for tool in &tools {
        match tool {
            Tool::Gap => {
                painter.fill_rect(
                    Rect::from_xywh(x + 4.0, bar.top + 7.0, 1.0, bar.height() - 14.0),
                    colors.border_subtle,
                );
                x += 9.0;
            }
            Tool::Text(text, id, bold) => {
                let width = if text.len() > 1 { 26.0 } else { 22.0 };
                let rect = Rect::from_xywh(x, bar.top, width, bar.height());
                let active = match *id {
                    ID_H1 => caret_prefix.as_deref() == Some("# "),
                    ID_H2 => caret_prefix.as_deref() == Some("## "),
                    _ => false,
                };
                if hovered(ctx, *id) || active {
                    painter.fill_round_rect(
                        rect.inset(1.0),
                        Radius::uniform(7.0),
                        if active {
                            colors.surface_interactive
                        } else {
                            colors.surface_overlay
                        },
                    );
                }
                let style = TextStyle {
                    size: 14.0,
                    weight: if *bold { 700 } else { 400 },
                    tabular: false,
                };
                let flags = match *id {
                    ID_ITALIC => ITALIC,
                    ID_UNDERLINE => UNDERLINE,
                    ID_STRIKE => STRIKE,
                    _ => 0,
                };
                let text_width = kit::text_width(text, style);
                let sample = match flags {
                    ITALIC => format!("*{text}*"),
                    UNDERLINE => format!("<u>{text}</u>"),
                    STRIKE => format!("~~{text}~~"),
                    _ => (*text).to_string(),
                };
                draw_spans(
                    painter,
                    &sample,
                    style,
                    Rect::from_xywh(
                        rect.center_x() - text_width / 2.0,
                        rect.top,
                        text_width + 4.0,
                        rect.height(),
                    ),
                    colors.text_primary,
                    false,
                );
                state.targets.push((rect, *id));
                x += width + 2.0;
            }
            Tool::Glyph(svg, id) => {
                let rect = Rect::from_xywh(x, bar.top, 26.0, bar.height());
                let active = match *id {
                    ID_BULLETS => caret_prefix.as_deref() == Some("- "),
                    ID_CHECKLIST => caret_prefix
                        .as_deref()
                        .is_some_and(|prefix| prefix.starts_with("- [")),
                    _ => false,
                };
                kit::icon_button(
                    painter,
                    rect,
                    *svg,
                    hovered(ctx, *id),
                    active,
                    colors.text_primary,
                    colors,
                );
                if *id == ID_BULLETS {
                    let glyph = kit::centered(rect, 16.0);
                    let scale = 16.0 / 24.0;
                    for row in [6.0, 12.0, 18.0] {
                        painter.fill_circle(
                            Point::new(glyph.left + 4.5 * scale, glyph.top + row * scale),
                            1.3,
                            colors.text_primary,
                        );
                    }
                }
                state.targets.push((rect, *id));
                x += 28.0;
            }
        }
    }
    // Colour dot with a chevron: cycles through the note colours.
    let color_rect = Rect::from_xywh(x, bar.top, 34.0, bar.height());
    if hovered(ctx, ID_COLOR) {
        painter.fill_round_rect(
            color_rect.inset(1.0),
            Radius::uniform(7.0),
            colors.surface_overlay,
        );
    }
    let dot = note
        .as_ref()
        .and_then(NoteInfo::color)
        .map_or(colors.text_disabled, |index| COLORS[index].1);
    painter.fill_circle(
        Point::new(color_rect.left + 12.0, color_rect.center_y()),
        7.0,
        dot,
    );
    kit::glyph(
        painter,
        glyphs::CHEVRON_DOWN,
        Rect::from_xywh(
            color_rect.left + 21.0,
            color_rect.center_y() - 5.0,
            10.0,
            10.0,
        ),
        colors.text_secondary,
    );
    if note.is_some() {
        state.targets.push((color_rect, ID_COLOR));
    }
    let tools_right = color_rect.right + 6.0;

    // Right side: pin, copy, delete and the edit time.
    let delete = Rect::from_xywh(bar.right - BAR, bar.top, BAR, BAR);
    let copy = Rect::from_xywh(delete.left - BAR, bar.top, BAR, BAR);
    let pin = Rect::from_xywh(copy.left - BAR, bar.top, BAR, BAR);
    if let Some(note) = &note {
        let armed = state
            .delete_armed
            .is_some_and(|at| at.elapsed() < DELETE_ARMED_FOR);
        let copied = state
            .copied_at
            .is_some_and(|at| at.elapsed() < Duration::from_millis(1500));
        kit::icon_button(
            painter,
            pin,
            glyphs::PIN,
            hovered(ctx, ID_PIN),
            note.pinned,
            colors.text_primary,
            colors,
        );
        if note.pinned {
            kit::glyph_fill(
                painter,
                glyphs::PIN_FILL,
                kit::centered(pin, 16.0),
                colors.text_primary,
            );
        }
        kit::icon_button(
            painter,
            copy,
            if copied { glyphs::CHECK } else { glyphs::COPY },
            hovered(ctx, ID_COPY),
            false,
            colors.text_primary,
            colors,
        );
        kit::icon_button(
            painter,
            delete,
            glyphs::TRASH,
            hovered(ctx, ID_DELETE),
            armed,
            if armed || hovered(ctx, ID_DELETE) {
                colors.state_error
            } else {
                colors.text_primary
            },
            colors,
        );
        state.targets.push((pin, ID_PIN));
        state.targets.push((copy, ID_COPY));
        state.targets.push((delete, ID_DELETE));
        let status = if armed {
            "Click again to delete".to_string()
        } else if copied {
            "Copied".to_string()
        } else {
            format!("Edited {}", kit::clock(note.updated))
        };
        let status_rect = Rect::from_ltrb(tools_right, bar.top, pin.left - 6.0, bar.bottom);
        if kit::text_width(&status, typography::CAPTION) <= status_rect.width() {
            kit::label_right(
                painter,
                &status,
                typography::CAPTION,
                status_rect,
                if armed {
                    colors.state_error
                } else {
                    colors.text_secondary
                },
            );
        }
    }
    painter.fill_rect(
        Rect::from_xywh(bar.left, bar.bottom + 5.0, bar.width(), 1.0),
        colors.border_subtle,
    );

    // Content and footer.
    let content = layout.content;
    state.targets.push((content, ID_BODY));
    let body_text = match &state.editing {
        Some(editing) => editing.area.text().to_string(),
        None => note
            .as_ref()
            .map(|note| note.body.clone())
            .unwrap_or_default(),
    };
    if state.editing.is_some() {
        draw_editor(ctx, state, content);
    } else if let Some(note) = &note {
        draw_note(ctx, state, note, content);
    }
    let footer_left = if state.editing.is_some() {
        "Editing · Esc to finish"
    } else {
        "Click the text to edit"
    };
    kit::label(
        painter,
        footer_left,
        typography::CAPTION,
        Rect::from_ltrb(
            layout.footer.left,
            layout.footer.top,
            layout.footer.center_x(),
            layout.footer.bottom,
        ),
        colors.text_disabled,
    );
    kit::label_right(
        painter,
        &kit::plural(kit::word_count(&plain(&body_text)), "word", "words"),
        typography::CAPTION,
        layout.footer,
        colors.text_secondary,
    );
}

const TITLE: TextStyle = TextStyle {
    size: 20.0,
    weight: 700,
    tabular: false,
};
const H1: TextStyle = TextStyle {
    size: 17.0,
    weight: 650,
    tabular: false,
};
const H2: TextStyle = TextStyle {
    size: 15.0,
    weight: 650,
    tabular: false,
};

/// The formatted, read-only note. Checkboxes are click targets.
fn draw_note(ctx: &TabContext<'_>, state: &mut State, note: &NoteInfo, area: Rect) {
    let (painter, colors) = (ctx.painter, ctx.colors);
    let save = painter.save();
    painter.clip_rect(area);
    let mut top = area.top;
    let mut seen_title = false;
    let mut check_slot = 0usize;
    let mut checkbox_row = |painter: Painter<'_>,
                            state: &mut State,
                            top: f32,
                            done: bool,
                            text: &str,
                            target: Check| {
        let row = Rect::from_xywh(area.left, top, area.width(), 26.0);
        let circle = Point::new(row.left + 9.0, row.center_y());
        let id = ID_CHECK + check_slot as u16;
        let hover = hovered(ctx, id);
        if done {
            painter.fill_circle(circle, 9.0, colors.text_primary);
            kit::glyph(
                painter,
                glyphs::CHECK,
                Rect::from_xywh(circle.x - 6.0, circle.y - 6.0, 12.0, 12.0),
                colors.surface_base,
            );
        } else {
            painter.stroke_circle(
                circle,
                8.5,
                1.5,
                if hover {
                    colors.text_primary
                } else {
                    colors.text_secondary
                },
            );
        }
        draw_spans(
            painter,
            text,
            typography::BODY,
            Rect::from_ltrb(row.left + 28.0, row.top, row.right, row.bottom),
            if done {
                colors.text_secondary
            } else {
                colors.text_primary
            },
            done,
        );
        if check_slot < MAX_CHECKS && top < area.bottom {
            let hit = Rect::from_xywh(row.left, row.top, 22.0, row.height());
            if let Some(hit) = hit.intersect(area) {
                state.targets.push((hit, id));
                state.checks.push(target);
            }
        }
        check_slot += 1;
    };
    for (index, line) in note.body.split('\n').enumerate() {
        if top >= area.bottom {
            break;
        }
        if !seen_title {
            if line.trim().is_empty() {
                continue;
            }
            seen_title = true;
            let title = plain(line.trim().trim_start_matches('#').trim());
            kit::label(
                painter,
                &title,
                TITLE,
                Rect::from_xywh(area.left, top, area.width(), 28.0),
                colors.text_primary,
            );
            top += 32.0;
            continue;
        }
        if let Some((done, text)) = checkbox(line) {
            checkbox_row(painter, state, top, done, text, Check::Body(index));
            top += 28.0;
            continue;
        }
        let prefix = block_prefix(line);
        let rest = &line[prefix.len()..];
        match prefix {
            "# " => {
                draw_spans(
                    painter,
                    rest,
                    H1,
                    Rect::from_xywh(area.left, top, area.width(), 26.0),
                    colors.text_primary,
                    false,
                );
                top += 26.0;
            }
            "## " => {
                draw_spans(
                    painter,
                    rest,
                    H2,
                    Rect::from_xywh(area.left, top, area.width(), 23.0),
                    colors.text_primary,
                    false,
                );
                top += 23.0;
            }
            "- " | "* " => {
                painter.fill_circle(
                    Point::new(area.left + 8.0, top + 11.0),
                    2.5,
                    colors.text_primary,
                );
                draw_spans(
                    painter,
                    rest,
                    typography::BODY,
                    Rect::from_xywh(area.left + 22.0, top, area.width() - 22.0, 22.0),
                    colors.text_primary,
                    false,
                );
                top += 22.0;
            }
            _ if line.trim().is_empty() => top += 10.0,
            _ if spans(line).len() <= 1 && !line.contains('*') && !line.contains('~') => {
                // Plain paragraphs wrap.
                top = kit::draw_wrapped(
                    painter,
                    line,
                    typography::BODY,
                    21.0,
                    Rect::from_ltrb(area.left, top, area.right, area.bottom),
                    colors.text_primary,
                );
            }
            _ => {
                draw_spans(
                    painter,
                    line,
                    typography::BODY,
                    Rect::from_xywh(area.left, top, area.width(), 21.0),
                    colors.text_primary,
                    false,
                );
                top += 21.0;
            }
        }
    }
    if !seen_title {
        kit::label(
            painter,
            "Untitled",
            TITLE,
            Rect::from_xywh(area.left, top, area.width(), 28.0),
            colors.text_disabled,
        );
        top += 32.0;
    }
    for (index, (text, done)) in note.checklist.iter().enumerate() {
        if top >= area.bottom {
            break;
        }
        checkbox_row(painter, state, top, *done, text, Check::Item(index));
        top += 28.0;
    }
    painter.restore_to(save);
}

/// The raw text with a caret, soft-wrapped, scrolled to keep the caret in view.
fn draw_editor(ctx: &TabContext<'_>, state: &mut State, area: Rect) {
    let (painter, colors) = (ctx.painter, ctx.colors);
    let Some(editing) = state.editing.as_ref() else {
        return;
    };
    let style = typography::BODY;
    let line_height = 21.0;
    let text = editing.area.text();
    let caret = caret_index(&editing.area);
    // Visual lines: (first char index, chars).
    let mut visual: Vec<(usize, Vec<char>)> = Vec::new();
    let mut caret_line = 0;
    let mut offset = 0;
    for logical in text.split('\n') {
        let chars: Vec<char> = logical.chars().collect();
        let ranges = kit::wrap(&chars, style, area.width() - 4.0);
        let count = ranges.len();
        for (position, (start, end)) in ranges.into_iter().enumerate() {
            let global = offset + start;
            let last = position + 1 == count;
            if caret >= global && (caret < offset + end || (last && caret <= offset + end)) {
                caret_line = visual.len();
            }
            visual.push((global, chars[start..end].to_vec()));
        }
        offset += chars.len() + 1;
    }
    let visible = ((area.height() / line_height).floor() as usize).max(1);
    let first = caret_line.saturating_sub(visible - 1);
    let save = painter.save();
    painter.clip_rect(area);
    let mut segments = Vec::new();
    for (row, (start, chars)) in visual.iter().enumerate().skip(first).take(visible) {
        let top = area.top + (row - first) as f32 * line_height;
        let rect = Rect::from_xywh(area.left, top, area.width(), line_height);
        let line: String = chars.iter().collect();
        kit::label(painter, &line, style, rect, colors.text_primary);
        if row == caret_line && kit::caret_visible() {
            let before: String = chars.iter().take(caret - start).collect();
            let x = area.left + kit::text_width(&before, style);
            painter.stroke_line(
                Point::new(x, top + 3.0),
                Point::new(x, top + line_height - 3.0),
                1.2,
                colors.accent_primary,
                StrokeCap::Butt,
            );
        }
        segments.push((rect, *start, chars.clone()));
    }
    painter.restore_to(save);
    state.segments = segments;
}

/// Char index under `point` in the editor, from the last drawn segments.
fn char_at(state: &State, point: Point) -> Option<usize> {
    let (rect, start, chars) = state
        .segments
        .iter()
        .find(|(rect, _, _)| point.y >= rect.top && point.y < rect.bottom)
        .or_else(|| {
            state
                .segments
                .last()
                .filter(|(rect, _, _)| point.y >= rect.bottom)
        })?;
    let style = typography::BODY;
    let mut x = rect.left;
    for (index, character) in chars.iter().enumerate() {
        let width = kit::text_width(&character.to_string(), style);
        if point.x < x + width / 2.0 {
            return Some(start + index);
        }
        x += width;
    }
    // Past the end: before a wrapped line's trailing space, else at the end.
    let end = chars.len() - usize::from(chars.last() == Some(&' '));
    Some(start + end)
}

// ---- Interaction ------------------------------------------------------------------------

pub fn custom_layout() -> bool {
    true
}

pub fn hit(grid: Rect, point: Point) -> Option<DashboardHit> {
    if !grid.contains(point) {
        return None;
    }
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        state.last_point = Some(point);
        let id = state
            .targets
            .iter()
            .rev()
            .find(|(rect, _)| rect.contains(point))
            .map_or(ID_INERT, |(_, id)| *id);
        Some(DashboardHit::Tab(id))
    })
}

fn select(state: &mut State, id: u64) {
    if state.selected == Some(id) && state.editing.is_none() && !state.draft {
        return;
    }
    stop_editing(state);
    state.selected = Some(id);
    state.delete_armed = None;
    reveal(state);
}

pub fn apply(hit: DashboardHit) -> bool {
    let id = match hit {
        DashboardHit::Tab(id) if (500..600).contains(&id) => id,
        // The legacy page's rows sit under this tab; never let them act.
        DashboardHit::NoteRow(_) => return true,
        _ => return false,
    };
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        let state = &mut *state;
        if id != ID_DELETE {
            state.delete_armed = None;
        }
        match id {
            ID_NEW => {
                stop_editing(state);
                state.query = None;
                state.draft = true;
                ensure_editing(state);
            }
            ID_SEARCH => {
                stop_editing(state);
                state.query = Some(String::new());
                state.scroll = 0.0;
            }
            ID_SEARCH_CLOSE => {
                state.query = None;
                reveal(state);
            }
            ID_BOLD | ID_ITALIC | ID_UNDERLINE | ID_STRIKE => {
                ensure_editing(state);
                if let Some(editing) = state.editing.as_mut() {
                    let (open, close) = match id {
                        ID_BOLD => ("**", "**"),
                        ID_ITALIC => ("*", "*"),
                        ID_UNDERLINE => ("<u>", "</u>"),
                        _ => ("~~", "~~"),
                    };
                    insert_pair(editing, open, close);
                }
            }
            ID_H1 | ID_H2 | ID_TEXT | ID_BULLETS | ID_CHECKLIST => {
                ensure_editing(state);
                if let Some(editing) = state.editing.as_mut() {
                    let prefix = match id {
                        ID_H1 => "# ",
                        ID_H2 => "## ",
                        ID_BULLETS => "- ",
                        ID_CHECKLIST => "- [ ] ",
                        _ => "",
                    };
                    if prefix.is_empty() {
                        // "T": back to plain text, whatever the line was.
                        let (line, _) = editing.area.caret_line_column();
                        let current = editing
                            .area
                            .text()
                            .split('\n')
                            .nth(line)
                            .map(|text| block_prefix(text).to_string())
                            .unwrap_or_default();
                        if !current.is_empty() {
                            set_line_prefix(editing, &current);
                        }
                    } else {
                        set_line_prefix(editing, prefix);
                    }
                }
            }
            ID_COLOR => {
                persist(state);
                if let Some(note) = selected_note(state).cloned() {
                    let next = match note.color() {
                        None => Some(0),
                        Some(index) if index + 1 < COLORS.len() => Some(index + 1),
                        Some(_) => None,
                    };
                    with_store(state, |store| {
                        for (name, _) in COLORS {
                            let _ = store.remove_tag(note.id, &format!("color-{name}"));
                        }
                        if let Some(index) = next {
                            let _ = store.add_tag(note.id, &format!("color-{}", COLORS[index].0));
                        }
                        true
                    });
                }
            }
            ID_PIN => {
                persist(state);
                if let Some(id) = selected_note(state).map(|note| note.id) {
                    with_store(state, |store| store.toggle_pin(id).is_ok());
                    reveal(state);
                }
            }
            ID_COPY => {
                let body = match &state.editing {
                    Some(editing) => Some(editing.area.text().to_string()),
                    None => selected_note(state).map(|note| note.body.clone()),
                };
                if let Some(body) = body {
                    match winisland_platform_windows::set_clipboard_text(&body) {
                        Ok(()) => state.copied_at = Some(Instant::now()),
                        Err(error) => log::warn!("Cannot copy note: {error}"),
                    }
                }
            }
            ID_DELETE => {
                let armed = state
                    .delete_armed
                    .is_some_and(|at| at.elapsed() < DELETE_ARMED_FOR);
                if !armed {
                    state.delete_armed = Some(Instant::now());
                } else if let Some(id) = selected_note(state).map(|note| note.id) {
                    state.delete_armed = None;
                    state.editing = None;
                    let position = state.notes.iter().position(|note| note.id == id);
                    with_store(state, |store| store.delete(id).is_ok());
                    state.selected = position
                        .and_then(|position| {
                            state.notes.get(position).or_else(|| state.notes.last())
                        })
                        .map(|note| note.id);
                    reveal(state);
                }
            }
            ID_BODY => {
                let was_editing = state.editing.is_some();
                ensure_editing(state);
                if was_editing
                    && let Some(point) = state.last_point
                    && let Some(index) = char_at(state, point)
                    && let Some(editing) = state.editing.as_mut()
                {
                    let text = editing.area.text().to_string();
                    editing.area = area_at(&text, index);
                }
            }
            _ if (ID_ROW..ID_CHECK).contains(&id) => {
                if let Some(note) = slot_note(state, usize::from(id - ID_ROW)) {
                    select(state, note);
                }
            }
            _ if (ID_CHECK..ID_BODY).contains(&id) => {
                let slot = usize::from(id - ID_CHECK);
                let check = state.checks.get(slot).copied();
                if let (Some(check), Some(note)) = (check, selected_note(state).cloned()) {
                    let now = SystemTime::now();
                    match check {
                        Check::Item(index) => {
                            with_store(state, |store| {
                                store.toggle_item(note.id, index, now).is_ok()
                            });
                        }
                        Check::Body(line) => {
                            let body: Vec<String> = note
                                .body
                                .split('\n')
                                .enumerate()
                                .map(|(index, text)| {
                                    if index != line {
                                        return text.to_string();
                                    }
                                    if let Some(rest) = text.strip_prefix("- [ ] ") {
                                        format!("- [x] {rest}")
                                    } else if let Some(rest) = text
                                        .strip_prefix("- [x] ")
                                        .or_else(|| text.strip_prefix("- [X] "))
                                    {
                                        format!("- [ ] {rest}")
                                    } else {
                                        text.to_string()
                                    }
                                })
                                .collect();
                            let body = body.join("\n");
                            with_store(state, |store| {
                                store.update_body(note.id, &body, now).unwrap_or(false)
                            });
                        }
                    }
                }
            }
            _ => {}
        }
    });
    true
}

pub fn wants_keyboard() -> bool {
    STATE.with(|state| {
        let state = state.borrow();
        state.editing.is_some() || state.query.is_some()
    })
}

pub fn handle_key(key: &Key) -> bool {
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        let state = &mut *state;
        if let Some(editing) = state.editing.as_mut() {
            if *key == Key::Escape {
                stop_editing(state);
                return true;
            }
            let area_key = match key {
                Key::Enter => Some(AreaKey::Enter),
                Key::Backspace => Some(AreaKey::Backspace),
                Key::Delete => Some(AreaKey::Delete),
                Key::ArrowLeft => Some(AreaKey::Left),
                Key::ArrowRight => Some(AreaKey::Right),
                Key::ArrowUp => Some(AreaKey::Up),
                Key::ArrowDown => Some(AreaKey::Down),
                Key::Home => Some(AreaKey::Home),
                Key::End => Some(AreaKey::End),
                Key::Tab => {
                    if editing.area.insert("    ") {
                        edited(editing);
                    }
                    None
                }
                Key::Character(text) => {
                    if editing.area.insert(text) {
                        edited(editing);
                    }
                    None
                }
                _ => None,
            };
            if let Some(area_key) = area_key {
                // Enter on a list line continues the list.
                let continue_prefix = if area_key == AreaKey::Enter {
                    let (line, _) = editing.area.caret_line_column();
                    let current = editing
                        .area
                        .text()
                        .split('\n')
                        .nth(line)
                        .unwrap_or_default();
                    let prefix = block_prefix(current);
                    match prefix {
                        "- " | "* " if current.len() > prefix.len() => Some(prefix.to_string()),
                        _ if prefix.starts_with("- [") && current.len() > prefix.len() => {
                            Some("- [ ] ".to_string())
                        }
                        _ => None,
                    }
                } else {
                    None
                };
                let changed = editing.area.apply(area_key);
                if let Some(prefix) = continue_prefix {
                    editing.area.insert(&prefix);
                }
                if changed
                    && matches!(
                        area_key,
                        AreaKey::Enter | AreaKey::Backspace | AreaKey::Delete
                    )
                {
                    edited(editing);
                }
            }
            return true;
        }
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
            }
            Key::Character(text) => {
                if query.chars().count() < 80 {
                    query.extend(text.chars().filter(|character| !character.is_control()));
                }
                state.scroll = 0.0;
            }
            Key::Enter => {
                let first = shown_notes(state).first().map(|note| note.id);
                state.query = None;
                if let Some(id) = first {
                    select(state, id);
                }
            }
            _ => {}
        }
        true
    })
}

/// Autosaves edits and picks up notes changed elsewhere (quick notes, the legacy editor).
pub fn poll(_now: Instant) {
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        let state = &mut *state;
        if state
            .editing
            .as_ref()
            .is_some_and(|editing| editing.dirty && editing.last_edit.elapsed() >= AUTOSAVE_AFTER)
        {
            persist(state);
        }
        if state.signature.is_some() {
            let changed = crate::ui::dashboard::NOTE_ROWS
                .with(|rows| state.signature.as_ref() != Some(&*rows.borrow()));
            if changed {
                reload(state);
            }
        }
    });
}
