//! Dashboard shell: header, responsive card grid and bottom dock (roadmap step 5).
//!
//! Content comes from deterministic sample data until modules provide snapshots. The
//! header always labels it as sample data.

// `semantics` and grid scrolling (`overflows`, `content_height`) wait for the UI Automation
// provider and scroll support.
#![allow(dead_code)]

pub mod layout;
pub mod sample;

use std::cell::RefCell;

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

fn card_text(card: &SampleCard, focus: Option<&FocusView>) -> (String, String) {
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
pub fn is_animating() -> bool {
    PAGE.with(|page| page.borrow().focus.state() == TimerState::Running)
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

    pub const fn label_key(self) -> &'static str {
        match self {
            Self::Home => "dashboard_home",
            Self::Media => "dashboard_media",
            Self::Clipboard => "dashboard_clipboard",
            Self::Shelf => "dashboard_shelf",
            Self::Focus => "dashboard_focus",
            Self::More => "dashboard_more",
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
    if cards.is_empty() {
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
