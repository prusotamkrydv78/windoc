//! Deterministic sample data for the first dashboard prototype.
//!
//! Values are fixed so screenshots and layout tests are stable. Real module snapshots
//! replace these one module at a time.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CardKind {
    ScreenTime,
    Weather,
    Focus,
    Clipboard,
    Notes,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SampleCard {
    pub kind: CardKind,
    pub title_key: &'static str,
    /// Language-neutral value text.
    pub value: &'static str,
    pub detail_key: &'static str,
    /// Grid columns this card spans; clamped to the available columns.
    pub column_span: usize,
}

pub const HOME_CARDS: [SampleCard; 5] = [
    SampleCard {
        kind: CardKind::ScreenTime,
        title_key: "card_screen_time",
        value: "3:12",
        detail_key: "card_screen_time_detail",
        column_span: 1,
    },
    SampleCard {
        kind: CardKind::Weather,
        title_key: "card_weather",
        value: "24°",
        detail_key: "card_weather_detail",
        column_span: 1,
    },
    SampleCard {
        kind: CardKind::Focus,
        title_key: "card_focus",
        value: "18:40",
        detail_key: "card_focus_detail",
        column_span: 2,
    },
    SampleCard {
        kind: CardKind::Clipboard,
        title_key: "card_clipboard",
        value: "12",
        detail_key: "card_clipboard_detail",
        column_span: 1,
    },
    SampleCard {
        kind: CardKind::Notes,
        title_key: "card_notes",
        value: "4",
        detail_key: "card_notes_detail",
        column_span: 1,
    },
];
