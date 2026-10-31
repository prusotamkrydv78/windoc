//! Short live activities shown in the collapsed notch with one shared layout:
//! `[icon] Primary · secondary [progress] [buttons]`, or a taller two-line card
//! (`[big icon] Primary / secondary [buttons]`) for richer events.

use std::cell::RefCell;
use std::time::{Duration, Instant};

use winisland_render::text::{DrawTextCachedParams, FontManager};
use winisland_render::{FontStyle, Painter, Radius, Rect, Rgba, Vec2};

use crate::ui::compact::CompactSize;
use crate::ui::components::icon::{Icon, draw_icon};

const DISPLAY_DURATION: Duration = Duration::from_millis(2600);
/// Tall cards carry more to read and act on, so they stay longer.
const TALL_DISPLAY_DURATION: Duration = Duration::from_millis(4500);
/// Time left on an activity with buttons once the pointer leaves it.
const HOLD_AFTER_LEAVE: Duration = Duration::from_millis(1200);
const ENTER_DURATION: Duration = Duration::from_millis(220);
const FADE_DURATION: Duration = Duration::from_millis(260);
const TEXT_SIZE: f32 = 13.0;
const ICON_SIZE: f32 = 18.0;
const TALL_ICON_SIZE: f32 = 34.0;
const TALL_EXTRA_HEIGHT: f32 = 26.0;
const SIDE_PADDING: f32 = 18.0;
const ICON_GAP: f32 = 10.0;
const PROGRESS_WIDTH: f32 = 54.0;
const PROGRESS_GAP: f32 = 12.0;
const BUTTON_GAP: f32 = 6.0;
const MIN_WIDTH: f32 = 150.0;
pub(super) const MAX_WIDTH: f32 = 420.0;

#[derive(Clone, PartialEq)]
pub struct Activity {
    pub icon: Icon,
    pub accent: Rgba,
    pub primary: String,
    pub secondary: String,
    pub progress: Option<f32>,
    /// Buttons, left to right: `(label, target)`. The target is a URL or path opened on
    /// click, or an app action such as `winisland:call-mute`. The last one is highlighted.
    pub actions: Vec<(String, String)>,
    /// Two-line card with a large icon, for richer events.
    pub tall: bool,
    /// A higher priority interrupts the visible activity; equal or lower ones wait.
    pub priority: Priority,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Priority {
    Low,
    #[default]
    Normal,
    High,
    Critical,
}

#[derive(Default)]
pub(super) struct ActivityIndicator {
    current: Option<Activity>,
    shown_at: Option<Instant>,
    updated_at: Option<Instant>,
    /// Activities waiting for the current one to finish, highest priority first and oldest
    /// first within a priority, one per kind.
    queue: std::collections::VecDeque<Activity>,
    /// The pointer is on the notch; an activity with buttons stays until it leaves.
    hovered: bool,
    /// Button rectangles and targets from the last draw.
    buttons: RefCell<Vec<(Rect, String)>>,
}

fn display_duration(activity: Option<&Activity>) -> Duration {
    if activity.is_some_and(|activity| activity.tall) {
        TALL_DISPLAY_DURATION
    } else {
        DISPLAY_DURATION
    }
}

fn button_width(label: &str, scale: f32) -> f32 {
    FontManager::global().measure_text_cached(label, 12.0 * scale, FontStyle::bold()) + 22.0 * scale
}

impl ActivityIndicator {
    /// Shows `activity`. An activity with the same icon updates in place instead of
    /// replaying its entrance, so repeated events do not flicker. A different one waits
    /// unless its priority is higher, in which case it interrupts and an interrupted
    /// activity with buttons comes back afterwards.
    pub(super) fn push(&mut self, activity: Activity) {
        let now = Instant::now();
        let visible = self.is_visible();
        let same_kind = visible
            && self
                .current
                .as_ref()
                .is_some_and(|current| current.icon == activity.icon);
        if visible && !same_kind {
            let current_priority = self.current.as_ref().map_or(Priority::Low, |c| c.priority);
            if activity.priority <= current_priority {
                self.enqueue(activity, false);
                return;
            }
            if let Some(current) = self.current.take()
                && !current.actions.is_empty()
            {
                self.enqueue(current, true);
            }
            self.queue.retain(|queued| queued.icon != activity.icon);
        }
        if !same_kind {
            self.shown_at = Some(now);
        }
        self.updated_at = Some(now);
        self.current = Some(activity);
    }

    /// Queues `activity` behind everything of higher priority: after its equals, or ahead
    /// of them when `first` is set (an interrupted activity resumes before newer ones).
    fn enqueue(&mut self, activity: Activity, first: bool) {
        self.queue.retain(|queued| queued.icon != activity.icon);
        let at = self
            .queue
            .iter()
            .position(|queued| {
                if first {
                    queued.priority <= activity.priority
                } else {
                    queued.priority < activity.priority
                }
            })
            .unwrap_or(self.queue.len());
        self.queue.insert(at, activity);
    }

    /// Starts the next queued activity once the current one has finished, and holds an
    /// activity with buttons while the pointer is on it. Call every frame.
    pub(super) fn advance(&mut self) {
        if self.is_held() {
            let duration = display_duration(self.current.as_ref());
            let now = Instant::now();
            if let Some(hold) = now.checked_sub(duration.saturating_sub(HOLD_AFTER_LEAVE)) {
                self.updated_at = self.updated_at.max(Some(hold));
            }
        }
        if !self.is_visible()
            && let Some(next) = self.queue.pop_front()
        {
            self.push(next);
        }
    }

    /// Records whether the pointer is on the notch. Returns true while that holds a visible
    /// activity with buttons, so the caller can keep the notch collapsed for a click.
    pub(super) fn set_hovered(&mut self, hovered: bool) -> bool {
        self.hovered = hovered;
        self.is_held()
    }

    fn is_held(&self) -> bool {
        self.hovered
            && self.is_visible()
            && self
                .current
                .as_ref()
                .is_some_and(|current| !current.actions.is_empty())
    }

    /// The action target when `point` is on one of the visible activity's buttons.
    pub(super) fn action_at(&self, point: winisland_render::Point) -> Option<String> {
        if !self.is_visible() {
            return None;
        }
        self.buttons
            .borrow()
            .iter()
            .find(|(rect, _)| rect.contains(point))
            .map(|(_, target)| target.clone())
    }

    pub(super) fn is_visible(&self) -> bool {
        let duration = display_duration(self.current.as_ref());
        self.updated_at
            .is_some_and(|at| at.elapsed() < duration + FADE_DURATION)
    }

    pub(super) fn target_size(&self, base_width: f32, base_height: f32, scale: f32) -> CompactSize {
        let Some(activity) = &self.current else {
            return CompactSize {
                width: base_width * scale,
                height: base_height * scale,
            };
        };
        let fonts = FontManager::global();
        let buttons: f32 = activity
            .actions
            .iter()
            .map(|(label, _)| button_width(label, 1.0) + BUTTON_GAP)
            .sum();
        let mut width = 2.0 * SIDE_PADDING + buttons;
        if activity.tall {
            width += TALL_ICON_SIZE + ICON_GAP;
            width += fonts
                .measure_text_cached(&activity.primary, TEXT_SIZE + 1.0, FontStyle::bold())
                .max(fonts.measure_text_cached(
                    &activity.secondary,
                    TEXT_SIZE - 0.5,
                    FontStyle::normal(),
                ));
        } else {
            width += ICON_SIZE + ICON_GAP;
            width += fonts.measure_text_cached(&activity.primary, TEXT_SIZE, FontStyle::bold());
            if !activity.secondary.is_empty() {
                width += fonts.measure_text_cached(
                    &format!("  ·  {}", activity.secondary),
                    TEXT_SIZE,
                    FontStyle::normal(),
                );
            }
        }
        if activity.progress.is_some() {
            width += PROGRESS_GAP + PROGRESS_WIDTH;
        }
        let height = base_height
            + if activity.tall {
                TALL_EXTRA_HEIGHT
            } else {
                0.0
            };
        CompactSize {
            width: width.clamp(MIN_WIDTH.max(base_width), MAX_WIDTH) * scale,
            height: height * scale,
        }
    }

    /// Opacity and vertical offset for the entrance and exit.
    fn presentation(&self) -> (f32, f32) {
        let (Some(shown), Some(updated)) = (self.shown_at, self.updated_at) else {
            return (0.0, 0.0);
        };
        let duration = display_duration(self.current.as_ref());
        let enter = (shown.elapsed().as_secs_f32() / ENTER_DURATION.as_secs_f32()).min(1.0);
        let since_update = updated.elapsed();
        let exit = if since_update > duration {
            ((since_update - duration).as_secs_f32() / FADE_DURATION.as_secs_f32()).min(1.0)
        } else {
            0.0
        };
        let eased = 1.0 - (1.0 - enter).powi(3);
        (eased * (1.0 - exit), (1.0 - eased) * 4.0 - exit * 4.0)
    }

    pub(super) fn draw(&self, painter: Painter<'_>, rect: Rect, scale: f32, alpha: f32) {
        self.buttons.borrow_mut().clear();
        let Some(activity) = &self.current else {
            return;
        };
        let (opacity, offset_y) = self.presentation();
        let alpha = (alpha * opacity).clamp(0.0, 1.0);
        if alpha <= 0.01 {
            return;
        }
        let to_alpha = |value: f32| (value * alpha * 255.0) as u8;
        let fonts = FontManager::global();
        painter.save();
        painter.translate(Vec2::new(0.0, offset_y * scale));

        let center_y = rect.center_y();
        let left = rect.left + SIDE_PADDING * scale;
        let mut right = rect.right - SIDE_PADDING * scale;

        // Icon: plain glyph, or a tinted rounded tile on tall cards.
        let icon_size = if activity.tall {
            TALL_ICON_SIZE
        } else {
            ICON_SIZE
        } * scale;
        let icon_rect = Rect::from_xywh(left, center_y - icon_size / 2.0, icon_size, icon_size);
        if activity.icon == Icon::Pet {
            // The robot stands at full height, without a tile.
            crate::ui::widget::compact::mascot::draw_waving(painter, icon_rect, to_alpha(1.0));
        } else if activity.tall {
            painter.fill_round_rect(
                icon_rect,
                Radius::uniform(10.0 * scale),
                activity.accent.with_alpha(to_alpha(0.22)),
            );
            draw_icon(
                painter,
                activity.icon,
                icon_rect.inset(8.0 * scale),
                activity.accent.with_alpha(to_alpha(1.0)),
            );
        } else {
            draw_icon(
                painter,
                activity.icon,
                icon_rect,
                activity.accent.with_alpha(to_alpha(1.0)),
            );
        }

        // Buttons from the right; the last action is the highlighted (blue) one.
        let mut buttons = Vec::new();
        for (index, (label, target)) in activity.actions.iter().enumerate().rev() {
            let label_size = 12.0 * scale;
            let pill_w = button_width(label, scale);
            let pill_h = 24.0 * scale;
            let pill = Rect::from_xywh(right - pill_w, center_y - pill_h / 2.0, pill_w, pill_h);
            let primary = index + 1 == activity.actions.len();
            let fill = if primary {
                Rgba::from_rgb(10, 132, 255)
            } else {
                Rgba::from_rgb(58, 58, 60)
            };
            painter.fill_round_rect(
                pill,
                Radius::uniform(pill_h / 2.0),
                fill.with_alpha(to_alpha(1.0)),
            );
            fonts.draw_text_cached(DrawTextCachedParams {
                painter,
                text: label,
                x: pill.left + 11.0 * scale,
                y: center_y + label_size * 0.36,
                size: label_size,
                bold: true,
                color: Rgba::WHITE.with_alpha(to_alpha(1.0)),
                blur: None,
            });
            buttons.push((pill, target.clone()));
            right = pill.left - BUTTON_GAP * scale;
        }
        if !buttons.is_empty() {
            right -= 4.0 * scale;
        }
        *self.buttons.borrow_mut() = buttons;

        if let Some(progress) = activity.progress {
            let width = PROGRESS_WIDTH * scale;
            let height = 4.0 * scale;
            let bar = Rect::from_xywh(right - width, center_y - height / 2.0, width, height);
            let radius = Radius::uniform(height / 2.0);
            painter.fill_round_rect(bar, radius, Rgba::WHITE.with_alpha(to_alpha(0.18)));
            let filled = Rect::from_xywh(
                bar.left,
                bar.top,
                (width * progress.clamp(0.0, 1.0)).max(height),
                height,
            );
            painter.fill_round_rect(filled, radius, activity.accent.with_alpha(to_alpha(1.0)));
            right -= width + PROGRESS_GAP * scale;
        }

        let x = icon_rect.right + ICON_GAP * scale;
        let text = |value: &str, x: f32, y: f32, size: f32, bold: bool, color: Rgba| {
            fonts.draw_text_cached(DrawTextCachedParams {
                painter,
                text: value,
                x,
                y,
                size,
                bold,
                color,
                blur: None,
            });
        };
        let fit = |value: &str, size: f32, style: FontStyle, width: f32| {
            crate::utils::settings_ui::ellipsize_text(fonts, value, size, style, width.max(0.0))
        };
        if activity.tall {
            let primary_size = (TEXT_SIZE + 1.0) * scale;
            let secondary_size = (TEXT_SIZE - 0.5) * scale;
            let primary = fit(
                &activity.primary,
                primary_size,
                FontStyle::bold(),
                right - x,
            );
            text(
                &primary,
                x,
                center_y - 3.0 * scale,
                primary_size,
                true,
                Rgba::WHITE.with_alpha(to_alpha(1.0)),
            );
            let secondary = fit(
                &activity.secondary,
                secondary_size,
                FontStyle::normal(),
                right - x,
            );
            text(
                &secondary,
                x,
                center_y + 14.0 * scale,
                secondary_size,
                false,
                Rgba::WHITE.with_alpha(to_alpha(0.6)),
            );
        } else {
            let size = TEXT_SIZE * scale;
            let baseline = center_y + size * 0.36;
            let primary = fit(&activity.primary, size, FontStyle::bold(), right - x);
            text(
                &primary,
                x,
                baseline,
                size,
                true,
                Rgba::WHITE.with_alpha(to_alpha(1.0)),
            );
            let after = x + fonts.measure_text_cached(&primary, size, FontStyle::bold());
            if !activity.secondary.is_empty() && right - after > 24.0 * scale {
                let secondary = fit(
                    &format!("  ·  {}", activity.secondary),
                    size,
                    FontStyle::normal(),
                    right - after,
                );
                text(
                    &secondary,
                    after,
                    baseline,
                    size,
                    false,
                    Rgba::WHITE.with_alpha(to_alpha(0.55)),
                );
            }
        }
        painter.restore();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn activity(icon: Icon, priority: Priority, buttons: bool) -> Activity {
        Activity {
            icon,
            accent: Rgba::from_rgb(255, 255, 255),
            primary: String::new(),
            secondary: String::new(),
            progress: None,
            actions: if buttons {
                vec![("Open".into(), "winisland:test".into())]
            } else {
                Vec::new()
            },
            tall: false,
            priority,
        }
    }

    fn queued(indicator: &ActivityIndicator) -> Vec<Icon> {
        indicator.queue.iter().map(|queued| queued.icon).collect()
    }

    #[test]
    fn equal_priority_waits_in_order() {
        let mut indicator = ActivityIndicator::default();
        indicator.push(activity(Icon::Music, Priority::Normal, false));
        indicator.push(activity(Icon::Calendar, Priority::Normal, false));
        indicator.push(activity(Icon::Clipboard, Priority::Normal, false));
        assert_eq!(indicator.current.as_ref().unwrap().icon, Icon::Music);
        assert_eq!(queued(&indicator), [Icon::Calendar, Icon::Clipboard]);
    }

    #[test]
    fn higher_priority_jumps_the_queue() {
        let mut indicator = ActivityIndicator::default();
        indicator.push(activity(Icon::Music, Priority::High, false));
        indicator.push(activity(Icon::Calendar, Priority::Low, false));
        indicator.push(activity(Icon::Clipboard, Priority::Normal, false));
        assert_eq!(queued(&indicator), [Icon::Clipboard, Icon::Calendar]);
    }

    #[test]
    fn interrupted_activity_with_buttons_resumes_first() {
        let mut indicator = ActivityIndicator::default();
        indicator.push(activity(Icon::Calendar, Priority::Normal, true));
        indicator.push(activity(Icon::Clipboard, Priority::Normal, false));
        indicator.push(activity(Icon::Music, Priority::Critical, false));
        assert_eq!(indicator.current.as_ref().unwrap().icon, Icon::Music);
        assert_eq!(queued(&indicator), [Icon::Calendar, Icon::Clipboard]);
    }

    #[test]
    fn interrupted_activity_without_buttons_is_dropped() {
        let mut indicator = ActivityIndicator::default();
        indicator.push(activity(Icon::Clipboard, Priority::Normal, false));
        indicator.push(activity(Icon::Music, Priority::High, false));
        assert!(queued(&indicator).is_empty());
    }

    #[test]
    fn same_kind_updates_in_place() {
        let mut indicator = ActivityIndicator::default();
        indicator.push(activity(Icon::Music, Priority::Normal, false));
        let shown = indicator.shown_at;
        indicator.push(activity(Icon::Music, Priority::Low, false));
        assert_eq!(indicator.shown_at, shown);
        assert!(queued(&indicator).is_empty());
    }

    #[test]
    fn hover_holds_only_activities_with_buttons() {
        let mut indicator = ActivityIndicator::default();
        indicator.push(activity(Icon::Music, Priority::Normal, false));
        assert!(!indicator.set_hovered(true));
        let mut indicator = ActivityIndicator::default();
        indicator.push(activity(Icon::Calendar, Priority::Normal, true));
        assert!(indicator.set_hovered(true));
        assert!(!indicator.set_hovered(false));
    }
}
