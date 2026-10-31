//! File Shelf: named temporary shelves for dragging files in and out (workflow §5).
//!
//! Each shelf decides how it holds files:
//! - `Reference`: points at the original; a missing original is shown, never recreated.
//! - `ManagedCopy`: the app keeps its own copy under its data directory.
//! - `Ephemeral`: a managed copy that expires after the shelf's retention period.
//!
//! This module only models shelves. Copying, deleting managed copies and checking whether
//! files exist belong to the caller; [`Shelf::expired`] and removal results tell it which
//! managed copies to delete. Originals are never returned for deletion.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};

pub const SNAPSHOT_VERSION: u32 = 1;
pub const MAX_SHELVES: usize = 32;
pub const MAX_ENTRIES_PER_SHELF: usize = 500;
pub const MAX_SHELF_NAME_CHARS: usize = 60;
pub const DEFAULT_EPHEMERAL_RETENTION: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ShelfMode {
    Reference,
    ManagedCopy,
    Ephemeral { retention: Duration },
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ShelfEntry {
    pub id: u64,
    /// Where the file came from. Shown to the user; never deleted by the shelf.
    pub original: PathBuf,
    /// The app-owned copy for managed and ephemeral shelves.
    pub managed_copy: Option<PathBuf>,
    pub added_at: SystemTime,
}

impl ShelfEntry {
    /// The path to drag out or open.
    pub fn path(&self) -> &Path {
        self.managed_copy.as_deref().unwrap_or(&self.original)
    }

    pub fn display_name(&self) -> String {
        self.original
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.original.display().to_string())
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Shelf {
    pub id: u64,
    pub name: String,
    pub mode: ShelfMode,
    entries: Vec<ShelfEntry>,
}

impl Shelf {
    pub fn entries(&self) -> &[ShelfEntry] {
        &self.entries
    }

    /// Entries of an ephemeral shelf whose retention has passed at `now`.
    pub fn expired(&self, now: SystemTime) -> Vec<&ShelfEntry> {
        let ShelfMode::Ephemeral { retention } = self.mode else {
            return Vec::new();
        };
        self.entries
            .iter()
            .filter(|entry| {
                now.duration_since(entry.added_at)
                    .is_ok_and(|age| age >= retention)
            })
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShelfError {
    ShelfNotFound,
    EntryNotFound,
    TooManyShelves,
    TooManyEntries,
    InvalidName,
    /// A managed or ephemeral shelf needs the caller to supply the copy's path.
    CopyRequired,
    UnsupportedVersion(u32),
}

impl std::fmt::Display for ShelfError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ShelfNotFound => write!(f, "shelf not found"),
            Self::EntryNotFound => write!(f, "shelf entry not found"),
            Self::TooManyShelves => write!(f, "too many shelves"),
            Self::TooManyEntries => write!(f, "shelf is full"),
            Self::InvalidName => write!(f, "invalid shelf name"),
            Self::CopyRequired => write!(f, "this shelf keeps copies; a copy path is required"),
            Self::UnsupportedVersion(version) => {
                write!(f, "unsupported shelf snapshot version {version}")
            }
        }
    }
}

impl std::error::Error for ShelfError {}

/// What the caller must clean up after a removal: only app-owned copies, never originals.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Cleanup {
    pub managed_copies: Vec<PathBuf>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ShelfSnapshot {
    pub version: u32,
    pub next_id: u64,
    pub shelves: Vec<Shelf>,
}

#[derive(Clone, Debug)]
pub struct ShelfStore {
    next_id: u64,
    shelves: Vec<Shelf>,
}

fn normalize_name(name: &str) -> Result<String, ShelfError> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > MAX_SHELF_NAME_CHARS {
        return Err(ShelfError::InvalidName);
    }
    Ok(name.to_string())
}

impl Default for ShelfStore {
    fn default() -> Self {
        Self::new()
    }
}

impl ShelfStore {
    pub fn new() -> Self {
        Self {
            next_id: 1,
            shelves: Vec::new(),
        }
    }

    fn allocate_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    pub fn shelves(&self) -> &[Shelf] {
        &self.shelves
    }

    pub fn shelf(&self, id: u64) -> Option<&Shelf> {
        self.shelves.iter().find(|shelf| shelf.id == id)
    }

    fn shelf_mut(&mut self, id: u64) -> Result<&mut Shelf, ShelfError> {
        self.shelves
            .iter_mut()
            .find(|shelf| shelf.id == id)
            .ok_or(ShelfError::ShelfNotFound)
    }

    pub fn create_shelf(&mut self, name: &str, mode: ShelfMode) -> Result<u64, ShelfError> {
        let name = normalize_name(name)?;
        if self.shelves.len() >= MAX_SHELVES {
            return Err(ShelfError::TooManyShelves);
        }
        let id = self.allocate_id();
        self.shelves.push(Shelf {
            id,
            name,
            mode,
            entries: Vec::new(),
        });
        Ok(id)
    }

    pub fn rename_shelf(&mut self, id: u64, name: &str) -> Result<(), ShelfError> {
        let name = normalize_name(name)?;
        self.shelf_mut(id)?.name = name;
        Ok(())
    }

    /// Adds a file. `managed_copy` is required for managed and ephemeral shelves and
    /// ignored for reference shelves. Adding the same original twice returns the existing
    /// entry's id (and, for copying shelves, the redundant new copy as cleanup).
    pub fn add(
        &mut self,
        shelf_id: u64,
        original: PathBuf,
        managed_copy: Option<PathBuf>,
        now: SystemTime,
    ) -> Result<(u64, Cleanup), ShelfError> {
        let mode = self.shelf(shelf_id).ok_or(ShelfError::ShelfNotFound)?.mode;
        let managed_copy = match mode {
            ShelfMode::Reference => None,
            ShelfMode::ManagedCopy | ShelfMode::Ephemeral { .. } => {
                Some(managed_copy.ok_or(ShelfError::CopyRequired)?)
            }
        };
        if let Some(existing) = self.shelf(shelf_id).and_then(|shelf| {
            shelf
                .entries
                .iter()
                .find(|entry| entry.original == original)
        }) {
            let cleanup = Cleanup {
                managed_copies: managed_copy.into_iter().collect(),
            };
            return Ok((existing.id, cleanup));
        }
        if self
            .shelf(shelf_id)
            .is_some_and(|shelf| shelf.entries.len() >= MAX_ENTRIES_PER_SHELF)
        {
            return Err(ShelfError::TooManyEntries);
        }
        let id = self.allocate_id();
        self.shelf_mut(shelf_id)?.entries.push(ShelfEntry {
            id,
            original,
            managed_copy,
            added_at: now,
        });
        Ok((id, Cleanup::default()))
    }

    pub fn remove(&mut self, shelf_id: u64, entry_id: u64) -> Result<Cleanup, ShelfError> {
        let shelf = self.shelf_mut(shelf_id)?;
        let index = shelf
            .entries
            .iter()
            .position(|entry| entry.id == entry_id)
            .ok_or(ShelfError::EntryNotFound)?;
        let entry = shelf.entries.remove(index);
        Ok(Cleanup {
            managed_copies: entry.managed_copy.into_iter().collect(),
        })
    }

    /// Deletes a shelf. Its managed copies are returned for cleanup; originals are kept.
    pub fn delete_shelf(&mut self, shelf_id: u64) -> Result<Cleanup, ShelfError> {
        let index = self
            .shelves
            .iter()
            .position(|shelf| shelf.id == shelf_id)
            .ok_or(ShelfError::ShelfNotFound)?;
        let shelf = self.shelves.remove(index);
        Ok(Cleanup {
            managed_copies: shelf
                .entries
                .into_iter()
                .filter_map(|entry| entry.managed_copy)
                .collect(),
        })
    }

    /// Removes expired ephemeral entries from every shelf.
    pub fn prune_expired(&mut self, now: SystemTime) -> Cleanup {
        let mut cleanup = Cleanup::default();
        for shelf in &mut self.shelves {
            let ShelfMode::Ephemeral { retention } = shelf.mode else {
                continue;
            };
            shelf.entries.retain(|entry| {
                let expired = now
                    .duration_since(entry.added_at)
                    .is_ok_and(|age| age >= retention);
                if expired && let Some(copy) = &entry.managed_copy {
                    cleanup.managed_copies.push(copy.clone());
                }
                !expired
            });
        }
        cleanup
    }

    /// Entries whose file is gone, according to the caller's `exists` check. They stay on
    /// the shelf so the user can see what moved.
    pub fn missing<'a>(&'a self, exists: impl Fn(&Path) -> bool) -> Vec<(u64, &'a ShelfEntry)> {
        self.shelves
            .iter()
            .flat_map(|shelf| shelf.entries.iter().map(move |entry| (shelf.id, entry)))
            .filter(|(_, entry)| !exists(entry.path()))
            .collect()
    }

    pub fn snapshot(&self) -> ShelfSnapshot {
        ShelfSnapshot {
            version: SNAPSHOT_VERSION,
            next_id: self.next_id,
            shelves: self.shelves.clone(),
        }
    }

    pub fn restore(snapshot: ShelfSnapshot) -> Result<Self, ShelfError> {
        if snapshot.version != SNAPSHOT_VERSION {
            return Err(ShelfError::UnsupportedVersion(snapshot.version));
        }
        let max_id = snapshot
            .shelves
            .iter()
            .flat_map(|shelf| {
                std::iter::once(shelf.id).chain(shelf.entries.iter().map(|entry| entry.id))
            })
            .max()
            .unwrap_or(0);
        Ok(Self {
            next_id: snapshot.next_id.max(max_id + 1),
            shelves: snapshot.shelves,
        })
    }
}

#[cfg(test)]
mod tests;
