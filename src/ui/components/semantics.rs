//! Accessibility-facing description of drawn components.
//!
//! These nodes mirror UI Automation concepts (control type, name, state, bounds) without
//! depending on Windows. The platform layer will translate them into UIA providers.

use winisland_render::Rect;

/// Maps onto a UI Automation control type.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Role {
    Button,
    Group,
    ListItem,
    Tab,
    TabList,
    Text,
}

/// Interaction state shared by every component. Each flag must look visually distinct.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct InteractionState {
    pub hovered: bool,
    pub pressed: bool,
    pub selected: bool,
    pub focused: bool,
    pub disabled: bool,
    pub dragging: bool,
}

#[derive(Clone, PartialEq, Debug)]
pub struct SemanticNode {
    pub role: Role,
    /// Accessible name. Must be non-empty for interactive roles.
    pub name: String,
    pub bounds: Rect,
    pub state: InteractionState,
    pub children: Vec<SemanticNode>,
}

impl SemanticNode {
    pub fn new(role: Role, name: impl Into<String>, bounds: Rect, state: InteractionState) -> Self {
        Self {
            role,
            name: name.into(),
            bounds,
            state,
            children: Vec::new(),
        }
    }

    pub fn with_children(mut self, children: Vec<SemanticNode>) -> Self {
        self.children = children;
        self
    }
}
