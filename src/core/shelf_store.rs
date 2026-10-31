//! The default File Shelf, holding references to dropped files in `shelf.json`.

use std::path::PathBuf;
use std::time::SystemTime;

use winisland_core::shelf::{ShelfMode, ShelfSnapshot, ShelfStore};

use super::json_store;

pub(crate) const FILE_NAME: &str = "shelf.json";
const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;

pub type ShelfRow = (u64, String, PathBuf, bool);

pub struct ShelfService {
    store: ShelfStore,
    default_shelf: u64,
    revision: u64,
}

impl ShelfService {
    pub fn load() -> Self {
        let mut store = match json_store::read::<ShelfSnapshot>(FILE_NAME, MAX_FILE_BYTES) {
            Ok(Some(snapshot)) => ShelfStore::restore(snapshot).unwrap_or_else(|error| {
                log::warn!("Ignoring saved shelf: {error}");
                ShelfStore::new()
            }),
            Ok(None) => ShelfStore::new(),
            Err(error) => {
                log::warn!("Ignoring saved shelf: {error}");
                ShelfStore::new()
            }
        };
        let default_shelf = match store.shelves().first() {
            Some(shelf) => shelf.id,
            None => store
                .create_shelf("Shelf", ShelfMode::Reference)
                .unwrap_or_default(),
        };
        Self {
            store,
            default_shelf,
            revision: 0,
        }
    }

    /// Adds a dropped file by reference. The original is never moved or copied.
    pub fn add(&mut self, path: PathBuf) {
        match self
            .store
            .add(self.default_shelf, path, None, SystemTime::now())
        {
            Ok(_) => self.changed(),
            Err(error) => log::warn!("Cannot add file to shelf: {error}"),
        }
    }

    pub fn remove(&mut self, entry_id: u64) {
        if self.store.remove(self.default_shelf, entry_id).is_ok() {
            self.changed();
        }
    }

    fn changed(&mut self) {
        self.revision = self.revision.wrapping_add(1);
        if let Err(error) = json_store::write(FILE_NAME, &self.store.snapshot()) {
            log::warn!("Cannot save shelf: {error}");
        }
    }

    /// `(entry id, file name, path, exists)` for the default shelf.
    pub fn entries(&self) -> Vec<ShelfRow> {
        self.store
            .shelf(self.default_shelf)
            .map(|shelf| {
                shelf
                    .entries()
                    .iter()
                    .map(|entry| {
                        let path = entry.path().to_path_buf();
                        let exists = path.exists();
                        (entry.id, entry.display_name(), path, exists)
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }
}
