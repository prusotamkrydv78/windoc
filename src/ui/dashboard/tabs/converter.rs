//! File converter tab: convert and shrink images to JPG, PNG, PDF or ICO.
//!
//! Left: the files to convert, mirrored from the active shelf's images until the user
//! edits the list, plus any chosen with "Choose files". Right: format, quality or target
//! size, resize and destination, with a live size estimate. Conversions run on a
//! background thread; `poll` collects results. Originals are never touched: outputs are
//! saved beside them (or in Downloads) and take a " (1)" suffix when the name is taken.

mod engine;

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::time::Instant;

use winisland_platform::Key;
use winisland_render::tokens::{TextStyle, typography};
use winisland_render::{Angle, Point, Radius, Rect, StrokeCap};

use super::TabContext;
use super::shelf::kit::{self, format_bytes, style, text, text_center, text_right, text_width};
use super::shelf::thumbs;
use crate::ui::dashboard::DashboardHit;
use engine::{Format, Resize, Settings};

const ID_ADD: u16 = 700;
const ID_CLEAR: u16 = 701;
const ID_CHOOSE: u16 = 702;
const ID_FROM_SHELF: u16 = 703;
const ID_FORMAT: u16 = 704;
const ID_MODE_QUALITY: u16 = 708;
const ID_MODE_TARGET: u16 = 709;
const ID_RESIZE: u16 = 710;
const ID_SAVE_TO: u16 = 711;
const ID_CONVERT: u16 = 712;
const ID_SHOW_FOLDER: u16 = 713;
const ID_MORE: u16 = 714;
const ID_ROW_REMOVE: u16 = 720;
const ID_ROW: u16 = 740;
const MAX_ROWS: u16 = 20;
const ID_SLIDER: u16 = 760;
/// Slider positions: 10% to 100% in steps of 5.
const SLIDER_STEPS: u16 = 18;

const LIST_W: f32 = 268.0;
const ROW_H: f32 = 38.0;
const ROW_STEP: f32 = 40.0;
const NAME: TextStyle = style(14.0, 500);

#[derive(Clone)]
enum Status {
    Ready,
    Working,
    Done { output: PathBuf, bytes: u64 },
    Failed(String),
}

struct FileItem {
    path: PathBuf,
    name: String,
    bytes: u64,
    /// The shelf entry it mirrors, if it came from the shelf.
    shelf_id: Option<u64>,
    status: Status,
}

impl FileItem {
    fn new(path: PathBuf, shelf_id: Option<u64>) -> Self {
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let bytes = std::fs::metadata(&path).map_or(0, |meta| meta.len());
        Self {
            path,
            name,
            bytes,
            shelf_id,
            status: Status::Ready,
        }
    }

    fn type_label(&self) -> String {
        match kit::extension(&self.path).as_str() {
            "jpeg" => "JPG".to_string(),
            "tiff" => "TIF".to_string(),
            other => other.to_ascii_uppercase(),
        }
    }
}

type JobResult = (usize, Result<(PathBuf, u64), String>);

struct Job {
    results: Receiver<JobResult>,
    total: usize,
    finished: usize,
    started: Instant,
}

struct Summary {
    saved: usize,
    failed: usize,
    bytes: u64,
}

struct State {
    files: Vec<FileItem>,
    /// Once the user edits the list, stop mirroring the shelf (new shelf files still join).
    manual: bool,
    seen: HashSet<u64>,
    shelf_revision: Option<u64>,
    shelf_name: String,
    format: Format,
    quality: u8,
    target: u8,
    use_target: bool,
    resize: Resize,
    to_downloads: bool,
    job: Option<Job>,
    summary: Option<Summary>,
    last_output: Option<PathBuf>,
    picker: Option<Receiver<Vec<PathBuf>>>,
    /// First row shown when the list is longer than the column.
    scroll: usize,
}

impl Default for State {
    fn default() -> Self {
        Self {
            files: Vec::new(),
            manual: false,
            seen: HashSet::new(),
            shelf_revision: None,
            shelf_name: String::new(),
            format: Format::Jpg,
            quality: 80,
            target: 60,
            use_target: false,
            resize: Resize::Original,
            to_downloads: false,
            job: None,
            summary: None,
            last_output: None,
            picker: None,
            scroll: 0,
        }
    }
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
    static LAST_GRID: Cell<Rect> = const { Cell::new(Rect::from_xywh(0.0, 0.0, 696.0, 222.0)) };
}

fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> R {
    STATE.with(|cell| f(&mut cell.borrow_mut()))
}

impl State {
    fn running(&self) -> bool {
        self.job.is_some()
    }

    /// Any change to files or options leaves the "done" state.
    fn touched(&mut self) {
        if self.running() {
            return;
        }
        self.summary = None;
        for file in &mut self.files {
            file.status = Status::Ready;
        }
    }

    fn settings(&self) -> Settings {
        Settings {
            format: self.format,
            quality: self.quality,
            target_percent: (self.use_target && self.format.lossy()).then_some(self.target),
            resize: self.resize,
            folder: if self.to_downloads {
                dirs::download_dir()
            } else {
                None
            },
        }
    }

    fn total_bytes(&self) -> u64 {
        self.files.iter().map(|file| file.bytes).sum()
    }

    fn estimate(&self) -> u64 {
        let settings = self.settings();
        self.files
            .iter()
            .map(|file| {
                let same = kit::extension(&file.path)
                    .replace("jpeg", "jpg")
                    .eq(&settings.format.label().to_ascii_lowercase());
                engine::estimate(&settings, file.bytes, thumbs::dimensions(&file.path), same)
            })
            .sum()
    }
}

/// Image entries of the active shelf: `(entry id, path)`.
fn shelf_images() -> (Option<u64>, String, Vec<(u64, PathBuf)>) {
    crate::ui::dashboard::SHELF.with(|cell| {
        let view = cell.borrow();
        (
            view.revision,
            view.name.clone(),
            view.entries
                .iter()
                .filter(|(_, _, path, exists)| *exists && kit::is_image(path))
                .map(|(id, _, path, _)| (*id, path.clone()))
                .collect(),
        )
    })
}

/// Mirrors the shelf's images into the list (see [`State::manual`]).
fn sync_shelf(state: &mut State, force: bool) {
    if state.running() {
        return;
    }
    let (revision, name, images) = shelf_images();
    if revision.is_none() || (!force && revision == state.shelf_revision) {
        return;
    }
    state.shelf_revision = revision;
    let switched = state.shelf_name != name;
    state.shelf_name = name;
    let before: Vec<PathBuf> = state.files.iter().map(|file| file.path.clone()).collect();
    if !state.manual {
        let mut old: Vec<FileItem> = std::mem::take(&mut state.files);
        let mut files = Vec::new();
        for (id, path) in &images {
            match old.iter().position(|file| file.path == *path) {
                Some(index) => {
                    let mut file = old.remove(index);
                    file.shelf_id = Some(*id);
                    files.push(file);
                }
                None => files.push(FileItem::new(path.clone(), Some(*id))),
            }
        }
        files.extend(old.into_iter().filter(|file| file.shelf_id.is_none()));
        state.files = files;
    } else if !switched {
        for (id, path) in &images {
            if !state.seen.contains(id) && !state.files.iter().any(|file| file.path == *path) {
                state.files.push(FileItem::new(path.clone(), Some(*id)));
            }
        }
    }
    state.seen.extend(images.iter().map(|(id, _)| *id));
    let after: Vec<&PathBuf> = state.files.iter().map(|file| &file.path).collect();
    if before.iter().collect::<Vec<_>>() != after {
        state.touched();
    }
}

/// Fills the list from the active shelf's images (the Shelf tab's "Convert images").
pub(super) fn import_shelf() {
    with_state(|state| {
        if state.running() {
            return;
        }
        state.manual = false;
        state.scroll = 0;
        sync_shelf(state, true);
    });
}

fn add_paths(state: &mut State, paths: Vec<PathBuf>) {
    let mut added = false;
    for path in paths {
        if kit::is_image(&path) && !state.files.iter().any(|file| file.path == path) {
            state.files.push(FileItem::new(path, None));
            added = true;
        }
    }
    if added {
        state.manual = true;
        state.touched();
    }
}

fn open_picker(state: &mut State) {
    if state.picker.is_some() || state.running() {
        return;
    }
    let (sender, receiver) = channel();
    let spawned = std::thread::Builder::new()
        .name("converter-picker".into())
        .spawn(move || {
            let files = rfd::FileDialog::new()
                .set_title("Choose images to convert")
                .add_filter(
                    "Images",
                    &[
                        "jpg", "jpeg", "png", "bmp", "gif", "webp", "tif", "tiff", "ico", "tga",
                    ],
                )
                .pick_files()
                .unwrap_or_default();
            let _ = sender.send(files);
        });
    if spawned.is_ok() {
        state.picker = Some(receiver);
    }
}

fn start(state: &mut State) {
    if state.running() || state.files.is_empty() {
        return;
    }
    state.touched();
    let settings = state.settings();
    let work: Vec<(usize, PathBuf)> = state
        .files
        .iter()
        .enumerate()
        .map(|(index, file)| (index, file.path.clone()))
        .collect();
    let (sender, receiver) = channel::<JobResult>();
    let spawned = std::thread::Builder::new()
        .name("converter".into())
        .spawn(move || {
            for (index, path) in work {
                let result = engine::convert(&path, &settings);
                if sender.send((index, result)).is_err() {
                    break;
                }
            }
        });
    if spawned.is_err() {
        return;
    }
    for file in &mut state.files {
        file.status = Status::Working;
    }
    state.job = Some(Job {
        results: receiver,
        total: state.files.len(),
        finished: 0,
        started: Instant::now(),
    });
}

/// Collects finished conversions and picked files.
fn collect(state: &mut State) {
    if let Some(picker) = &state.picker {
        match picker.try_recv() {
            Ok(paths) => {
                state.picker = None;
                add_paths(state, paths);
            }
            Err(TryRecvError::Disconnected) => state.picker = None,
            Err(TryRecvError::Empty) => {}
        }
    }
    let Some(job) = state.job.as_mut() else {
        return;
    };
    let mut stopped = false;
    loop {
        match job.results.try_recv() {
            Ok((index, result)) => {
                job.finished += 1;
                if let Some(file) = state.files.get_mut(index) {
                    file.status = match result {
                        Ok((output, bytes)) => {
                            state.last_output = Some(output.clone());
                            Status::Done { output, bytes }
                        }
                        Err(error) => {
                            log::warn!("Image conversion failed: {error}");
                            Status::Failed(error)
                        }
                    };
                }
            }
            Err(TryRecvError::Empty) => break,
            Err(TryRecvError::Disconnected) => {
                stopped = true;
                break;
            }
        }
    }
    if job.finished >= job.total || stopped {
        state.job = None;
        let mut summary = Summary {
            saved: 0,
            failed: 0,
            bytes: 0,
        };
        for file in &mut state.files {
            match &file.status {
                Status::Done { bytes, .. } => {
                    summary.saved += 1;
                    summary.bytes += bytes;
                }
                Status::Working => {
                    file.status = Status::Failed("Stopped".into());
                    summary.failed += 1;
                }
                _ => summary.failed += 1,
            }
        }
        state.summary = Some(summary);
    }
}

struct Layout {
    list: Rect,
    list_header: Rect,
    add: Rect,
    clear: Rect,
    rows_top: f32,
    divider_x: f32,
    panel: Rect,
    segments: Rect,
    size_row: Rect,
    slider: Rect,
    note: Rect,
    resize_row: Rect,
    save_row: Rect,
    footer: Rect,
}

fn layout(grid: Rect) -> Layout {
    let list = Rect::from_xywh(grid.left, grid.top, LIST_W, grid.height());
    let list_header = Rect::from_xywh(list.left, list.top, list.width(), 24.0);
    let clear = kit::square(Point::new(list.right - 12.0, list_header.center_y()), 24.0);
    let add = kit::square(Point::new(clear.left - 16.0, list_header.center_y()), 24.0);
    let divider_x = list.right + 12.0;
    let panel = Rect::from_ltrb(divider_x + 13.0, grid.top, grid.right, grid.bottom);
    let top = panel.top;
    let row =
        |offset: f32, height: f32| Rect::from_xywh(panel.left, top + offset, panel.width(), height);
    let footer_h = 36.0;
    Layout {
        list,
        list_header,
        add,
        clear,
        rows_top: list.top + 30.0,
        divider_x,
        segments: row(20.0, 30.0),
        size_row: row(56.0, 18.0),
        slider: row(78.0, 22.0),
        note: row(101.0, 16.0),
        resize_row: row(122.0, 26.0),
        save_row: row(152.0, 26.0),
        footer: Rect::from_ltrb(
            panel.left,
            (panel.bottom - footer_h).max(top + 182.0),
            panel.right,
            panel.bottom.max(top + 182.0 + footer_h),
        ),
        panel,
    }
}

fn segment(layout: &Layout, index: usize) -> Rect {
    let inner = layout.segments.inset(3.0);
    let width = inner.width() / Format::ALL.len() as f32;
    Rect::from_xywh(
        inner.left + index as f32 * width,
        inner.top,
        width,
        inner.height(),
    )
}

/// The slider track (without the percentage label).
fn track(layout: &Layout) -> Rect {
    Rect::from_ltrb(
        layout.slider.left + 2.0,
        layout.slider.center_y() - 2.0,
        layout.slider.right - 60.0,
        layout.slider.center_y() + 2.0,
    )
}

/// `(quality rect, target rect)` in the SIZE row.
fn mode_buttons(layout: &Layout) -> (Rect, Rect) {
    let target_w = text_width("Target", typography::BODY) + 8.0;
    let quality_w = text_width("Quality", typography::BODY) + 8.0;
    let target = Rect::from_xywh(
        layout.size_row.right - target_w,
        layout.size_row.top - 3.0,
        target_w,
        layout.size_row.height() + 6.0,
    );
    let quality = Rect::from_xywh(
        target.left - 14.0 - quality_w,
        target.top,
        quality_w,
        target.height(),
    );
    (quality, target)
}

fn pill(layout: &Layout, label: &str) -> Rect {
    let width = text_width(label, style(14.0, 600)) + 36.0;
    Rect::from_xywh(
        layout.footer.right - width,
        layout.footer.center_y() - 16.0,
        width,
        32.0,
    )
}

fn pill_label(state: &State) -> (u16, String) {
    if let Some(job) = &state.job {
        return (
            ID_CONVERT,
            format!(
                "Converting {}/{}",
                (job.finished + 1).min(job.total),
                job.total
            ),
        );
    }
    if state
        .summary
        .as_ref()
        .is_some_and(|summary| summary.saved > 0)
    {
        return (ID_SHOW_FOLDER, "Show in folder".to_string());
    }
    let count = state.files.len();
    (
        ID_CONVERT,
        match count {
            0 => "Convert".to_string(),
            1 => "Convert 1 file".to_string(),
            _ => format!("Convert {count} files"),
        },
    )
}

/// `(rows that fit, whether the last one is a "more" row)`.
fn row_window(layout: &Layout, count: usize) -> (usize, bool) {
    let fit = (((layout.list.bottom - layout.rows_top + (ROW_STEP - ROW_H)) / ROW_STEP).floor()
        as usize)
        .clamp(1, MAX_ROWS as usize);
    if count > fit {
        (fit - 1, true)
    } else {
        (count, false)
    }
}

fn row_rect(layout: &Layout, slot: usize) -> Rect {
    Rect::from_xywh(
        layout.list.left,
        layout.rows_top + slot as f32 * ROW_STEP,
        layout.list.width(),
        ROW_H,
    )
}

fn row_remove(row: Rect) -> Rect {
    kit::square(Point::new(row.right - 14.0, row.center_y()), 24.0)
}

/// The area below the rows that accepts a click to choose more files.
fn drop_area(layout: &Layout, state: &State) -> Option<Rect> {
    if state.files.is_empty() {
        return Some(Rect::from_ltrb(
            layout.list.left,
            layout.rows_top - 2.0,
            layout.list.right,
            layout.list.bottom,
        ));
    }
    let (shown, more) = row_window(layout, state.files.len());
    if more {
        return None;
    }
    let top = layout.rows_top + shown as f32 * ROW_STEP + 2.0;
    (layout.list.bottom - top >= 34.0)
        .then(|| Rect::from_ltrb(layout.list.left, top, layout.list.right, layout.list.bottom))
}

/// "Add from shelf" and "Choose files" in the empty list.
fn empty_buttons(area: Rect) -> (Rect, Rect) {
    let y = area.center_y() + 22.0;
    let shelf_w = text_width("Add from shelf", typography::LABEL) + 24.0;
    let choose_w = text_width("Choose files", typography::LABEL) + 24.0;
    let total = shelf_w + 8.0 + choose_w;
    let left = area.center_x() - total / 2.0;
    (
        Rect::from_xywh(left, y, shelf_w, 28.0),
        Rect::from_xywh(left + shelf_w + 8.0, y, choose_w, 28.0),
    )
}

fn hovered(ctx: &TabContext<'_>, id: u16) -> bool {
    ctx.hover == Some(DashboardHit::Tab(id))
}

/// Draws the tab. Return `false` to fall back to the legacy page.
pub fn draw(ctx: &TabContext<'_>) -> bool {
    LAST_GRID.with(|cell| cell.set(ctx.grid));
    with_state(|state| {
        sync_shelf(state, false);
        let layout = layout(ctx.grid);
        draw_list(ctx, state, &layout);
        ctx.painter.fill_rect(
            Rect::from_xywh(
                layout.divider_x,
                ctx.grid.top + 4.0,
                1.0,
                ctx.grid.height() - 8.0,
            ),
            ctx.colors.border_subtle,
        );
        draw_panel(ctx, state, &layout);
    });
    true
}

fn draw_list(ctx: &TabContext<'_>, state: &State, layout: &Layout) {
    let painter = ctx.painter;
    let colors = ctx.colors;
    let count = state.files.len();
    let header = match count {
        0 => "NO FILES".to_string(),
        1 => format!("1 FILE · {}", format_bytes(state.total_bytes())),
        _ => format!("{count} FILES · {}", format_bytes(state.total_bytes())),
    };
    text(
        painter,
        &header,
        typography::CAPTION,
        Rect::from_ltrb(
            layout.list_header.left + 2.0,
            layout.list_header.top,
            layout.add.left - 8.0,
            layout.list_header.bottom,
        ),
        colors.text_secondary,
    );
    let editable = !state.running();
    kit::icon_button(
        painter,
        layout.add,
        kit::PLUS,
        hovered(ctx, ID_ADD),
        editable,
        colors,
    );
    kit::icon_button(
        painter,
        layout.clear,
        kit::TRASH,
        hovered(ctx, ID_CLEAR),
        editable && count > 0,
        colors,
    );

    let (shown, more) = row_window(layout, count);
    let first = state.scroll.min(count.saturating_sub(shown));
    for slot in 0..shown {
        let Some(file) = state.files.get(first + slot) else {
            break;
        };
        let row = row_rect(layout, slot);
        let remove = row_remove(row);
        let row_hover =
            hovered(ctx, ID_ROW + slot as u16) || hovered(ctx, ID_ROW_REMOVE + slot as u16);
        if row_hover {
            painter.fill_round_rect(row, Radius::uniform(10.0), colors.surface_raised);
        }
        let thumb = kit::square(Point::new(row.left + 20.0, row.center_y()), 32.0);
        kit::file_icon(painter, thumb, &file.path, false, false, colors);
        let text_right_edge = remove.left - 6.0;
        text(
            painter,
            &file.name,
            NAME,
            Rect::from_ltrb(
                row.left + 44.0,
                row.top + 2.0,
                text_right_edge,
                row.top + 20.0,
            ),
            colors.text_primary,
        );
        let (detail, color) = match &file.status {
            Status::Ready => (
                format!("{} · {}", file.type_label(), format_bytes(file.bytes)),
                colors.text_secondary,
            ),
            Status::Working => ("Converting…".to_string(), colors.text_secondary),
            Status::Done { output, bytes } => (
                format!(
                    "Saved {} · {}",
                    output
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                    format_bytes(*bytes)
                ),
                colors.state_success,
            ),
            Status::Failed(error) => (format!("Failed · {error}"), colors.state_error),
        };
        text(
            painter,
            &detail,
            typography::CAPTION,
            Rect::from_ltrb(
                row.left + 44.0,
                row.top + 20.0,
                text_right_edge,
                row.bottom - 2.0,
            ),
            color,
        );
        match (&file.status, &state.job) {
            (Status::Working, Some(job)) => {
                let spin = job.started.elapsed().as_secs_f32() * 360.0;
                painter.stroke_arc(
                    remove.inset(6.0),
                    Angle::from_degrees(spin % 360.0),
                    Angle::from_degrees(250.0),
                    1.8,
                    colors.text_secondary,
                    StrokeCap::Round,
                );
            }
            (Status::Done { .. }, _) => {
                kit::glyph(painter, kit::CHECK, remove.inset(5.0), colors.state_success);
            }
            _ => {
                let over = hovered(ctx, ID_ROW_REMOVE + slot as u16);
                kit::icon_button(painter, remove, kit::CLOSE, over, editable, colors);
            }
        }
    }
    if more {
        let row = row_rect(layout, shown);
        let hover = hovered(ctx, ID_MORE);
        if hover {
            painter.fill_round_rect(row, Radius::uniform(10.0), colors.surface_raised);
        }
        let after = count.saturating_sub(first + shown);
        let label = if after > 0 {
            format!("+{after} more · show next")
        } else {
            "Back to the first files".to_string()
        };
        text_center(
            painter,
            &label,
            typography::LABEL,
            row,
            if hover {
                colors.text_primary
            } else {
                colors.text_secondary
            },
        );
    }
    let Some(area) = drop_area(layout, state) else {
        return;
    };
    let hover = hovered(ctx, ID_CHOOSE);
    if count == 0 {
        kit::dashed_round_rect(painter, area.inset(1.0), 12.0, colors.border_strong);
        kit::glyph(
            painter,
            kit::UPLOAD,
            kit::square(Point::new(area.center_x(), area.center_y() - 34.0), 24.0),
            colors.text_secondary,
        );
        text_center(
            painter,
            "Add images to convert",
            typography::SECTION_TITLE,
            Rect::from_xywh(area.left, area.center_y() - 16.0, area.width(), 20.0),
            colors.text_primary,
        );
        text_center(
            painter,
            "From this shelf, or pick files on this PC",
            typography::CAPTION,
            Rect::from_xywh(area.left, area.center_y() + 4.0, area.width(), 14.0),
            colors.text_secondary,
        );
        let (from_shelf, choose) = empty_buttons(area);
        for (rect, label, id) in [
            (from_shelf, "Add from shelf", ID_FROM_SHELF),
            (choose, "Choose files", ID_CHOOSE),
        ] {
            let over = hovered(ctx, id);
            painter.fill_round_rect(
                rect,
                Radius::uniform(14.0),
                if over {
                    colors.surface_interactive
                } else {
                    colors.surface_overlay
                },
            );
            text_center(painter, label, typography::LABEL, rect, colors.text_primary);
        }
        return;
    }
    if hover && editable {
        painter.fill_round_rect(
            area.inset(2.0),
            Radius::uniform(10.0),
            colors.surface_raised,
        );
    }
    let color = if hover && editable {
        colors.text_primary
    } else {
        colors.text_secondary
    };
    let center = area.center_y().min(area.top + 34.0);
    kit::glyph(
        painter,
        kit::UPLOAD,
        kit::square(Point::new(area.center_x(), center - 10.0), 18.0),
        color,
    );
    text_center(
        painter,
        if state.picker.is_some() {
            "Choosing files…"
        } else {
            "Drop more files here · or click to choose"
        },
        typography::LABEL,
        Rect::from_xywh(area.left, center + 4.0, area.width(), 16.0),
        color,
    );
}

fn caption(ctx: &TabContext<'_>, label: &str, rect: Rect) {
    text(
        ctx.painter,
        label,
        typography::CAPTION,
        rect,
        ctx.colors.text_secondary,
    );
}

/// A label on the left and a value with a chevron on the right; hover lights the row.
fn picker_row(ctx: &TabContext<'_>, rect: Rect, label: &str, value: &str, id: u16, enabled: bool) {
    let colors = ctx.colors;
    let hover = hovered(ctx, id) && enabled;
    if hover {
        ctx.painter.fill_round_rect(
            rect.inset(-2.0),
            Radius::uniform(8.0),
            colors.surface_raised,
        );
    }
    caption(
        ctx,
        label,
        Rect::from_xywh(rect.left + 2.0, rect.top, 120.0, rect.height()),
    );
    let color = if !enabled {
        colors.text_disabled
    } else if hover {
        colors.text_primary
    } else {
        colors.text_primary.with_alpha(220)
    };
    kit::glyph(
        ctx.painter,
        kit::CHEVRON_DOWN,
        kit::square(Point::new(rect.right - 9.0, rect.center_y() + 1.0), 14.0),
        color,
    );
    text_right(
        ctx.painter,
        value,
        typography::BODY,
        Rect::from_ltrb(rect.left + 120.0, rect.top, rect.right - 22.0, rect.bottom),
        color,
    );
}

fn draw_panel(ctx: &TabContext<'_>, state: &State, layout: &Layout) {
    let painter = ctx.painter;
    let colors = ctx.colors;
    let editable = !state.running();

    caption(
        ctx,
        "FORMAT",
        Rect::from_xywh(layout.panel.left + 2.0, layout.panel.top, 120.0, 16.0),
    );
    painter.fill_round_rect(
        layout.segments,
        Radius::uniform(15.0),
        colors.surface_raised,
    );
    for (index, format) in Format::ALL.iter().enumerate() {
        let rect = segment(layout, index);
        let selected = state.format == *format;
        let hover = hovered(ctx, ID_FORMAT + index as u16) && editable;
        if selected {
            painter.fill_round_rect(rect, Radius::uniform(12.0), colors.surface_overlay);
            painter.stroke_round_rect(
                rect.inset(0.5),
                Radius::uniform(12.0),
                1.2,
                colors.text_primary.with_alpha(200),
            );
        } else if hover {
            painter.fill_round_rect(
                rect,
                Radius::uniform(12.0),
                colors.surface_overlay.with_alpha(160),
            );
        }
        text_center(
            painter,
            format.label(),
            style(14.0, if selected { 650 } else { 500 }),
            rect,
            if selected || hover {
                colors.text_primary
            } else {
                colors.text_secondary
            },
        );
    }

    // SIZE: quality or target, then the slider.
    caption(
        ctx,
        "SIZE",
        Rect::from_xywh(
            layout.size_row.left + 2.0,
            layout.size_row.top,
            80.0,
            layout.size_row.height(),
        ),
    );
    let lossy = state.format.lossy();
    if lossy {
        let (quality, target) = mode_buttons(layout);
        for (rect, label, id, active) in [
            (quality, "Quality", ID_MODE_QUALITY, !state.use_target),
            (target, "Target", ID_MODE_TARGET, state.use_target),
        ] {
            let hover = hovered(ctx, id) && editable;
            text_center(
                painter,
                label,
                if active {
                    style(14.0, 600)
                } else {
                    typography::BODY
                },
                rect,
                if active || hover {
                    colors.text_primary
                } else {
                    colors.text_secondary
                },
            );
        }
        text_center(
            painter,
            "·",
            typography::BODY,
            Rect::from_ltrb(quality.right, quality.top, target.left, quality.bottom),
            colors.text_secondary,
        );
    } else {
        text_right(
            painter,
            "Lossless · quality not used",
            typography::LABEL,
            layout.size_row,
            colors.text_secondary,
        );
    }
    let bar = track(layout);
    let value = if state.use_target {
        state.target
    } else {
        state.quality
    };
    let fraction = f32::from(value.saturating_sub(10)) / 90.0;
    let active_color = if lossy && editable {
        colors.text_primary
    } else {
        colors.text_disabled
    };
    painter.fill_round_rect(bar, Radius::uniform(2.0), colors.surface_overlay);
    let knob_x = bar.left + bar.width() * fraction.clamp(0.0, 1.0);
    painter.fill_round_rect(
        Rect::from_ltrb(bar.left, bar.top, knob_x.max(bar.left + 4.0), bar.bottom),
        Radius::uniform(2.0),
        active_color,
    );
    let slider_hover = matches!(ctx.hover, Some(DashboardHit::Tab(id)) if (ID_SLIDER..=ID_SLIDER + SLIDER_STEPS).contains(&id));
    painter.fill_circle(
        Point::new(knob_x, bar.center_y()),
        if slider_hover && lossy { 9.0 } else { 8.0 },
        active_color,
    );
    let percent = if lossy {
        format!("{value}%")
    } else {
        "—".to_string()
    };
    text_right(
        painter,
        &percent,
        style(14.0, 600),
        Rect::from_ltrb(
            bar.right + 8.0,
            layout.slider.top,
            layout.slider.right,
            layout.slider.bottom,
        ),
        if lossy {
            colors.text_primary
        } else {
            colors.text_disabled
        },
    );
    let note = match (state.use_target && lossy, state.to_downloads) {
        (true, false) => {
            "Aims for that share of each file's size · saved next to it, never overwrites"
        }
        (true, true) => {
            "Aims for that share of each file's size · saved to Downloads, never overwrites"
        }
        (false, false) => "Saved next to each original · never overwrites (adds \"(1)\")",
        (false, true) => "Saved to Downloads · never overwrites (adds \"(1)\")",
    };
    text(
        painter,
        note,
        typography::CAPTION,
        Rect::from_xywh(
            layout.note.left + 2.0,
            layout.note.top,
            layout.note.width() - 2.0,
            layout.note.height(),
        ),
        colors.text_secondary,
    );

    for y in [layout.resize_row.top - 3.0, layout.save_row.top - 3.0] {
        painter.fill_rect(
            Rect::from_xywh(layout.panel.left, y, layout.panel.width(), 1.0),
            colors.border_subtle,
        );
    }
    picker_row(
        ctx,
        layout.resize_row,
        "RESIZE",
        state.resize.label(),
        ID_RESIZE,
        editable,
    );
    picker_row(
        ctx,
        layout.save_row,
        "SAVE TO",
        if state.to_downloads {
            "Downloads"
        } else {
            "Same folder"
        },
        ID_SAVE_TO,
        editable,
    );

    // Footer: estimate or progress, and the action pill.
    let (pill_id, label) = pill_label(state);
    let pill = pill(layout, &label);
    let footer_text = if let Some(job) = &state.job {
        format!(
            "Converting {} of {}…",
            (job.finished + 1).min(job.total),
            job.total
        )
    } else if let Some(summary) = &state.summary {
        match (summary.saved, summary.failed) {
            (0, failed) => format!("{failed} failed · nothing saved"),
            (saved, 0) => format!(
                "Saved {saved} file{} · {}",
                if saved == 1 { "" } else { "s" },
                format_bytes(summary.bytes)
            ),
            (saved, failed) => format!("Saved {saved} · {failed} failed"),
        }
    } else if state.files.is_empty() {
        "Add images on the left to start".to_string()
    } else {
        let estimate = state.estimate();
        let total = state.total_bytes();
        let change = if estimate <= total {
            format!("saves {}", format_bytes(total - estimate))
        } else {
            format!("adds {}", format_bytes(estimate - total))
        };
        format!("Est. {} total · {change}", format_bytes(estimate))
    };
    text(
        painter,
        &footer_text,
        typography::LABEL,
        Rect::from_ltrb(
            layout.footer.left + 2.0,
            layout.footer.top,
            pill.left - 10.0,
            layout.footer.bottom,
        ),
        if state
            .summary
            .as_ref()
            .is_some_and(|summary| summary.failed > 0 && summary.saved == 0)
        {
            colors.state_error
        } else {
            colors.text_secondary
        },
    );
    let radius = Radius::uniform(16.0);
    let hover = hovered(ctx, pill_id);
    if let Some(job) = &state.job {
        painter.fill_round_rect(pill, radius, colors.surface_overlay);
        let progress = job.finished as f32 / job.total.max(1) as f32;
        if progress > 0.0 {
            let save = painter.save();
            painter.clip_round_rect(pill, radius);
            painter.fill_rect(
                Rect::from_xywh(pill.left, pill.top, pill.width() * progress, pill.height()),
                colors.text_primary.with_alpha(70),
            );
            painter.restore_to(save);
        }
        text_center(painter, &label, style(14.0, 600), pill, colors.text_primary);
    } else if state.files.is_empty() {
        painter.fill_round_rect(pill, radius, colors.surface_overlay);
        text_center(
            painter,
            &label,
            style(14.0, 600),
            pill,
            colors.text_disabled,
        );
    } else {
        painter.fill_round_rect(
            pill,
            radius,
            if hover {
                colors.text_primary.with_alpha(225)
            } else {
                colors.text_primary
            },
        );
        text_center(painter, &label, style(14.0, 600), pill, colors.surface_base);
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
    LAST_GRID.with(|cell| cell.set(grid));
    let layout = layout(grid);
    with_state(|state| hit_id(&layout, state, point)).map(DashboardHit::Tab)
}

fn hit_id(layout: &Layout, state: &State, point: Point) -> Option<u16> {
    if point.x < layout.divider_x {
        if layout.add.contains(point) {
            return Some(ID_ADD);
        }
        if layout.clear.contains(point) {
            return Some(ID_CLEAR);
        }
        let count = state.files.len();
        if count == 0 {
            let area = drop_area(layout, state)?;
            let (from_shelf, choose) = empty_buttons(area);
            if from_shelf.contains(point) {
                return Some(ID_FROM_SHELF);
            }
            if choose.contains(point) {
                return Some(ID_CHOOSE);
            }
            return None;
        }
        let (shown, more) = row_window(layout, count);
        for slot in 0..shown {
            let row = row_rect(layout, slot);
            if row_remove(row).contains(point) {
                return Some(ID_ROW_REMOVE + slot as u16);
            }
            if row.contains(point) {
                return Some(ID_ROW + slot as u16);
            }
        }
        if more && row_rect(layout, shown).contains(point) {
            return Some(ID_MORE);
        }
        return drop_area(layout, state)
            .filter(|area| area.contains(point))
            .map(|_| ID_CHOOSE);
    }
    if let Some(index) =
        (0..Format::ALL.len()).find(|index| segment(layout, *index).contains(point))
    {
        return Some(ID_FORMAT + index as u16);
    }
    if state.format.lossy() {
        let (quality, target) = mode_buttons(layout);
        if quality.contains(point) {
            return Some(ID_MODE_QUALITY);
        }
        if target.contains(point) {
            return Some(ID_MODE_TARGET);
        }
        let bar = track(layout);
        if layout.slider.contains(point)
            && point.x >= bar.left - 10.0
            && point.x <= bar.right + 10.0
        {
            let fraction = ((point.x - bar.left) / bar.width()).clamp(0.0, 1.0);
            return Some(ID_SLIDER + (fraction * f32::from(SLIDER_STEPS)).round() as u16);
        }
    }
    if layout.resize_row.inset(-2.0).contains(point) {
        return Some(ID_RESIZE);
    }
    if layout.save_row.inset(-2.0).contains(point) {
        return Some(ID_SAVE_TO);
    }
    let (id, label) = pill_label(state);
    pill(layout, &label).contains(point).then_some(id)
}

/// Applies a click from [`hit`]. `true` when handled.
pub fn apply(hit: DashboardHit) -> bool {
    let DashboardHit::Tab(id) = hit else {
        return false;
    };
    if !(700..800).contains(&id) {
        return false;
    }
    let layout = layout(LAST_GRID.with(Cell::get));
    with_state(|state| {
        if state.running() && id != ID_MORE && !(ID_ROW..ID_ROW + MAX_ROWS).contains(&id) {
            return;
        }
        match id {
            ID_ADD | ID_CHOOSE => open_picker(state),
            ID_CLEAR => {
                state.files.clear();
                state.manual = true;
                state.scroll = 0;
                state.touched();
            }
            ID_FROM_SHELF => {
                state.manual = false;
                sync_shelf(state, true);
            }
            ID_MODE_QUALITY | ID_MODE_TARGET => {
                state.use_target = id == ID_MODE_TARGET;
                state.touched();
            }
            ID_RESIZE => {
                state.resize = state.resize.next();
                state.touched();
            }
            ID_SAVE_TO => {
                state.to_downloads = !state.to_downloads && dirs::download_dir().is_some();
                state.touched();
            }
            ID_CONVERT => start(state),
            ID_SHOW_FOLDER => {
                if let Some(folder) = state.last_output.as_ref().and_then(|path| path.parent())
                    && let Err(error) = crate::platform::shell().open_url(&folder.to_string_lossy())
                {
                    log::warn!("Cannot open folder: {error}");
                }
            }
            ID_MORE => {
                let (shown, _) = row_window(&layout, state.files.len());
                state.scroll += shown.max(1);
                if state.scroll >= state.files.len() {
                    state.scroll = 0;
                }
            }
            _ if (ID_FORMAT..ID_FORMAT + 4).contains(&id) => {
                state.format = Format::ALL[usize::from(id - ID_FORMAT)];
                state.touched();
            }
            _ if (ID_SLIDER..=ID_SLIDER + SLIDER_STEPS).contains(&id) => {
                let value = 10 + 5 * (id - ID_SLIDER) as u8;
                if state.use_target {
                    state.target = value;
                } else {
                    state.quality = value;
                }
                state.touched();
            }
            _ if (ID_ROW_REMOVE..ID_ROW_REMOVE + MAX_ROWS).contains(&id) => {
                let (shown, _) = row_window(&layout, state.files.len());
                let first = state.scroll.min(state.files.len().saturating_sub(shown));
                let index = first + usize::from(id - ID_ROW_REMOVE);
                if index < state.files.len() {
                    state.files.remove(index);
                    state.manual = true;
                    state.touched();
                }
            }
            _ if (ID_ROW..ID_ROW + MAX_ROWS).contains(&id) => {
                let (shown, _) = row_window(&layout, state.files.len());
                let first = state.scroll.min(state.files.len().saturating_sub(shown));
                if let Some(file) = state.files.get(first + usize::from(id - ID_ROW)) {
                    let target = match &file.status {
                        Status::Done { output, .. } => output.clone(),
                        _ => file.path.clone(),
                    };
                    if let Err(error) = crate::platform::shell().open_url(&target.to_string_lossy())
                    {
                        log::warn!("Cannot open file: {error}");
                    }
                }
            }
            _ => {}
        }
    });
    true
}

pub fn wants_keyboard() -> bool {
    false
}

pub fn handle_key(_key: &Key) -> bool {
    false
}

/// Every frame, visible or not.
pub fn poll(_now: Instant) {
    thumbs::pump();
    with_state(|state| {
        collect(state);
        sync_shelf(state, false);
    });
}
