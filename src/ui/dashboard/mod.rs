//! Dashboard shell: header, responsive card grid and bottom dock (roadmap step 5).
//!
//! Content comes from deterministic sample data until modules provide snapshots. The
//! header always labels it as sample data.

// Wired into the island in a follow-up change; until then nothing calls it.
#![allow(dead_code)]

pub mod layout;
pub mod sample;

use winisland_core::i18n::tr;
use winisland_render::tokens::{SemanticColors, radius, space, typography};
use winisland_render::{Painter, Radius, Rect};

use crate::ui::components::card::{CardLayout, draw_card};
use crate::ui::components::dock::draw_dock;
use crate::ui::components::semantics::{InteractionState, Role, SemanticNode};
use crate::ui::components::text::{Align, draw_text};
use layout::DashboardLayout;
use sample::SampleCard;

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
    pub interaction: DashboardInteraction,
}

pub fn draw_dashboard(params: DashboardParams<'_>) {
    let DashboardParams {
        painter,
        layout,
        colors,
        active,
        cards,
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
        draw_card_value(painter, &card_layout, card, colors);
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
    card: &SampleCard,
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
        card.value,
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
        &tr(card.detail_key),
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
