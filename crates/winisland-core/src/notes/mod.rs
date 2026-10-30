//! Notes and checklists: an in-memory store with search, revisions and Markdown export.
//!
//! Optimized for capturing and retrieving small pieces of information quickly, not for
//! long documents. Timestamps are supplied by the caller so behavior is deterministic.
//! Persistence is the caller's job: [`NoteStore::snapshot`] produces a serializable value
//! and [`NoteStore::restore`] validates one. Every limit returns a [`NotesError`]; nothing
//! here panics on user input.

use std::cmp::Reverse;
use std::fmt;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

pub const SNAPSHOT_VERSION: u32 = 1;
pub const MAX_NOTES: usize = 5_000;
pub const MAX_BODY_BYTES: usize = 256 * 1024;
pub const MAX_TAGS: usize = 16;
pub const MAX_TAG_CHARS: usize = 40;
pub const MAX_CHECKLIST_ITEMS: usize = 200;
pub const MAX_ITEM_CHARS: usize = 500;
/// Earlier bodies kept per note, oldest dropped first.
pub const MAX_REVISIONS: usize = 20;
pub const MAX_TITLE_CHARS: usize = 120;
/// Heading used in Markdown export when a note has no text yet.
pub const UNTITLED: &str = "Untitled";

pub type NoteId = u64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NotesError {
    NotFound(NoteId),
    TooManyNotes,
    BodyTooLarge,
    TooManyTags,
    /// Empty after normalization, or longer than [`MAX_TAG_CHARS`].
    InvalidTag,
    TooManyItems,
    /// Empty after trimming, or longer than [`MAX_ITEM_CHARS`].
    InvalidItem,
    ItemNotFound(usize),
    UnsupportedVersion(u32),
    /// Duplicate ids or limits exceeded in a restored snapshot.
    CorruptSnapshot,
}

impl fmt::Display for NotesError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound(id) => write!(f, "note {id} does not exist"),
            Self::TooManyNotes => write!(f, "note limit of {MAX_NOTES} reached"),
            Self::BodyTooLarge => write!(f, "note body exceeds {MAX_BODY_BYTES} bytes"),
            Self::TooManyTags => write!(f, "a note can have at most {MAX_TAGS} tags"),
            Self::InvalidTag => write!(f, "tag is empty or too long"),
            Self::TooManyItems => {
                write!(
                    f,
                    "a note can have at most {MAX_CHECKLIST_ITEMS} checklist items"
                )
            }
            Self::InvalidItem => write!(f, "checklist item is empty or too long"),
            Self::ItemNotFound(index) => write!(f, "checklist item {index} does not exist"),
            Self::UnsupportedVersion(version) => {
                write!(f, "notes snapshot version {version} is not supported")
            }
            Self::CorruptSnapshot => write!(f, "notes snapshot is inconsistent"),
        }
    }
}

impl std::error::Error for NotesError {}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ChecklistItem {
    pub text: String,
    pub done: bool,
}

/// A previous body, kept so edits can be reviewed or undone.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Revision {
    pub body: String,
    /// When this body was last saved, before it was replaced.
    pub saved_at: SystemTime,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Note {
    id: NoteId,
    title: String,
    body: String,
    checklist: Vec<ChecklistItem>,
    tags: Vec<String>,
    pinned: bool,
    created: SystemTime,
    updated: SystemTime,
    revisions: Vec<Revision>,
}

impl Note {
    pub fn id(&self) -> NoteId {
        self.id
    }

    /// First non-empty line of the body without Markdown heading markers; may be empty.
    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn body(&self) -> &str {
        &self.body
    }

    pub fn checklist(&self) -> &[ChecklistItem] {
        &self.checklist
    }

    /// Lowercase, without a leading `#`, in insertion order.
    pub fn tags(&self) -> &[String] {
        &self.tags
    }

    pub fn pinned(&self) -> bool {
        self.pinned
    }

    pub fn created(&self) -> SystemTime {
        self.created
    }

    pub fn updated(&self) -> SystemTime {
        self.updated
    }

    /// Oldest first.
    pub fn revisions(&self) -> &[Revision] {
        &self.revisions
    }

    /// Markdown export: title heading, tag line, remaining body, then the checklist.
    pub fn to_markdown(&self) -> String {
        let title = if self.title.is_empty() {
            UNTITLED
        } else {
            &self.title
        };
        let mut out = format!("# {title}\n");
        if !self.tags.is_empty() {
            let tags: Vec<String> = self.tags.iter().map(|tag| format!("#{tag}")).collect();
            out.push_str(&format!("\nTags: {}\n", tags.join(" ")));
        }
        let rest = body_after_title(&self.body).trim();
        if !rest.is_empty() {
            out.push('\n');
            out.push_str(rest);
            out.push('\n');
        }
        if !self.checklist.is_empty() {
            out.push('\n');
            for item in &self.checklist {
                let mark = if item.done { 'x' } else { ' ' };
                out.push_str(&format!("- [{mark}] {}\n", item.text));
            }
        }
        out
    }

    fn touch(&mut self, now: SystemTime) {
        // Never move `updated` backwards when the wall clock does.
        if now > self.updated {
            self.updated = now;
        }
    }

    fn item_mut(&mut self, index: usize) -> Result<&mut ChecklistItem, NotesError> {
        self.checklist
            .get_mut(index)
            .ok_or(NotesError::ItemNotFound(index))
    }
}

fn derive_title(body: &str) -> String {
    let line = body
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default();
    line.trim_start_matches('#')
        .trim()
        .chars()
        .take(MAX_TITLE_CHARS)
        .collect()
}

/// The body without its first non-empty line, which already serves as the title.
fn body_after_title(body: &str) -> &str {
    let mut offset = 0;
    for line in body.split_inclusive('\n') {
        offset += line.len();
        if !line.trim().is_empty() {
            return &body[offset..];
        }
    }
    ""
}

fn normalize_tag(tag: &str) -> Result<String, NotesError> {
    let tag = tag.trim().trim_start_matches('#').trim().to_lowercase();
    if tag.is_empty() || tag.chars().count() > MAX_TAG_CHARS {
        return Err(NotesError::InvalidTag);
    }
    Ok(tag)
}

fn check_body(body: &str) -> Result<(), NotesError> {
    if body.len() > MAX_BODY_BYTES {
        return Err(NotesError::BodyTooLarge);
    }
    Ok(())
}

fn normalize_item(text: &str) -> Result<String, NotesError> {
    let text = text.trim();
    if text.is_empty() || text.chars().count() > MAX_ITEM_CHARS {
        return Err(NotesError::InvalidItem);
    }
    Ok(text.to_string())
}

/// Serializable form of a [`NoteStore`].
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct NotesSnapshot {
    pub version: u32,
    pub next_id: NoteId,
    pub notes: Vec<Note>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NoteStore {
    notes: Vec<Note>,
    next_id: NoteId,
}

impl Default for NoteStore {
    fn default() -> Self {
        Self::new()
    }
}

impl NoteStore {
    pub fn new() -> Self {
        Self {
            notes: Vec::new(),
            next_id: 1,
        }
    }

    pub fn len(&self) -> usize {
        self.notes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.notes.is_empty()
    }

    pub fn note(&self, id: NoteId) -> Option<&Note> {
        self.notes.iter().find(|note| note.id == id)
    }

    fn note_mut(&mut self, id: NoteId) -> Result<&mut Note, NotesError> {
        self.notes
            .iter_mut()
            .find(|note| note.id == id)
            .ok_or(NotesError::NotFound(id))
    }

    pub fn create(&mut self, body: &str, now: SystemTime) -> Result<NoteId, NotesError> {
        if self.notes.len() >= MAX_NOTES {
            return Err(NotesError::TooManyNotes);
        }
        check_body(body)?;
        let id = self.next_id;
        self.next_id += 1;
        self.notes.push(Note {
            id,
            title: derive_title(body),
            body: body.to_string(),
            checklist: Vec::new(),
            tags: Vec::new(),
            pinned: false,
            created: now,
            updated: now,
            revisions: Vec::new(),
        });
        Ok(id)
    }

    /// Replaces the body. Returns `false`, without recording a revision, when the text is
    /// unchanged.
    pub fn update_body(
        &mut self,
        id: NoteId,
        body: &str,
        now: SystemTime,
    ) -> Result<bool, NotesError> {
        check_body(body)?;
        let note = self.note_mut(id)?;
        if note.body == body {
            return Ok(false);
        }
        let previous = std::mem::replace(&mut note.body, body.to_string());
        note.revisions.push(Revision {
            body: previous,
            saved_at: note.updated,
        });
        if note.revisions.len() > MAX_REVISIONS {
            let excess = note.revisions.len() - MAX_REVISIONS;
            note.revisions.drain(..excess);
        }
        note.title = derive_title(body);
        note.touch(now);
        Ok(true)
    }

    /// Returns the new pinned state. Pinning does not change `updated`.
    pub fn toggle_pin(&mut self, id: NoteId) -> Result<bool, NotesError> {
        let note = self.note_mut(id)?;
        note.pinned = !note.pinned;
        Ok(note.pinned)
    }

    /// Returns `false` when the note already has the tag.
    pub fn add_tag(&mut self, id: NoteId, tag: &str) -> Result<bool, NotesError> {
        let tag = normalize_tag(tag)?;
        let note = self.note_mut(id)?;
        if note.tags.contains(&tag) {
            return Ok(false);
        }
        if note.tags.len() >= MAX_TAGS {
            return Err(NotesError::TooManyTags);
        }
        note.tags.push(tag);
        Ok(true)
    }

    /// Returns `false` when the note did not have the tag.
    pub fn remove_tag(&mut self, id: NoteId, tag: &str) -> Result<bool, NotesError> {
        let tag = normalize_tag(tag)?;
        let note = self.note_mut(id)?;
        let before = note.tags.len();
        note.tags.retain(|existing| *existing != tag);
        Ok(note.tags.len() != before)
    }

    /// Appends an unchecked item and returns its index.
    pub fn add_item(
        &mut self,
        id: NoteId,
        text: &str,
        now: SystemTime,
    ) -> Result<usize, NotesError> {
        let text = normalize_item(text)?;
        let note = self.note_mut(id)?;
        if note.checklist.len() >= MAX_CHECKLIST_ITEMS {
            return Err(NotesError::TooManyItems);
        }
        note.checklist.push(ChecklistItem { text, done: false });
        note.touch(now);
        Ok(note.checklist.len() - 1)
    }

    /// Returns the item's new `done` state.
    pub fn toggle_item(
        &mut self,
        id: NoteId,
        index: usize,
        now: SystemTime,
    ) -> Result<bool, NotesError> {
        let note = self.note_mut(id)?;
        let item = note.item_mut(index)?;
        item.done = !item.done;
        let done = item.done;
        note.touch(now);
        Ok(done)
    }

    pub fn remove_item(
        &mut self,
        id: NoteId,
        index: usize,
        now: SystemTime,
    ) -> Result<ChecklistItem, NotesError> {
        let note = self.note_mut(id)?;
        if index >= note.checklist.len() {
            return Err(NotesError::ItemNotFound(index));
        }
        let item = note.checklist.remove(index);
        note.touch(now);
        Ok(item)
    }

    pub fn delete(&mut self, id: NoteId) -> Result<Note, NotesError> {
        let index = self
            .notes
            .iter()
            .position(|note| note.id == id)
            .ok_or(NotesError::NotFound(id))?;
        Ok(self.notes.remove(index))
    }

    /// Pinned notes first, then most recently updated; ties fall back to newest id.
    pub fn list(&self) -> Vec<&Note> {
        let mut notes: Vec<&Note> = self.notes.iter().collect();
        notes.sort_by_key(|note| {
            (
                Reverse(note.pinned),
                Reverse(note.updated),
                Reverse(note.id),
            )
        });
        notes
    }

    /// Case-insensitive substring search. Title matches rank above tag matches, which rank
    /// above body or checklist matches; equal ranks keep [`NoteStore::list`] order. An
    /// empty query returns every note.
    pub fn search(&self, query: &str) -> Vec<&Note> {
        let query = query.trim().to_lowercase();
        if query.is_empty() {
            return self.list();
        }
        let mut ranked: Vec<(u8, &Note)> = self
            .list()
            .into_iter()
            .filter_map(|note| {
                let rank = if note.title.to_lowercase().contains(&query) {
                    3
                } else if note.tags.iter().any(|tag| tag.contains(&query)) {
                    2
                } else if note.body.to_lowercase().contains(&query)
                    || note
                        .checklist
                        .iter()
                        .any(|item| item.text.to_lowercase().contains(&query))
                {
                    1
                } else {
                    return None;
                };
                Some((rank, note))
            })
            .collect();
        // Stable sort keeps list order within a rank.
        ranked.sort_by_key(|(rank, _)| Reverse(*rank));
        ranked.into_iter().map(|(_, note)| note).collect()
    }

    pub fn export_markdown(&self, id: NoteId) -> Result<String, NotesError> {
        self.note(id)
            .map(Note::to_markdown)
            .ok_or(NotesError::NotFound(id))
    }

    pub fn snapshot(&self) -> NotesSnapshot {
        NotesSnapshot {
            version: SNAPSHOT_VERSION,
            next_id: self.next_id,
            notes: self.notes.clone(),
        }
    }

    /// Validates a snapshot before trusting it. Ids are never reused, even when the stored
    /// `next_id` is behind the largest id present.
    pub fn restore(snapshot: NotesSnapshot) -> Result<Self, NotesError> {
        if snapshot.version != SNAPSHOT_VERSION {
            return Err(NotesError::UnsupportedVersion(snapshot.version));
        }
        if snapshot.notes.len() > MAX_NOTES {
            return Err(NotesError::CorruptSnapshot);
        }
        let mut ids: Vec<NoteId> = snapshot.notes.iter().map(|note| note.id).collect();
        ids.sort_unstable();
        if ids.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(NotesError::CorruptSnapshot);
        }
        let over_limit = snapshot.notes.iter().any(|note| {
            note.body.len() > MAX_BODY_BYTES
                || note.tags.len() > MAX_TAGS
                || note.checklist.len() > MAX_CHECKLIST_ITEMS
        });
        if over_limit {
            return Err(NotesError::CorruptSnapshot);
        }
        let next_id = ids
            .last()
            .map_or(1, |max| max.saturating_add(1))
            .max(snapshot.next_id)
            .max(1);
        let mut notes = snapshot.notes;
        for note in &mut notes {
            // Derived data is recomputed so a hand-edited snapshot cannot desync it.
            note.title = derive_title(&note.body);
            if note.revisions.len() > MAX_REVISIONS {
                let excess = note.revisions.len() - MAX_REVISIONS;
                note.revisions.drain(..excess);
            }
        }
        Ok(Self { notes, next_id })
    }
}

#[cfg(test)]
mod tests;
