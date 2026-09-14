// GPL-3.0-or-later - see LICENSE file for full terms
// Copyright 2026 Michael Van Auker (HMRDSmoke)
// Do not remove these comments.
// VoidWatcher/src/store.rs
// src/store.rs
// On-disk to-do storage: one JSON file, entries grouped by calendar date.
//
// This is Void Watcher's OWN data, so unlike the read-only time config it owns
// both the format and the file. Serde does the (de)serialization; jiff's `Date`
// round-trips through its ISO string (2026-09-13), which is also what we use as
// the map key so the file stays greppable by eye.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use jiff::civil::Date;
use serde::{Deserialize, Serialize};

/// One to-do line on a given day.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    /// What the user typed.
    pub text: String,
    /// Ticked off or not.
    #[serde(default)]
    pub done: bool,
}

impl Entry {
    fn new(text: String) -> Self {
        Self { text, done: false }
    }
}

/// Every day's entries, keyed by ISO date string ("2026-09-13").
///
/// A `BTreeMap` keeps the file sorted by date on save, so a hand-inspection
/// reads top-to-bottom in calendar order. Days with no entries simply aren't
/// present — an empty list is never written, it's removed.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Store {
    days: BTreeMap<String, Vec<Entry>>,
}

impl Store {
    /// Load the to-do file, or start empty.
    ///
    /// A missing file is the first-run case, not an error. A file that fails to
    /// parse (hand-edited into invalid JSON, say) also falls back to empty
    /// rather than taking the applet down — the panel keeping its clock matters
    /// more than surfacing a parse error nobody asked for.
    pub fn load() -> Self {
        let Some(path) = data_path() else {
            return Self::default();
        };
        match std::fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    /// Write the file back to disk, creating the directory on first save.
    ///
    /// Writes to a sibling temp file and renames over the target, so a crash
    /// mid-write can't leave a half-written file where the real one was — the
    /// rename either happens or it doesn't. A failed save is swallowed on
    /// purpose: losing one edit is better than a panic in the panel process.
    pub fn save(&self) {
        let Some(path) = data_path() else {
            return;
        };
        if let Some(dir) = path.parent() {
            if std::fs::create_dir_all(dir).is_err() {
                return;
            }
        }
        let Ok(text) = serde_json::to_string_pretty(self) else {
            return;
        };
        write_atomic(&path, text.as_bytes());
    }

    /// The entries for one day, in the order they were added. Never `None`:
    /// a day the user hasn't touched just yields an empty slice.
    pub fn day(&self, date: Date) -> &[Entry] {
        self.days.get(&date.to_string()).map_or(&[], Vec::as_slice)
    }

    /// Whether a day has any entries — this is what the calendar dot keys off.
    pub fn has_entries(&self, date: Date) -> bool {
        self.days.get(&date.to_string()).is_some_and(|v| !v.is_empty())
    }

    /// Add a line to a day. Blank input is ignored so an empty text box plus
    /// Enter doesn't create a phantom entry.
    pub fn add(&mut self, date: Date, text: String) {
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        self.days
            .entry(date.to_string())
            .or_default()
            .push(Entry::new(text.to_owned()));
        self.save();
    }

    /// Flip the done flag on one entry of a day, addressed by its position in
    /// that day's list. Out-of-range indices are a no-op.
    pub fn toggle(&mut self, date: Date, index: usize) {
        if let Some(entries) = self.days.get_mut(&date.to_string()) {
            if let Some(entry) = entries.get_mut(index) {
                entry.done = !entry.done;
                self.save();
            }
        }
    }

    /// Remove one entry of a day by position. When the last entry of a day
    /// goes, the day's key is dropped too, so the file never accrues empty
    /// lists (and the calendar dot clears on its own).
    pub fn remove(&mut self, date: Date, index: usize) {
        let key = date.to_string();
        if let Some(entries) = self.days.get_mut(&key) {
            if index < entries.len() {
                entries.remove(index);
                if entries.is_empty() {
                    self.days.remove(&key);
                }
                self.save();
            }
        }
    }
}

/// `$XDG_DATA_HOME/void-watcher/todos.json` (falling back to `~/.local/share`).
fn data_path() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join("void-watcher").join("todos.json"))
}

/// Write `bytes` to `path` via a temp-file-and-rename, so readers never see a
/// partially written file.
fn write_atomic(path: &Path, bytes: &[u8]) {
    let tmp = path.with_extension("json.tmp");
    if std::fs::write(&tmp, bytes).is_ok() {
        let _ = std::fs::rename(&tmp, path);
    }
}
