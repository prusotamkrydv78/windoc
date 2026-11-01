//! Shelf tab: the parked files of every shelf as big file tiles.
//!
//! Left: the list of shelves with their item counts. Right: the active shelf's tiles with
//! sort, select, copy and clear actions, and a drop zone. Shelf data lives in the app's
//! `ShelfService`; this tab reads the snapshot the frame loop publishes (`sync_shelf`) and
//! sends changes back through the dashboard's one-per-frame request cells, which `poll`
//! feeds one request at a time.

pub(super) mod kit;
pub(super) mod thumbs;

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use winisland_platform::Key;
use winisland_render::tokens::{TextStyle, typography};
use winisland_render::{Point, Radius, Rect};

use super::TabContext;
use crate::ui::dashboard::{DashboardHit, Destination};
use kit::{format_bytes, style, text, text_center, text_right, text_width};

const ID_NEW_SHELF: u16 = 600;
const ID_SORT: u16 = 601;
const ID_SELECT: u16 = 602;
const ID_COPY: u16 = 603;
const ID_CLEAR: u16 = 604;
const ID_CONVERT: u16 = 605;
const ID_MORE: u16 = 606;
const ID_SHELF_ROW: u16 = 610;
const MAX_SHELF_ROWS: u16 = 20;
const ID_TILE: u16 = 630;
const ID_TILE_REMOVE: u16 = 660;
const MAX_TILES: u16 = 30;

const SIDEBAR_W: f32 = 168.0;
const SHELF_ROW_H: f32 = 32.0;
const SHELF_ROW_STEP: f32 = 34.0;
const TILE_MIN_W: f32 = 92.0;
const TILE_MIN_H: f32 = 104.0;
const TILE_GAP: f32 = 8.0;
const FEEDBACK: Duration = Duration::from_millis(1600);
const CONFIRM: Duration = Duration::from_secs(3);

const NAME: TextStyle = style(13.0, 500);
const TITLE: TextStyle = style(19.0, 650);

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Sort {
    #[default]
    Recent,
    Name,
    Size,
    Type,
}

impl Sort {
    fn label(self) -> &'static str {
        match self {
            Self::Recent => "Recent",
            Self::Name => "Name",
            Self::Size => "Size",
            Self::Type => "Type",
        }
    }

    fn next(self) -> Self {
        match self {
            Self::Recent => Self::Name,
            Self::Name => Self::Size,
            Self::Size => Self::Type,
            Self::Type => Self::Recent,
        }
    }
}

#[derive(Clone)]
struct Item {
    id: u64,
    name: String,
    path: PathBuf,
    exists: bool,
    is_dir: bool,
    bytes: u64,
    /// Position on the shelf (the order files were added in).
    order: usize,
}

#[derive(Default)]
struct State {
    revision: Option<u64>,
    name: String,
    items: Vec<Item>,
    total_bytes: u64,
    /// `(name, item count)` for every shelf, in the store's order.
    shelves: Vec<(String, usize)>,
    active: usize,
    sort: Sort,
    selecting: bool,
    selected: HashSet<u64>,
    page: usize,
    /// Shelf switches still to send, one per frame.
    pending_cycles: usize,
    /// Entries still to remove, one per frame.
    pending_removals: Vec<u64>,
    confirm_clear: Option<Instant>,
    copied: Option<Instant>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
    /// The grid of the last draw or hit test; clicks map onto the same layout.
    static LAST_GRID: Cell<Rect> = const { Cell::new(Rect::from_xywh(0.0, 0.0, 696.0, 222.0)) };
}

fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> R {
    STATE.with(|cell| f(&mut cell.borrow_mut()))
}

/// Rebuilds the tiles when the app published a new shelf snapshot.
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
    state.items = entries
        .into_iter()
        .enumerate()
        .map(|(order, (id, name, path, exists))| {
            let meta = if exists {
                std::fs::metadata(&path).ok()
            } else {
                None
            };
            Item {
                id,
                name,
                exists,
                is_dir: meta.as_ref().is_some_and(std::fs::Metadata::is_dir),
                bytes: meta
                    .as_ref()
                    .filter(|meta| meta.is_file())
                    .map_or(0, std::fs::Metadata::len),
                path,
                order,
            }
        })
        .collect();
    state.total_bytes = state.items.iter().map(|item| item.bytes).sum();
    let ids: HashSet<u64> = state.items.iter().map(|item| item.id).collect();
    state.selected.retain(|id| ids.contains(id));
    if switched {
        state.selected.clear();
        state.selecting = false;
        state.page = 0;
        state.confirm_clear = None;
    }
    sort_items(state);
    state.shelves = crate::core::json_store::read::<winisland_core::shelf::ShelfSnapshot>(
        crate::core::shelf_store::FILE_NAME,
        4 * 1024 * 1024,
    )
    .ok()
    .flatten()
    .map(|snapshot| {
        snapshot
            .shelves
            .iter()
            .map(|shelf| (shelf.name.clone(), shelf.entries().len()))
            .collect()
    })
    .unwrap_or_default();
    match state
        .shelves
        .iter()
        .position(|(name, _)| *name == state.name)
    {
        Some(index) => state.active = index,
        None => {
            state.shelves = vec![(state.name.clone(), 0)];
            state.active = 0;
        }
    }
    let count = state.items.len();
    if let Some(active) = state.shelves.get_mut(state.active) {
        active.1 = count;
    }
}

fn sort_items(state: &mut State) {
    let items = &mut state.items;
    match state.sort {
        Sort::Recent => items.sort_by(|a, b| b.order.cmp(&a.order)),
        Sort::Name => items.sort_by_key(|item| item.name.to_lowercase()),
        Sort::Size => items.sort_by(|a, b| b.bytes.cmp(&a.bytes)),
        Sort::Type => items.sort_by_cached_key(|item| {
            (
                !item.is_dir,
                kit::kind(&item.path).label,
                item.name.to_lowercase(),
            )
        }),
    }
}

struct Layout {
    sidebar: Rect,
    divider_x: f32,
    header: Rect,
    tiles: Rect,
    drop: Rect,
    new_shelf: Rect,
    rows_top: f32,
    rows_bottom: f32,
}

fn layout(grid: Rect) -> Layout {
    let sidebar = Rect::from_xywh(grid.left, grid.top, SIDEBAR_W, grid.height());
    let divider_x = sidebar.right + 8.0;
    let main = Rect::from_ltrb(divider_x + 9.0, grid.top, grid.right, grid.bottom);
    let header = Rect::from_xywh(main.left, main.top, main.width(), 28.0);
    let drop = Rect::from_ltrb(main.left, main.bottom - 32.0, main.right, main.bottom);
    let tiles = Rect::from_ltrb(main.left, header.bottom + 6.0, main.right, drop.top - 6.0);
    let new_shelf = Rect::from_xywh(sidebar.left, sidebar.bottom - 30.0, sidebar.width(), 30.0);
    Layout {
        sidebar,
        divider_x,
        header,
        tiles,
        drop,
        new_shelf,
        rows_top: sidebar.top + 24.0,
        rows_bottom: new_shelf.top - 8.0,
    }
}

/// `(first shelf index shown, how many rows fit)`.
fn shelf_window(layout: &Layout, state: &State) -> (usize, usize) {
    let fit = (((layout.rows_bottom - layout.rows_top + (SHELF_ROW_STEP - SHELF_ROW_H))
        / SHELF_ROW_STEP)
        .floor()
        .max(1.0) as usize)
        .min(MAX_SHELF_ROWS as usize);
    let first = if state.active >= fit {
        state.active + 1 - fit
    } else {
        0
    };
    (first, fit.min(state.shelves.len().saturating_sub(first)))
}

fn shelf_row(layout: &Layout, slot: usize) -> Rect {
    Rect::from_xywh(
        layout.sidebar.left,
        layout.rows_top + slot as f32 * SHELF_ROW_STEP,
        layout.sidebar.width(),
        SHELF_ROW_H,
    )
}

struct Action {
    id: u16,
    rect: Rect,
    label: String,
    chevron: bool,
    danger: bool,
}

fn actions(header: Rect, state: &State, now: Instant) -> Vec<Action> {
    let selected = state.selected.len();
    let confirming = state
        .confirm_clear
        .is_some_and(|at| now.saturating_duration_since(at) < CONFIRM);
    let copied = state
        .copied
        .is_some_and(|at| now.saturating_duration_since(at) < FEEDBACK);
    let specs = [
        (
            ID_CLEAR,
            if state.selecting {
                if selected > 0 {
                    format!("Remove ({selected})")
                } else {
                    "Remove".to_string()
                }
            } else if confirming {
                "Confirm clear".to_string()
            } else {
                "Clear".to_string()
            },
            false,
            confirming || (state.selecting && selected > 0),
        ),
        (
            ID_COPY,
            if copied {
                "Copied".to_string()
            } else if state.selecting {
                format!("Copy ({selected})")
            } else {
                "Copy all".to_string()
            },
            false,
            false,
        ),
        (
            ID_SELECT,
            if state.selecting { "Done" } else { "Select" }.to_string(),
            false,
            false,
        ),
        (ID_SORT, state.sort.label().to_string(), true, false),
    ];
    let mut right = header.right;
    let mut out = Vec::with_capacity(specs.len());
    for (id, label, chevron, danger) in specs {
        let width = text_width(&label, typography::BODY) + 16.0 + if chevron { 14.0 } else { 0.0 };
        let rect = Rect::from_xywh(right - width, header.center_y() - 13.0, width, 26.0);
        right = rect.left - 2.0;
        out.push(Action {
            id,
            rect,
            label,
            chevron,
            danger,
        });
    }
    out
}

struct TileGrid {
    columns: usize,
    rows: usize,
    width: f32,
    height: f32,
}

fn tile_grid(tiles: Rect) -> TileGrid {
    let columns = (((tiles.width() + TILE_GAP) / (TILE_MIN_W + TILE_GAP)).floor() as usize).max(1);
    let rows = (((tiles.height() + TILE_GAP) / (TILE_MIN_H + TILE_GAP)).floor() as usize).max(1);
    let width = (tiles.width() - TILE_GAP * (columns - 1) as f32) / columns as f32;
    let height = ((tiles.height() - TILE_GAP * (rows - 1) as f32) / rows as f32).min(128.0);
    TileGrid {
        columns,
        rows,
        width,
        height,
    }
}

impl TileGrid {
    fn capacity(&self) -> usize {
        (self.columns * self.rows).min(MAX_TILES as usize)
    }

    fn rect(&self, tiles: Rect, slot: usize) -> Rect {
        let column = slot % self.columns;
        let row = slot / self.columns;
        Rect::from_xywh(
            tiles.left + column as f32 * (self.width + TILE_GAP),
            tiles.top + row as f32 * (self.height + TILE_GAP),
            self.width,
            self.height,
        )
    }

    /// `(items per page, whether a "more" tile is needed)`.
    fn paging(&self, count: usize) -> (usize, bool) {
        let capacity = self.capacity();
        if count > capacity && capacity > 1 {
            (capacity - 1, true)
        } else {
            (capacity, false)
        }
    }
}

/// The icon square and the remove/check badge of a tile.
fn tile_parts(tile: Rect) -> (Rect, Rect) {
    let size = (tile.width() - 28.0)
        .min(tile.height() - 44.0)
        .clamp(36.0, 72.0);
    let icon = Rect::from_xywh(tile.center_x() - size / 2.0, tile.top + 6.0, size, size);
    let badge = kit::square(Point::new(icon.right + 2.0, icon.top + 4.0), 20.0);
    (icon, badge)
}

/// Index range of the items on the visible page.
fn page_items(state: &State, per_page: usize) -> (usize, usize) {
    let pages = state.items.len().div_ceil(per_page.max(1)).max(1);
    let page = state.page.min(pages - 1);
    let start = page * per_page;
    (start, (start + per_page).min(state.items.len()))
}

fn drop_button(drop: Rect) -> Rect {
    let width = text_width("Convert images", typography::LABEL) + 22.0;
    Rect::from_xywh(
        drop.right - width - 4.0,
        drop.top + 4.0,
        width,
        drop.height() - 8.0,
    )
}

/// Draws the tab. Return `false` to fall back to the legacy page.
pub fn draw(ctx: &TabContext<'_>) -> bool {
    LAST_GRID.with(|cell| cell.set(ctx.grid));
    with_state(|state| {
        refresh(state);
        draw_page(ctx, state);
    });
    true
}

fn hovered(ctx: &TabContext<'_>, id: u16) -> bool {
    ctx.hover == Some(DashboardHit::Tab(id))
}

fn draw_page(ctx: &TabContext<'_>, state: &State) {
    let painter = ctx.painter;
    let colors = ctx.colors;
    let layout = layout(ctx.grid);

    // Sidebar.
    text(
        painter,
        "SHELVES",
        typography::CAPTION,
        Rect::from_xywh(layout.sidebar.left + 10.0, layout.sidebar.top, 120.0, 18.0),
        colors.text_secondary,
    );
    let (first, shown) = shelf_window(&layout, state);
    for slot in 0..shown {
        let index = first + slot;
        let Some((name, count)) = state.shelves.get(index) else {
            break;
        };
        let row = shelf_row(&layout, slot);
        let active = index == state.active;
        let hover = hovered(ctx, ID_SHELF_ROW + slot as u16);
        if active || hover {
            painter.fill_round_rect(
                row,
                Radius::uniform(10.0),
                if active {
                    colors.surface_overlay
                } else {
                    colors.surface_raised
                },
            );
        }
        let text_color = if active || hover {
            colors.text_primary
        } else {
            colors.text_secondary
        };
        kit::glyph(
            painter,
            kit::FOLDER,
            kit::square(Point::new(row.left + 20.0, row.center_y()), 16.0),
            text_color,
        );
        let count_w = text_right(
            painter,
            &count.to_string(),
            typography::LABEL,
            Rect::from_ltrb(row.left, row.top, row.right - 12.0, row.bottom),
            colors.text_secondary,
        );
        text(
            painter,
            name,
            if active {
                style(14.0, 600)
            } else {
                typography::BODY
            },
            Rect::from_ltrb(
                row.left + 36.0,
                row.top,
                row.right - 18.0 - count_w,
                row.bottom,
            ),
            text_color,
        );
    }
    painter.fill_rect(
        Rect::from_xywh(
            layout.sidebar.left + 10.0,
            layout.new_shelf.top - 4.0,
            layout.sidebar.width() - 20.0,
            1.0,
        ),
        colors.border_subtle,
    );
    let new_hover = hovered(ctx, ID_NEW_SHELF);
    if new_hover {
        painter.fill_round_rect(
            layout.new_shelf,
            Radius::uniform(10.0),
            colors.surface_raised,
        );
    }
    let new_color = if new_hover {
        colors.text_primary
    } else {
        colors.text_secondary
    };
    kit::glyph(
        painter,
        kit::PLUS,
        kit::square(
            Point::new(layout.new_shelf.left + 20.0, layout.new_shelf.center_y()),
            16.0,
        ),
        new_color,
    );
    text(
        painter,
        "New shelf",
        typography::BODY,
        Rect::from_ltrb(
            layout.new_shelf.left + 36.0,
            layout.new_shelf.top,
            layout.new_shelf.right - 8.0,
            layout.new_shelf.bottom,
        ),
        new_color,
    );
    painter.fill_rect(
        Rect::from_xywh(
            layout.divider_x,
            ctx.grid.top + 4.0,
            1.0,
            ctx.grid.height() - 8.0,
        ),
        colors.border_subtle,
    );

    // Header: name, summary and actions.
    let actions = actions(layout.header, state, ctx.now);
    let actions_left = actions
        .last()
        .map_or(layout.header.right, |action| action.rect.left);
    let summary = if state.items.len() == 1 {
        format!("1 item · {}", format_bytes(state.total_bytes))
    } else {
        format!(
            "{} items · {}",
            state.items.len(),
            format_bytes(state.total_bytes)
        )
    };
    let space = (actions_left - 10.0 - layout.header.left).max(0.0);
    let summary_w = text_width(&summary, typography::BODY).min(space * 0.5);
    let title_w = (text_width(&state.name, TITLE) + 2.0).min(space - summary_w - 10.0);
    text(
        painter,
        &state.name,
        TITLE,
        Rect::from_xywh(
            layout.header.left,
            layout.header.top,
            title_w.max(0.0),
            layout.header.height(),
        ),
        colors.text_primary,
    );
    text(
        painter,
        &summary,
        typography::BODY,
        Rect::from_xywh(
            layout.header.left + title_w + 10.0,
            layout.header.top + 1.0,
            summary_w + 1.0,
            layout.header.height(),
        ),
        colors.text_secondary,
    );
    for action in &actions {
        let hover = hovered(ctx, action.id);
        let enabled = action_enabled(state, action.id);
        if hover && enabled {
            painter.fill_round_rect(action.rect, Radius::uniform(13.0), colors.surface_overlay);
        }
        let color = if !enabled {
            colors.text_disabled
        } else if action.danger {
            colors.state_error
        } else if hover || (action.id == ID_SELECT && state.selecting) {
            colors.text_primary
        } else {
            colors.text_secondary
        };
        let label_rect = Rect::from_ltrb(
            action.rect.left + 8.0,
            action.rect.top,
            action.rect.right - 8.0,
            action.rect.bottom,
        );
        text(painter, &action.label, typography::BODY, label_rect, color);
        if action.chevron {
            kit::glyph(
                painter,
                kit::CHEVRON_DOWN,
                kit::square(
                    Point::new(label_rect.right - 6.0, action.rect.center_y() + 1.0),
                    14.0,
                ),
                color,
            );
        }
    }

    // Tiles.
    if state.items.is_empty() {
        let center = layout.tiles.center_y();
        kit::glyph(
            painter,
            kit::TRAY,
            kit::square(Point::new(layout.tiles.center_x(), center - 24.0), 28.0),
            colors.text_secondary,
        );
        text_center(
            painter,
            "This shelf is empty",
            typography::SECTION_TITLE,
            Rect::from_xywh(layout.tiles.left, center - 4.0, layout.tiles.width(), 20.0),
            colors.text_primary,
        );
        text_center(
            painter,
            "Drag files onto the island to park them here",
            typography::LABEL,
            Rect::from_xywh(layout.tiles.left, center + 18.0, layout.tiles.width(), 16.0),
            colors.text_secondary,
        );
    } else {
        draw_tiles(ctx, state, &layout);
    }

    // Drop zone.
    kit::dashed_round_rect(painter, layout.drop, 10.0, colors.border_strong);
    let button = drop_button(layout.drop);
    kit::glyph(
        painter,
        kit::TRAY,
        kit::square(
            Point::new(layout.drop.left + 20.0, layout.drop.center_y()),
            16.0,
        ),
        colors.text_secondary,
    );
    text(
        painter,
        "Drop files on the island to park them · originals never move",
        typography::LABEL,
        Rect::from_ltrb(
            layout.drop.left + 36.0,
            layout.drop.top,
            button.left - 12.0,
            layout.drop.bottom,
        ),
        colors.text_secondary,
    );
    painter.fill_rect(
        Rect::from_xywh(
            button.left - 6.0,
            layout.drop.top + 8.0,
            1.0,
            layout.drop.height() - 16.0,
        ),
        colors.border_subtle,
    );
    let convert_hover = hovered(ctx, ID_CONVERT);
    if convert_hover {
        painter.fill_round_rect(
            button,
            Radius::uniform(button.height() / 2.0),
            colors.surface_overlay,
        );
    }
    text_center(
        painter,
        "Convert images",
        style(12.0, 600),
        button,
        if convert_hover {
            colors.text_primary
        } else {
            colors.text_secondary
        },
    );
}

fn action_enabled(state: &State, id: u16) -> bool {
    match id {
        ID_COPY | ID_CLEAR if state.selecting => !state.selected.is_empty(),
        ID_COPY => state.items.iter().any(|item| item.exists),
        ID_CLEAR | ID_SELECT | ID_SORT => !state.items.is_empty(),
        _ => true,
    }
}

fn draw_tiles(ctx: &TabContext<'_>, state: &State, layout: &Layout) {
    let painter = ctx.painter;
    let colors = ctx.colors;
    let grid = tile_grid(layout.tiles);
    let (per_page, more) = grid.paging(state.items.len());
    let (start, end) = page_items(state, per_page);
    for (slot, item) in state.items[start..end].iter().enumerate() {
        let tile = grid.rect(layout.tiles, slot);
        let tile_id = ID_TILE + slot as u16;
        let remove_id = ID_TILE_REMOVE + slot as u16;
        let hover = hovered(ctx, tile_id) || hovered(ctx, remove_id);
        let selected = state.selecting && state.selected.contains(&item.id);
        if hover || selected {
            painter.fill_round_rect(
                tile,
                Radius::uniform(12.0),
                if selected {
                    colors.accent_soft
                } else {
                    colors.surface_raised
                },
            );
        }
        if selected {
            painter.stroke_round_rect(
                tile.inset(0.5),
                Radius::uniform(12.0),
                1.0,
                colors.accent_primary,
            );
        }
        let (icon, badge) = tile_parts(tile);
        kit::file_icon(painter, icon, &item.path, item.is_dir, !item.exists, colors);
        let name_rect =
            Rect::from_xywh(tile.left + 4.0, icon.bottom + 6.0, tile.width() - 8.0, 17.0);
        text_center(
            painter,
            &item.name,
            NAME,
            name_rect,
            if item.exists {
                colors.text_primary
            } else {
                colors.text_disabled
            },
        );
        let detail = if !item.exists {
            "Missing · click to remove".to_string()
        } else if item.is_dir {
            "Folder".to_string()
        } else {
            format_bytes(item.bytes)
        };
        text_center(
            painter,
            &detail,
            typography::CAPTION,
            Rect::from_xywh(tile.left + 4.0, name_rect.bottom, tile.width() - 8.0, 15.0),
            if item.exists {
                colors.text_secondary
            } else {
                colors.state_error
            },
        );
        if state.selecting {
            let center = Point::new(badge.center_x(), badge.center_y());
            if selected {
                painter.fill_circle(center, 9.0, colors.accent_primary);
                kit::glyph(
                    painter,
                    kit::CHECK,
                    kit::square(center, 13.0),
                    colors.text_primary,
                );
            } else {
                painter.fill_circle(center, 9.0, colors.surface_base.with_alpha(200));
                painter.stroke_circle(center, 8.5, 1.2, colors.text_secondary);
            }
        } else if hover {
            let center = Point::new(badge.center_x(), badge.center_y());
            let over = hovered(ctx, remove_id);
            painter.fill_circle(
                center,
                10.0,
                if over {
                    colors.state_error
                } else {
                    colors.surface_overlay
                },
            );
            painter.stroke_circle(center, 10.0, 1.0, colors.border_subtle);
            kit::glyph(
                painter,
                kit::CLOSE,
                kit::square(center, 13.0),
                colors.text_primary,
            );
        }
    }
    if more {
        let slot = end - start;
        let tile = grid.rect(layout.tiles, slot);
        let (icon, _) = tile_parts(tile);
        let hover = hovered(ctx, ID_MORE);
        painter.fill_round_rect(
            icon,
            Radius::uniform(14.0),
            if hover {
                colors.surface_interactive
            } else {
                colors.surface_overlay
            },
        );
        let after = state.items.len() - end;
        let label = if after > 0 {
            format!("+{after}")
        } else {
            "↺".to_string()
        };
        text_center(painter, &label, style(18.0, 650), icon, colors.text_primary);
        text_center(
            painter,
            if after > 0 {
                "Show more"
            } else {
                "Back to start"
            },
            NAME,
            Rect::from_xywh(tile.left + 4.0, icon.bottom + 6.0, tile.width() - 8.0, 17.0),
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
    LAST_GRID.with(|cell| cell.set(grid));
    let layout = layout(grid);
    with_state(|state| {
        if layout.new_shelf.contains(point) {
            return Some(ID_NEW_SHELF);
        }
        let (_, shown) = shelf_window(&layout, state);
        if let Some(slot) = (0..shown).find(|slot| shelf_row(&layout, *slot).contains(point)) {
            return Some(ID_SHELF_ROW + slot as u16);
        }
        if let Some(action) = actions(layout.header, state, Instant::now())
            .into_iter()
            .find(|action| action.rect.contains(point))
        {
            return Some(action.id);
        }
        if drop_button(layout.drop).contains(point) {
            return Some(ID_CONVERT);
        }
        if state.items.is_empty() || !layout.tiles.contains(point) {
            return None;
        }
        let tiles = tile_grid(layout.tiles);
        let (per_page, more) = tiles.paging(state.items.len());
        let (start, end) = page_items(state, per_page);
        for slot in 0..end - start {
            let tile = tiles.rect(layout.tiles, slot);
            let (_, badge) = tile_parts(tile);
            if !state.selecting && badge.inset(-2.0).contains(point) {
                return Some(ID_TILE_REMOVE + slot as u16);
            }
            if tile.contains(point) {
                return Some(ID_TILE + slot as u16);
            }
        }
        if more && tiles.rect(layout.tiles, end - start).contains(point) {
            return Some(ID_MORE);
        }
        None
    })
    .map(DashboardHit::Tab)
}

/// Applies a click from [`hit`]. `true` when handled.
pub fn apply(hit: DashboardHit) -> bool {
    let DashboardHit::Tab(id) = hit else {
        return false;
    };
    if !(600..700).contains(&id) {
        return false;
    }
    let now = Instant::now();
    let mut convert = false;
    with_state(|state| match id {
        ID_NEW_SHELF => {
            // Shelves are created and deleted in Settings › Behavior.
            crate::ui::dashboard::HEADER_REQUEST
                .with(|cell| cell.set(Some(DashboardHit::OpenSettings)));
        }
        ID_SORT => {
            state.sort = state.sort.next();
            state.page = 0;
            sort_items(state);
        }
        ID_SELECT => {
            state.selecting = !state.selecting && !state.items.is_empty();
            state.selected.clear();
        }
        ID_COPY => copy_paths(state, now),
        ID_CLEAR => clear(state, now),
        ID_CONVERT => convert = true,
        ID_MORE => {
            let (per_page, _) = tile_grid(layout(last_grid()).tiles).paging(state.items.len());
            let pages = state.items.len().div_ceil(per_page.max(1)).max(1);
            state.page = (state.page.min(pages - 1) + 1) % pages;
        }
        _ if (ID_SHELF_ROW..ID_SHELF_ROW + MAX_SHELF_ROWS).contains(&id) => {
            switch_shelf(state, (id - ID_SHELF_ROW) as usize);
        }
        _ if (ID_TILE..ID_TILE + MAX_TILES).contains(&id) => {
            click_tile(state, (id - ID_TILE) as usize, false);
        }
        _ if (ID_TILE_REMOVE..ID_TILE_REMOVE + MAX_TILES).contains(&id) => {
            click_tile(state, (id - ID_TILE_REMOVE) as usize, true);
        }
        _ => {}
    });
    if convert {
        super::converter::import_shelf();
        crate::ui::dashboard::show(Destination::Converter);
    }
    true
}

/// Switches to the shelf in sidebar row `slot` by cycling through the shelves.
fn switch_shelf(state: &mut State, slot: usize) {
    let count = state.shelves.len();
    if count == 0 {
        return;
    }
    let (first, _) = shelf_window(&layout(last_grid()), state);
    let target = first + slot;
    if target >= count || target == state.active || state.pending_cycles > 0 {
        return;
    }
    state.pending_cycles = (target + count - state.active) % count;
}

fn last_grid() -> Rect {
    LAST_GRID.with(Cell::get)
}

fn click_tile(state: &mut State, slot: usize, remove: bool) {
    let (per_page, _) = tile_grid(layout(last_grid()).tiles).paging(state.items.len());
    let (start, end) = page_items(state, per_page);
    let Some(item) = state.items[start..end].get(slot).cloned() else {
        return;
    };
    if state.selecting {
        if !state.selected.remove(&item.id) {
            state.selected.insert(item.id);
        }
        return;
    }
    if remove || !item.exists {
        state.pending_removals.push(item.id);
        return;
    }
    if let Err(error) = crate::platform::shell().open_url(&item.path.to_string_lossy()) {
        log::warn!("Cannot open shelf file: {error}");
    }
}

fn copy_paths(state: &mut State, now: Instant) {
    let paths: Vec<String> = state
        .items
        .iter()
        .filter(|item| item.exists)
        .filter(|item| !state.selecting || state.selected.contains(&item.id))
        .map(|item| item.path.display().to_string())
        .collect();
    if paths.is_empty() {
        return;
    }
    match winisland_platform_windows::set_clipboard_text(&paths.join("\r\n")) {
        Ok(()) => state.copied = Some(now),
        Err(error) => log::warn!("Cannot copy shelf paths: {error}"),
    }
}

fn clear(state: &mut State, now: Instant) {
    if state.selecting {
        let ids: Vec<u64> = state.selected.drain().collect();
        state.pending_removals.extend(ids);
        state.selecting = false;
        return;
    }
    if state.items.is_empty() {
        return;
    }
    let confirming = state
        .confirm_clear
        .is_some_and(|at| now.saturating_duration_since(at) < CONFIRM);
    if confirming {
        state.confirm_clear = None;
        state.pending_removals = state.items.iter().map(|item| item.id).collect();
        state.page = 0;
    } else {
        state.confirm_clear = Some(now);
    }
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
        refresh(state);
        if state.pending_cycles > 0 {
            let sent = crate::ui::dashboard::SHELF_CYCLE.with(|cell| {
                if cell.get() {
                    false
                } else {
                    cell.set(true);
                    true
                }
            });
            if sent {
                state.pending_cycles -= 1;
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
