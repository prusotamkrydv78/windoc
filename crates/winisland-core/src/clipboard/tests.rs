use super::*;

const DAY: Duration = Duration::from_secs(24 * 60 * 60);

fn t0() -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000)
}

fn enabled() -> ClipboardSettings {
    ClipboardSettings {
        enabled: true,
        ..ClipboardSettings::default()
    }
}

fn text(value: &str) -> ClipboardBody {
    ClipboardBody::Text(value.to_string())
}

fn add(history: &mut ClipboardHistory, body: ClipboardBody, now: SystemTime) -> Capture {
    history.capture(body, &[], None, now)
}

fn bodies(history: &ClipboardHistory) -> Vec<ClipboardBody> {
    history
        .items()
        .iter()
        .map(|item| item.body.clone())
        .collect()
}

#[test]
fn monitoring_is_off_by_default_and_disabled_capture_is_a_no_op() {
    let mut history = ClipboardHistory::new(ClipboardSettings::default());
    assert_eq!(
        add(&mut history, text("a"), t0()).outcome,
        CaptureOutcome::Disabled
    );
    assert!(history.items().is_empty());

    history.set_enabled(true);
    assert!(matches!(
        add(&mut history, text("a"), t0()).outcome,
        CaptureOutcome::Added(_)
    ));
    history.set_enabled(false);
    assert_eq!(
        add(&mut history, text("b"), t0()).outcome,
        CaptureOutcome::Disabled
    );
    assert_eq!(history.items().len(), 1, "disabling keeps existing history");
}

#[test]
fn password_manager_markers_exclude_capture() {
    let mut history = ClipboardHistory::new(enabled());
    let cases = [
        (EXCLUDE_FROM_MONITORING_FORMAT, None, true),
        (CAN_INCLUDE_IN_HISTORY_FORMAT, Some(0), true),
        (CAN_UPLOAD_TO_CLOUD_FORMAT, Some(0), true),
        (CAN_INCLUDE_IN_HISTORY_FORMAT, Some(1), false),
        (CAN_UPLOAD_TO_CLOUD_FORMAT, None, false),
        ("CF_UNICODETEXT", None, false),
    ];
    for (index, (name, value, excluded)) in cases.into_iter().enumerate() {
        let formats = [
            SourceFormat {
                name: "CF_TEXT",
                value: None,
            },
            SourceFormat { name, value },
        ];
        assert_eq!(is_excluded(&formats), excluded, "{name} {value:?}");
        let outcome = history
            .capture(text(&format!("item {index}")), &formats, None, t0())
            .outcome;
        assert_eq!(
            outcome == CaptureOutcome::Excluded,
            excluded,
            "{name} {value:?}"
        );
    }
    // Three of the six cases were accepted.
    assert_eq!(history.items().len(), 3);
}

#[test]
fn consecutive_duplicates_are_suppressed() {
    let mut history = ClipboardHistory::new(enabled());
    add(&mut history, text("a"), t0());
    assert_eq!(
        add(&mut history, text("a"), t0()).outcome,
        CaptureOutcome::Duplicate
    );
    add(&mut history, text("b"), t0());
    // Not consecutive any more, so "a" is captured again.
    assert!(matches!(
        add(&mut history, text("a"), t0()).outcome,
        CaptureOutcome::Added(_)
    ));
    assert_eq!(bodies(&history), [text("a"), text("b"), text("a")]);
}

#[test]
fn link_detection_accepts_only_single_http_urls() {
    let link = |value: &str| ClipboardBody::Link(value.to_string());
    assert_eq!(
        ClipboardBody::from_text("https://example.com/a"),
        link("https://example.com/a")
    );
    assert_eq!(
        ClipboardBody::from_text("  http://a.b/c \n"),
        link("http://a.b/c")
    );
    assert_eq!(
        ClipboardBody::from_text("HTTPS://EXAMPLE.COM"),
        link("HTTPS://EXAMPLE.COM")
    );
    for not_link in [
        "see https://example.com",
        "ftp://example.com",
        "https://",
        "example.com",
        "é",
    ] {
        assert_eq!(
            ClipboardBody::from_text(not_link),
            text(not_link),
            "{not_link}"
        );
    }
}

#[test]
fn empty_bodies_are_ignored() {
    let mut history = ClipboardHistory::new(enabled());
    for body in [
        text(""),
        text("  \n\t"),
        ClipboardBody::Files(Vec::new()),
        ClipboardBody::Image {
            width: 0,
            height: 10,
            blob_id: "x".into(),
        },
    ] {
        assert_eq!(add(&mut history, body, t0()).outcome, CaptureOutcome::Empty);
    }
    assert!(history.items().is_empty());
}

#[test]
fn items_are_newest_first_with_increasing_ids() {
    let mut history = ClipboardHistory::new(enabled());
    let first = add(&mut history, text("a"), t0()).outcome;
    let second = add(&mut history, text("b"), t0()).outcome;
    assert_eq!(first, CaptureOutcome::Added(1));
    assert_eq!(second, CaptureOutcome::Added(2));
    assert_eq!(history.items()[0].id, 2);
    assert_eq!(history.get(1).map(|item| &item.body), Some(&text("a")));
}

#[test]
fn pin_and_delete() {
    let mut history = ClipboardHistory::new(enabled());
    add(&mut history, text("a"), t0());
    assert!(history.set_pinned(1, true));
    assert!(history.items()[0].pinned);
    assert!(!history.set_pinned(99, true));
    assert_eq!(history.delete(1).map(|item| item.body), Some(text("a")));
    assert_eq!(history.delete(1), None);
    assert!(history.items().is_empty());
}

#[test]
fn clear_unpinned_keeps_pins() {
    let mut history = ClipboardHistory::new(enabled());
    add(&mut history, text("a"), t0());
    add(&mut history, text("b"), t0());
    history.set_pinned(1, true);
    let removed = history.clear_unpinned();
    assert_eq!(removed.len(), 1);
    assert_eq!(bodies(&history), [text("a")]);
}

#[test]
fn search_is_case_insensitive_across_fields() {
    let mut history = ClipboardHistory::new(enabled());
    history.capture(text("Invoice 42"), &[], Some("Mail".into()), t0());
    history.capture(
        ClipboardBody::Files(vec![PathBuf::from(r"C:\Docs\Report.pdf")]),
        &[],
        None,
        t0(),
    );
    history.capture(
        ClipboardBody::from_text("https://Example.com"),
        &[],
        None,
        t0(),
    );

    let ids =
        |query: &str| -> Vec<u64> { history.search(query).iter().map(|item| item.id).collect() };
    assert_eq!(ids("invoice"), [1]);
    assert_eq!(ids("report.PDF"), [2]);
    assert_eq!(ids("example"), [3]);
    assert_eq!(ids("mail"), [1]);
    assert_eq!(ids("  "), [3, 2, 1]);
    assert!(ids("missing").is_empty());
}

#[test]
fn retention_removes_old_unpinned_items_only() {
    let mut history = ClipboardHistory::new(enabled());
    add(&mut history, text("old"), t0());
    add(&mut history, text("old pinned"), t0());
    history.set_pinned(2, true);
    let later = t0() + DEFAULT_RETENTION + Duration::from_secs(1);
    let capture = add(&mut history, text("new"), later);
    assert_eq!(capture.evicted.len(), 1);
    assert_eq!(capture.evicted[0].body, text("old"));
    assert_eq!(bodies(&history), [text("new"), text("old pinned")]);

    // Exactly at the retention age the item is still kept.
    let mut history = ClipboardHistory::new(enabled());
    add(&mut history, text("edge"), t0());
    assert!(history.enforce_limits(t0() + DEFAULT_RETENTION).is_empty());
}

#[test]
fn clock_moving_backwards_keeps_items() {
    let mut history = ClipboardHistory::new(enabled());
    add(&mut history, text("future"), t0() + DAY);
    assert!(history.enforce_limits(t0()).is_empty());
    assert_eq!(history.items().len(), 1);
}

#[test]
fn count_cap_evicts_oldest_unpinned_and_exempts_pins() {
    let settings = ClipboardSettings {
        max_items: 2,
        ..enabled()
    };
    let mut history = ClipboardHistory::new(settings);
    add(&mut history, text("pinned"), t0());
    history.set_pinned(1, true);
    add(&mut history, text("a"), t0());
    add(&mut history, text("b"), t0());
    let capture = add(&mut history, text("c"), t0());
    assert_eq!(capture.evicted.len(), 1);
    assert_eq!(capture.evicted[0].body, text("a"));
    assert_eq!(bodies(&history), [text("c"), text("b"), text("pinned")]);
}

#[test]
fn size_cap_evicts_until_under_limit() {
    let settings = ClipboardSettings {
        max_total_bytes: 10,
        ..enabled()
    };
    let mut history = ClipboardHistory::new(settings);
    add(&mut history, text("aaaaaa"), t0());
    let capture = add(&mut history, text("bbbbbb"), t0());
    assert_eq!(capture.evicted.len(), 1);
    assert_eq!(bodies(&history), [text("bbbbbb")]);

    // A single item larger than the cap is evicted immediately.
    let capture = add(&mut history, text("cccccccccccc"), t0());
    assert_eq!(capture.evicted.len(), 2);
    assert!(history.items().is_empty());
}

#[test]
fn image_size_counts_pixels() {
    let image = ClipboardBody::Image {
        width: 10,
        height: 20,
        blob_id: "blob".into(),
    };
    assert_eq!(image.size_bytes(), 800);
    let huge = ClipboardBody::Image {
        width: u32::MAX,
        height: u32::MAX,
        blob_id: "blob".into(),
    };
    assert_eq!(huge.size_bytes(), u64::MAX);
}

#[test]
fn shrinking_settings_applies_limits() {
    let mut history = ClipboardHistory::new(enabled());
    for value in ["a", "b", "c"] {
        add(&mut history, text(value), t0());
    }
    let evicted = history.set_settings(
        ClipboardSettings {
            max_items: 1,
            ..enabled()
        },
        t0(),
    );
    assert_eq!(evicted.len(), 2);
    assert_eq!(bodies(&history), [text("c")]);
}

#[test]
fn snapshot_round_trips_and_continues_ids() {
    let mut history = ClipboardHistory::new(enabled());
    add(&mut history, text("a"), t0());
    add(
        &mut history,
        ClipboardBody::from_text("https://example.com"),
        t0(),
    );
    history.set_pinned(1, true);

    let json = serde_json::to_string(&history.snapshot()).expect("serialize");
    let snapshot: ClipboardSnapshot = serde_json::from_str(&json).expect("deserialize");
    let (mut restored, evicted) =
        ClipboardHistory::restore(snapshot, enabled(), t0()).expect("restore");
    assert!(evicted.is_empty());
    assert_eq!(restored.items(), history.items());
    assert_eq!(
        add(&mut restored, text("b"), t0()).outcome,
        CaptureOutcome::Added(3)
    );
}

#[test]
fn restore_rejects_unknown_versions_and_repairs_ids() {
    let snapshot = ClipboardSnapshot {
        version: SNAPSHOT_VERSION + 1,
        next_id: 1,
        items: Vec::new(),
    };
    assert_eq!(
        ClipboardHistory::restore(snapshot, enabled(), t0()).map(|_| ()),
        Err(SnapshotError::UnsupportedVersion(SNAPSHOT_VERSION + 1))
    );

    let item = ClipboardItem {
        id: 7,
        body: text("a"),
        source_app: None,
        captured_at: t0(),
        pinned: false,
    };
    let snapshot = ClipboardSnapshot {
        version: SNAPSHOT_VERSION,
        next_id: 1,
        items: vec![item],
    };
    let (mut restored, _) = ClipboardHistory::restore(snapshot, enabled(), t0()).expect("restore");
    assert_eq!(
        add(&mut restored, text("b"), t0()).outcome,
        CaptureOutcome::Added(8)
    );
}

#[test]
fn restore_applies_retention() {
    let mut history = ClipboardHistory::new(enabled());
    add(&mut history, text("a"), t0());
    let (restored, evicted) =
        ClipboardHistory::restore(history.snapshot(), enabled(), t0() + 8 * DAY).expect("restore");
    assert_eq!(evicted.len(), 1);
    assert!(restored.items().is_empty());
}
