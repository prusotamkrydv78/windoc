use super::*;

fn at(seconds: u64) -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(seconds)
}

fn path(value: &str) -> PathBuf {
    PathBuf::from(value)
}

#[test]
fn reference_shelf_ignores_copies_and_dedupes_originals() {
    let mut store = ShelfStore::new();
    let shelf = store
        .create_shelf(" Project ", ShelfMode::Reference)
        .expect("shelf");
    assert_eq!(store.shelf(shelf).expect("shelf").name, "Project");
    let (first, cleanup) = store
        .add(shelf, path("C:/a.txt"), Some(path("ignored")), at(0))
        .expect("add");
    assert!(cleanup.managed_copies.is_empty());
    assert_eq!(
        store.shelf(shelf).expect("shelf").entries()[0].managed_copy,
        None
    );
    let (again, cleanup) = store
        .add(shelf, path("C:/a.txt"), None, at(1))
        .expect("add");
    assert_eq!(again, first);
    assert!(cleanup.managed_copies.is_empty());
    assert_eq!(store.shelf(shelf).expect("shelf").entries().len(), 1);
}

#[test]
fn copying_shelves_require_a_copy_and_return_redundant_ones() {
    let mut store = ShelfStore::new();
    let shelf = store
        .create_shelf("Copies", ShelfMode::ManagedCopy)
        .expect("shelf");
    assert_eq!(
        store.add(shelf, path("a.txt"), None, at(0)),
        Err(ShelfError::CopyRequired)
    );
    store
        .add(shelf, path("a.txt"), Some(path("copy1")), at(0))
        .expect("add");
    let (_, cleanup) = store
        .add(shelf, path("a.txt"), Some(path("copy2")), at(1))
        .expect("add");
    assert_eq!(cleanup.managed_copies, vec![path("copy2")]);
    let entry = &store.shelf(shelf).expect("shelf").entries()[0];
    assert_eq!(entry.path(), Path::new("copy1"));
    assert_eq!(entry.display_name(), "a.txt");
}

#[test]
fn removal_returns_only_managed_copies() {
    let mut store = ShelfStore::new();
    let reference = store
        .create_shelf("Refs", ShelfMode::Reference)
        .expect("shelf");
    let managed = store
        .create_shelf("Managed", ShelfMode::ManagedCopy)
        .expect("shelf");
    let (ref_entry, _) = store
        .add(reference, path("orig"), None, at(0))
        .expect("add");
    let (managed_entry, _) = store
        .add(managed, path("orig2"), Some(path("copy")), at(0))
        .expect("add");
    assert!(
        store
            .remove(reference, ref_entry)
            .expect("remove")
            .managed_copies
            .is_empty()
    );
    assert_eq!(
        store
            .remove(managed, managed_entry)
            .expect("remove")
            .managed_copies,
        vec![path("copy")]
    );
    assert_eq!(
        store.remove(managed, managed_entry),
        Err(ShelfError::EntryNotFound)
    );
}

#[test]
fn deleting_a_shelf_cleans_up_its_copies() {
    let mut store = ShelfStore::new();
    let shelf = store
        .create_shelf("Temp", ShelfMode::ManagedCopy)
        .expect("shelf");
    store
        .add(shelf, path("a"), Some(path("ca")), at(0))
        .expect("add");
    store
        .add(shelf, path("b"), Some(path("cb")), at(0))
        .expect("add");
    let cleanup = store.delete_shelf(shelf).expect("delete");
    assert_eq!(cleanup.managed_copies, vec![path("ca"), path("cb")]);
    assert!(store.shelves().is_empty());
    assert_eq!(store.delete_shelf(shelf), Err(ShelfError::ShelfNotFound));
}

#[test]
fn ephemeral_entries_expire_at_retention() {
    let mut store = ShelfStore::new();
    let shelf = store
        .create_shelf(
            "Today",
            ShelfMode::Ephemeral {
                retention: Duration::from_secs(100),
            },
        )
        .expect("shelf");
    store
        .add(shelf, path("old"), Some(path("c-old")), at(0))
        .expect("add");
    store
        .add(shelf, path("new"), Some(path("c-new")), at(50))
        .expect("add");
    assert_eq!(store.shelf(shelf).expect("shelf").expired(at(100)).len(), 1);
    let cleanup = store.prune_expired(at(100));
    assert_eq!(cleanup.managed_copies, vec![path("c-old")]);
    let entries = store.shelf(shelf).expect("shelf").entries();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].original, path("new"));
    // A clock that moved backwards never expires anything.
    assert!(store.prune_expired(at(10)).managed_copies.is_empty());
}

#[test]
fn missing_files_are_reported_but_kept() {
    let mut store = ShelfStore::new();
    let shelf = store
        .create_shelf("Refs", ShelfMode::Reference)
        .expect("shelf");
    store.add(shelf, path("here"), None, at(0)).expect("add");
    store.add(shelf, path("gone"), None, at(0)).expect("add");
    let missing = store.missing(|candidate| candidate == Path::new("here"));
    assert_eq!(missing.len(), 1);
    assert_eq!(missing[0].1.original, path("gone"));
    assert_eq!(store.shelf(shelf).expect("shelf").entries().len(), 2);
}

#[test]
fn names_and_limits_are_validated() {
    let mut store = ShelfStore::new();
    assert_eq!(
        store.create_shelf("   ", ShelfMode::Reference),
        Err(ShelfError::InvalidName)
    );
    let long = "x".repeat(MAX_SHELF_NAME_CHARS + 1);
    assert_eq!(
        store.create_shelf(&long, ShelfMode::Reference),
        Err(ShelfError::InvalidName)
    );
    for index in 0..MAX_SHELVES {
        store
            .create_shelf(&format!("S{index}"), ShelfMode::Reference)
            .expect("shelf");
    }
    assert_eq!(
        store.create_shelf("one more", ShelfMode::Reference),
        Err(ShelfError::TooManyShelves)
    );
    let first = store.shelves()[0].id;
    store.rename_shelf(first, "Renamed").expect("rename");
    assert_eq!(store.shelf(first).expect("shelf").name, "Renamed");
}

#[test]
fn snapshot_round_trip_repairs_next_id() {
    let mut store = ShelfStore::new();
    let shelf = store
        .create_shelf("Refs", ShelfMode::Reference)
        .expect("shelf");
    store.add(shelf, path("a"), None, at(0)).expect("add");
    let mut snapshot = store.snapshot();
    let json = serde_json::to_string(&snapshot).expect("serialize");
    let decoded: ShelfSnapshot = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(decoded, snapshot);

    snapshot.next_id = 1;
    let mut restored = ShelfStore::restore(snapshot).expect("restore");
    // Ids 1 (shelf) and 2 (entry) exist, so the next id must be 3.
    let (id, _) = restored.add(shelf, path("b"), None, at(1)).expect("add");
    assert_eq!(id, 3);

    let mut wrong = restored.snapshot();
    wrong.version = 7;
    assert_eq!(
        ShelfStore::restore(wrong).err(),
        Some(ShelfError::UnsupportedVersion(7))
    );
}
