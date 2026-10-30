use super::*;
use std::time::Duration;

fn at(seconds: u64) -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(seconds)
}

fn ids(notes: &[&Note]) -> Vec<NoteId> {
    notes.iter().map(|note| note.id()).collect()
}

#[test]
fn create_derives_title_and_assigns_increasing_ids() {
    let mut store = NoteStore::new();
    let first = store
        .create("\n  # Groceries  \nmilk", at(10))
        .expect("create");
    let second = store.create("", at(11)).expect("create");
    assert_eq!((first, second), (1, 2));
    let note = store.note(first).expect("note");
    assert_eq!(note.title(), "Groceries");
    assert_eq!(note.created(), at(10));
    assert_eq!(store.note(second).expect("note").title(), "");
}

#[test]
fn title_is_truncated() {
    let mut store = NoteStore::new();
    let long = "a".repeat(MAX_TITLE_CHARS + 30);
    let id = store.create(&long, at(1)).expect("create");
    assert_eq!(
        store.note(id).expect("note").title().chars().count(),
        MAX_TITLE_CHARS
    );
}

#[test]
fn update_records_revision_only_on_change() {
    let mut store = NoteStore::new();
    let id = store.create("one", at(1)).expect("create");
    assert!(!store.update_body(id, "one", at(2)).expect("update"));
    assert!(store.note(id).expect("note").revisions().is_empty());
    assert_eq!(store.note(id).expect("note").updated(), at(1));

    assert!(store.update_body(id, "two\nmore", at(3)).expect("update"));
    let note = store.note(id).expect("note");
    assert_eq!(note.title(), "two");
    assert_eq!(note.updated(), at(3));
    assert_eq!(
        note.revisions(),
        [Revision {
            body: "one".into(),
            saved_at: at(1),
        }]
    );
}

#[test]
fn revisions_are_bounded_oldest_first() {
    let mut store = NoteStore::new();
    let id = store.create("v0", at(0)).expect("create");
    for version in 1..=(MAX_REVISIONS as u64 + 5) {
        store
            .update_body(id, &format!("v{version}"), at(version))
            .expect("update");
    }
    // 25 updates produce bodies v0..v24 as revisions; the oldest five are dropped.
    let revisions = store.note(id).expect("note").revisions();
    assert_eq!(revisions.len(), MAX_REVISIONS);
    assert_eq!(revisions[0].body, "v5");
    assert_eq!(revisions[MAX_REVISIONS - 1].body, "v24");
}

#[test]
fn updated_never_moves_backwards() {
    let mut store = NoteStore::new();
    let id = store.create("a", at(100)).expect("create");
    store.update_body(id, "b", at(50)).expect("update");
    assert_eq!(store.note(id).expect("note").updated(), at(100));
}

#[test]
fn tags_are_normalized_deduplicated_and_limited() {
    let mut store = NoteStore::new();
    let id = store.create("x", at(1)).expect("create");
    assert!(store.add_tag(id, "  #Work ").expect("tag"));
    assert!(!store.add_tag(id, "work").expect("tag"));
    assert_eq!(store.note(id).expect("note").tags(), ["work"]);
    assert_eq!(store.add_tag(id, " # "), Err(NotesError::InvalidTag));
    assert_eq!(
        store.add_tag(id, &"t".repeat(MAX_TAG_CHARS + 1)),
        Err(NotesError::InvalidTag)
    );
    for index in 1..MAX_TAGS {
        store.add_tag(id, &format!("tag{index}")).expect("tag");
    }
    assert_eq!(store.add_tag(id, "overflow"), Err(NotesError::TooManyTags));
    // An existing tag is still accepted as a no-op at the limit.
    assert_eq!(store.add_tag(id, "WORK"), Ok(false));
    assert!(store.remove_tag(id, "#WORK").expect("remove"));
    assert!(!store.remove_tag(id, "work").expect("remove"));
}

#[test]
fn checklist_add_toggle_remove() {
    let mut store = NoteStore::new();
    let id = store.create("trip", at(1)).expect("create");
    assert_eq!(store.add_item(id, " passport ", at(2)), Ok(0));
    assert_eq!(store.add_item(id, "tickets", at(3)), Ok(1));
    assert_eq!(
        store.add_item(id, "   ", at(3)),
        Err(NotesError::InvalidItem)
    );
    assert_eq!(store.toggle_item(id, 0, at(4)), Ok(true));
    assert_eq!(
        store.toggle_item(id, 5, at(4)),
        Err(NotesError::ItemNotFound(5))
    );
    let removed = store.remove_item(id, 1, at(5)).expect("remove");
    assert_eq!(removed.text, "tickets");
    let note = store.note(id).expect("note");
    assert_eq!(
        note.checklist(),
        [ChecklistItem {
            text: "passport".into(),
            done: true,
        }]
    );
    assert_eq!(note.updated(), at(5));
    assert_eq!(
        store.remove_item(id, 1, at(6)),
        Err(NotesError::ItemNotFound(1))
    );
}

#[test]
fn checklist_limit() {
    let mut store = NoteStore::new();
    let id = store.create("long list", at(1)).expect("create");
    for index in 0..MAX_CHECKLIST_ITEMS {
        store
            .add_item(id, &format!("item {index}"), at(1))
            .expect("item");
    }
    assert_eq!(
        store.add_item(id, "one more", at(1)),
        Err(NotesError::TooManyItems)
    );
}

#[test]
fn body_and_note_limits() {
    let mut store = NoteStore::new();
    let big = "x".repeat(MAX_BODY_BYTES + 1);
    assert_eq!(store.create(&big, at(1)), Err(NotesError::BodyTooLarge));
    let id = store.create("ok", at(1)).expect("create");
    assert_eq!(
        store.update_body(id, &big, at(2)),
        Err(NotesError::BodyTooLarge)
    );
    assert_eq!(store.note(id).expect("note").body(), "ok");
    // Rejected creates do not consume ids.
    assert_eq!(id, 1);

    let mut full = NoteStore::new();
    for _ in 0..MAX_NOTES {
        full.create("n", at(1)).expect("create");
    }
    assert_eq!(full.create("n", at(1)), Err(NotesError::TooManyNotes));
}

#[test]
fn missing_notes_report_not_found() {
    let mut store = NoteStore::new();
    assert_eq!(
        store.update_body(9, "x", at(1)),
        Err(NotesError::NotFound(9))
    );
    assert_eq!(store.toggle_pin(9), Err(NotesError::NotFound(9)));
    assert_eq!(
        store.delete(9).map(|note| note.id()),
        Err(NotesError::NotFound(9))
    );
    assert_eq!(store.export_markdown(9), Err(NotesError::NotFound(9)));
}

#[test]
fn list_orders_pinned_then_recent() {
    let mut store = NoteStore::new();
    let old = store.create("old", at(1)).expect("create");
    let mid = store.create("mid", at(2)).expect("create");
    let new = store.create("new", at(3)).expect("create");
    let tie = store.create("tie", at(3)).expect("create");
    assert_eq!(ids(&store.list()), [tie, new, mid, old]);

    assert!(store.toggle_pin(old).expect("pin"));
    // Pinning does not touch `updated`.
    assert_eq!(store.note(old).expect("note").updated(), at(1));
    store
        .update_body(mid, "mid edited", at(10))
        .expect("update");
    assert_eq!(ids(&store.list()), [old, mid, tie, new]);

    let deleted = store.delete(tie).expect("delete");
    assert_eq!(deleted.title(), "tie");
    assert_eq!(store.len(), 3);
    // Ids are never reused after a delete.
    assert_eq!(store.create("next", at(11)), Ok(5));
}

#[test]
fn search_ranks_title_over_tag_over_body() {
    let mut store = NoteStore::new();
    let body = store
        .create("Shopping\nbuy a Plan notebook", at(4))
        .expect("create");
    let tag = store.create("Ideas", at(3)).expect("create");
    store.add_tag(tag, "plans").expect("tag");
    let title = store.create("Project plan", at(1)).expect("create");
    let item = store.create("Trip", at(5)).expect("create");
    store.add_item(item, "Planetarium", at(5)).expect("item");
    store.create("Unrelated", at(6)).expect("create");

    // Body and checklist matches share rank 1 and keep list order (item updated later).
    assert_eq!(ids(&store.search("  PLAN ")), [title, tag, item, body]);
    assert!(store.search("zzz").is_empty());
    assert_eq!(store.search("").len(), store.len());
}

#[test]
fn markdown_export() {
    let mut store = NoteStore::new();
    let id = store
        .create("# Weekend\n\nRelax.\nCall mum.\n", at(1))
        .expect("create");
    store.add_tag(id, "Home").expect("tag");
    store.add_tag(id, "family").expect("tag");
    store.add_item(id, "Laundry", at(2)).expect("item");
    store.add_item(id, "Groceries", at(2)).expect("item");
    store.toggle_item(id, 1, at(3)).expect("toggle");
    assert_eq!(
        store.export_markdown(id).expect("export"),
        "# Weekend\n\nTags: #home #family\n\nRelax.\nCall mum.\n\n- [ ] Laundry\n- [x] Groceries\n"
    );

    let empty = store.create("   ", at(4)).expect("create");
    assert_eq!(
        store.export_markdown(empty).expect("export"),
        "# Untitled\n"
    );
}

#[test]
fn snapshot_round_trip_through_json() {
    let mut store = NoteStore::new();
    let id = store.create("alpha", at(1)).expect("create");
    store.update_body(id, "beta", at(2)).expect("update");
    store.add_tag(id, "greek").expect("tag");
    store.add_item(id, "gamma", at(3)).expect("item");
    store.toggle_pin(id).expect("pin");

    let json = serde_json::to_string(&store.snapshot()).expect("serialize");
    let snapshot: NotesSnapshot = serde_json::from_str(&json).expect("deserialize");
    let restored = NoteStore::restore(snapshot).expect("restore");
    assert_eq!(restored, store);
}

#[test]
fn restore_rejects_bad_snapshots() {
    let mut store = NoteStore::new();
    store.create("a", at(1)).expect("create");
    store.create("b", at(2)).expect("create");

    let mut wrong_version = store.snapshot();
    wrong_version.version = SNAPSHOT_VERSION + 1;
    assert_eq!(
        NoteStore::restore(wrong_version),
        Err(NotesError::UnsupportedVersion(SNAPSHOT_VERSION + 1))
    );

    let mut duplicate = store.snapshot();
    let copy = duplicate.notes[0].clone();
    duplicate.notes.push(copy);
    assert_eq!(
        NoteStore::restore(duplicate),
        Err(NotesError::CorruptSnapshot)
    );

    let mut oversized = store.snapshot();
    oversized.notes[0].body = "x".repeat(MAX_BODY_BYTES + 1);
    assert_eq!(
        NoteStore::restore(oversized),
        Err(NotesError::CorruptSnapshot)
    );
}

#[test]
fn restore_repairs_next_id_and_title() {
    let mut store = NoteStore::new();
    store.create("first", at(1)).expect("create");
    store.create("second", at(2)).expect("create");
    let mut snapshot = store.snapshot();
    snapshot.next_id = 1;
    snapshot.notes[0].title = "tampered".into();
    let mut restored = NoteStore::restore(snapshot).expect("restore");
    assert_eq!(restored.note(1).expect("note").title(), "first");
    assert_eq!(restored.create("third", at(3)), Ok(3));
}
