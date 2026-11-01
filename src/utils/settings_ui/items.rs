use winisland_render::Image;
use winisland_render::Rect;
use winisland_render::Rgba;

pub const CONTENT_PADDING: f32 = 24.0;
pub const ROW_HEIGHT: f32 = 48.0;
pub const GROUP_RADIUS: f32 = 12.0;
pub const GROUP_INNER_PAD: f32 = 16.0;
pub const SIDEBAR_PAD: f32 = 8.0;
pub const SIDEBAR_SEL_RADIUS: f32 = 7.0;

pub const TOGGLE_W: f32 = 36.0;
pub const TOGGLE_H: f32 = 20.0;
pub const TOGGLE_R: f32 = 10.0;
pub const TOGGLE_KNOB: f32 = 16.0;
pub const TOGGLE_INSET: f32 = 2.0;

pub const STEPPER_BTN_SIZE: f32 = 28.0;
pub const STEPPER_VALUE_W: f32 = 52.0;
pub const STEPPER_GAP: f32 = 0.0;

/// Wide enough for choices such as "Display 1: Generic monitor" without cutting them off.
pub const POPUP_BTN_W: f32 = 176.0;
pub const POPUP_BTN_H: f32 = 28.0;
/// Width of the trailing box in a text field row.
pub const TEXT_FIELD_W: f32 = 200.0;
pub const POPUP_BTN_R: f32 = 7.0;
pub const POPUP_ITEM_H: f32 = 32.0;
pub const POPUP_MENU_R: f32 = 10.0;
pub const POPUP_MENU_PAD: f32 = 5.0;
pub const PICKER_BTN_W: f32 = 72.0;
pub const PICKER_BTN_GAP: f32 = 6.0;

pub fn trailing_control_rect(
    y: f32,
    row_height: f32,
    content_width: f32,
    control_width: f32,
    control_height: f32,
) -> Rect {
    Rect::from_xywh(
        CONTENT_PADDING + content_width - GROUP_INNER_PAD - control_width,
        y + (row_height - control_height) / 2.0,
        control_width,
        control_height,
    )
}

pub fn picker_button_rects(y: f32, row_height: f32, content_width: f32) -> (Rect, Rect) {
    let primary = trailing_control_rect(y, row_height, content_width, PICKER_BTN_W, POPUP_BTN_H);
    let secondary = Rect::from_xywh(
        primary.left - PICKER_BTN_GAP - PICKER_BTN_W,
        primary.top,
        PICKER_BTN_W,
        POPUP_BTN_H,
    );
    (primary, secondary)
}

#[derive(Clone)]
pub enum SettingsItem {
    SectionHeader {
        label: String,
    },
    GroupStart,
    GroupEnd,
    RowStepper {
        label: String,
        value: String,
        enabled: bool,
    },
    RowSwitch {
        label: String,
        on: bool,
        enabled: bool,
    },
    RowFontPicker {
        label: String,
        btn_label: String,
        reset_label: Option<String>,
    },
    RowFolderPicker {
        label: String,
        btn_label: String,
        clear_label: Option<String>,
        current_path: Option<String>,
        enabled: bool,
    },
    RowSourceSelect {
        label: String,
        options: Vec<(String, bool)>,
        enabled: bool,
    },
    RowButton {
        label: String,
        btn_label: String,
        enabled: bool,
    },
    RowTextField {
        label: String,
        value: String,
        placeholder: String,
        enabled: bool,
    },
    RowAppItem {
        label: String,
        active: bool,
        enabled: bool,
    },
    RowLabel {
        label: String,
    },
    CenterLink {
        label: String,
        color: Rgba,
    },
    CenterText {
        text: String,
        size: f32,
        color: Rgba,
    },
    CenterImage {
        image: Image,
        size: f32,
        height: f32,
    },
    Spacer {
        height: f32,
    },
    Custom {
        height: f32,
    },
    WidgetPreview {
        height: f32,
    },
}

/// Description rows (`RowLabel`) wrap onto as many lines as they need.
pub const LABEL_SIZE: f32 = 13.0;
pub const LABEL_LINE_H: f32 = 18.0;
/// Space above the first line and below the last.
pub const LABEL_PAD: f32 = 13.0;

thread_local! {
    /// The width description text wraps to, set by the window whenever its width changes.
    static LABEL_WRAP_W: std::cell::Cell<f32> = const { std::cell::Cell::new(520.0) };
}

/// Sets the width description rows wrap to, from the page's content width.
pub fn set_label_wrap_width(content_width: f32) {
    let width = content_width - CONTENT_PADDING * 2.0 - GROUP_INNER_PAD * 2.0;
    LABEL_WRAP_W.with(|cell| cell.set(width.max(80.0)));
}

/// `text` broken into lines that fit the description width, breaking between words.
pub fn wrap_label(text: &str) -> Vec<String> {
    let width = LABEL_WRAP_W.with(std::cell::Cell::get);
    let fonts = winisland_render::text::FontManager::global();
    let measure =
        |line: &str| fonts.measure_text_cached(line, LABEL_SIZE, winisland_render::FontStyle::normal());
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        let candidate = if line.is_empty() {
            word.to_string()
        } else {
            format!("{line} {word}")
        };
        if measure(&candidate) <= width || line.is_empty() {
            line = candidate;
        } else {
            lines.push(std::mem::replace(&mut line, word.to_string()));
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

impl SettingsItem {
    pub fn height(&self) -> f32 {
        match self {
            SettingsItem::RowLabel { label } => {
                let lines = wrap_label(label).len().max(1) as f32;
                (lines * LABEL_LINE_H + LABEL_PAD * 2.0 - 4.0).max(ROW_HEIGHT)
            }
            SettingsItem::SectionHeader { .. } => 36.0,
            SettingsItem::GroupStart => 0.0,
            SettingsItem::GroupEnd => 16.0,
            SettingsItem::CenterLink { .. } => 40.0,
            SettingsItem::CenterText { .. } => 35.0,
            SettingsItem::CenterImage { height, .. } => *height,
            SettingsItem::Spacer { height } => *height,
            SettingsItem::Custom { height } => *height,
            SettingsItem::WidgetPreview { height } => *height,
            SettingsItem::RowFolderPicker { current_path, .. } => {
                if current_path.as_ref().is_some_and(|p| !p.is_empty()) {
                    64.0
                } else {
                    ROW_HEIGHT
                }
            }
            _ => ROW_HEIGHT,
        }
    }

    pub fn is_row(&self) -> bool {
        matches!(
            self,
            SettingsItem::RowStepper { .. }
                | SettingsItem::RowSwitch { .. }
                | SettingsItem::RowFontPicker { .. }
                | SettingsItem::RowFolderPicker { .. }
                | SettingsItem::RowSourceSelect { .. }
                | SettingsItem::RowButton { .. }
                | SettingsItem::RowTextField { .. }
                | SettingsItem::RowAppItem { .. }
                | SettingsItem::RowLabel { .. }
        )
    }
}
