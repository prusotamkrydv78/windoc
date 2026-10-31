//! The active product theme, shared by every surface that draws with design tokens.

use std::cell::Cell;

use winisland_render::tokens::{SemanticColors, Theme};

thread_local! {
    static THEME: Cell<Theme> = const { Cell::new(Theme::PureBlack) };
}

/// Applies the configured theme key (`pure_black`, `ember`, `vivid` or `silver`).
pub fn set(key: &str) {
    THEME.with(|cell| cell.set(Theme::from_key(key)));
}

pub fn current() -> Theme {
    THEME.with(Cell::get)
}

pub fn colors() -> SemanticColors {
    SemanticColors::for_theme(current())
}
