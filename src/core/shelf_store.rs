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
    active_shelf: u64,
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
        let active_shelf = match store.shelves().first() {
            Some(shelf) => shelf.id,
            None => store
                .create_shelf("Shelf", ShelfMode::Reference)
                .unwrap_or_default(),
        };
        Self {
            store,
            active_shelf,
            revision: 0,
        }
    }

    /// Adds a dropped file by reference. The original is never moved or copied.
    pub fn add(&mut self, path: PathBuf) {
        match self
            .store
            .add(self.active_shelf, path, None, SystemTime::now())
        {
            Ok(_) => self.changed(),
            Err(error) => log::warn!("Cannot add file to shelf: {error}"),
        }
    }

    pub fn remove(&mut self, entry_id: u64) {
        if self.store.remove(self.active_shelf, entry_id).is_ok() {
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
            .shelf(self.active_shelf)
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

impl ShelfService {
    pub fn active_name(&self) -> String {
        self.store
            .shelf(self.active_shelf)
            .map(|shelf| shelf.name.clone())
            .unwrap_or_default()
    }

    /// Makes the next shelf active, wrapping around.
    pub fn cycle(&mut self) {
        let shelves = self.store.shelves();
        if let Some(position) = shelves
            .iter()
            .position(|shelf| shelf.id == self.active_shelf)
        {
            self.active_shelf = shelves[(position + 1) % shelves.len()].id;
            self.revision = self.revision.wrapping_add(1);
        }
    }

    pub fn create(&mut self, name: &str) {
        match self
            .store
            .create_shelf(name, winisland_core::shelf::ShelfMode::Reference)
        {
            Ok(id) => {
                self.active_shelf = id;
                self.changed();
            }
            Err(error) => log::warn!("Cannot create shelf: {error}"),
        }
    }

    /// Deletes a shelf (never the last one). Originals are untouched.
    pub fn delete(&mut self, id: u64) {
        if self.store.shelves().len() <= 1 {
            return;
        }
        if self.store.delete_shelf(id).is_ok() {
            if self.active_shelf == id {
                self.active_shelf = self.store.shelves().first().map_or(0, |shelf| shelf.id);
            }
            self.changed();
        }
    }
}

/// Shelf `(id, name)` pairs as saved on disk, for the settings window.
pub fn saved_shelves() -> Vec<(u64, String)> {
    match json_store::read::<ShelfSnapshot>(FILE_NAME, MAX_FILE_BYTES) {
        Ok(Some(snapshot)) => snapshot
            .shelves
            .iter()
            .map(|shelf| (shelf.id, shelf.name.clone()))
            .collect(),
        _ => Vec::new(),
    }
}

impl ShelfService {
    /// Paths of files on the active shelf that still exist.
    pub fn existing_paths(&self) -> Vec<PathBuf> {
        self.entries()
            .into_iter()
            .filter(|row| row.3)
            .map(|row| row.2)
            .collect()
    }
}
