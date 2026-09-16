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
//
// Recurrence (v2): an entry can repeat Daily, Weekly, or Yearly. A recurring
// entry is stored ONCE, on its start date - the map key is its anchor. A given
// day's list is then the entries stored on that date PLUS any recurring entries
// anchored earlier whose rule lands on that day (expand-on-read; nothing is
// duplicated on disk). Because one recurring entry shows on many days, its
// "done" and "notified" state can't be a single flag - it's per-occurrence,
// keyed by the date the occurrence falls on.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use jiff::civil::Date;
use serde::{Deserialize, Serialize};

/// How an entry repeats. `None` is a one-off on its stored date; the others
/// recur from the stored date forward, with the stored date as the anchor.
///
/// Monthly is deliberately absent: day-of-month recurrence has to answer "what
/// does the 31st do in February," and every answer (skip, clamp) surprises
/// someone. Daily/Weekly/Yearly have no such ambiguity. Yearly's only edge is
/// a Feb-29 anchor, which simply doesn't occur in non-leap years - unambiguous.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Repeat {
    #[default]
    None,
    Daily,
    Weekly,
    Yearly,
}

impl Repeat {
    /// Cycle to the next option, for the day view's tap-to-advance pill:
    /// None -> Daily -> Weekly -> Yearly -> None.
    pub fn next(self) -> Self {
        match self {
            Repeat::None => Repeat::Daily,
            Repeat::Daily => Repeat::Weekly,
            Repeat::Weekly => Repeat::Yearly,
            Repeat::Yearly => Repeat::None,
        }
    }

    /// Short label for the recurrence pill.
    pub fn label(self) -> &'static str {
        match self {
            Repeat::None => "Once",
            Repeat::Daily => "Daily",
            Repeat::Weekly => "Weekly",
            Repeat::Yearly => "Yearly",
        }
    }

    /// Whether an entry anchored at `start` and repeating by `self` has an
    /// occurrence on `date`. `None` occurs only on its exact stored date; the
    /// recurring variants occur on `start` and every matching day after it,
    /// never before. All arithmetic is on civil dates, so it's DST-agnostic.
    fn occurs_on(self, start: Date, date: Date) -> bool {
        match self {
            Repeat::None => date == start,
            // From the anchor forward, every day.
            Repeat::Daily => date >= start,
            // From the anchor forward, same weekday.
            Repeat::Weekly => date >= start && date.weekday() == start.weekday(),
            // From the anchor forward, same month and day-of-month. A Feb-29
            // anchor therefore lands only in leap years (Date::new rejects
            // Feb 29 in common years, so those years just don't match) - which
            // is the correct, unsurprising behavior.
            Repeat::Yearly => {
                date >= start && date.month() == start.month() && date.day() == start.day()
            }
        }
    }
}

/// One to-do line on a given day (its stored/anchor day, for recurring items).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    /// What the user typed.
    pub text: String,
    /// Ticked off or not. Used only by non-recurring entries; recurring ones
    /// track completion per-occurrence in `done_dates` instead.
    #[serde(default)]
    pub done: bool,
    /// Optional time this to-do is "due" at, as minutes since midnight
    /// (0-1439). `None` means no time and no reminder. The reminder fires a
    /// configurable lead before this time (see `due_reminders`).
    #[serde(default)]
    pub at_minute: Option<u16>,
    /// Whether this entry's reminder has already been sent. Used only by
    /// non-recurring entries; recurring ones track this per-occurrence in
    /// `notified_dates` instead. Persisted so a reboot doesn't re-fire.
    #[serde(default)]
    pub notified: bool,
    /// How this entry repeats. Absent in v1 files -> `None` on load, i.e. a
    /// plain one-off, so old data upgrades transparently.
    #[serde(default)]
    pub repeat: Repeat,
    /// For recurring entries: the occurrence dates (ISO strings) the user has
    /// ticked off. Empty/absent for non-recurring entries.
    #[serde(default)]
    pub done_dates: Vec<String>,
    /// For recurring entries: the occurrence dates whose reminder has already
    /// fired. Empty/absent for non-recurring entries.
    #[serde(default)]
    pub notified_dates: Vec<String>,
}

impl Entry {
    fn new(text: String, at_minute: Option<u16>, repeat: Repeat) -> Self {
        Self {
            text,
            done: false,
            at_minute,
            notified: false,
            repeat,
            done_dates: Vec::new(),
            notified_dates: Vec::new(),
        }
    }

    /// Is this entry considered done on `date`? For a one-off, its single
    /// `done` flag (the date is its own anchor). For a recurring entry, whether
    /// `date` is in its completed set.
    fn is_done_on(&self, date: Date) -> bool {
        match self.repeat {
            Repeat::None => self.done,
            _ => self.done_dates.iter().any(|d| d == &date.to_string()),
        }
    }
}

/// A single reminder ready to fire, handed to the app. Carries just enough to
/// write the notification and to mark the source occurrence sent.
#[derive(Debug, Clone)]
pub struct DueReminder {
    /// ISO date key of the day this entry is ANCHORED on (its map key).
    pub date: String,
    /// Index of the entry within that anchor day's list.
    pub index: usize,
    /// The occurrence date this reminder is for (ISO). Same as `date` for a
    /// one-off; the specific recurring day otherwise. This is what gets marked
    /// notified, so each occurrence fires once.
    pub occurrence: String,
    /// The entry's text, for the notification body.
    pub text: String,
    /// The time the entry is due at (minutes since midnight), for the body.
    pub at_minute: u16,
}

/// One entry as it appears on a particular day, after recurrence expansion.
///
/// The day view renders from these, not from `Entry` directly, because a day's
/// list mixes entries anchored on that day with recurring entries anchored
/// elsewhere. `source_date` + `source_index` point back to the stored `Entry`
/// so a toggle or delete on this row reaches the right place; `occurrence` is
/// the day being viewed, which is what per-occurrence completion keys on.
#[derive(Debug, Clone)]
pub struct DayItem {
    /// The entry text.
    pub text: String,
    /// Done state for THIS occurrence.
    pub done: bool,
    /// The entry's due time, if any.
    pub at_minute: Option<u16>,
    /// How the source entry repeats (so the row can show a repeat glyph).
    pub repeat: Repeat,
    /// ISO key of the day the source entry is stored under.
    pub source_date: String,
    /// Index of the source entry within its stored day.
    pub source_index: usize,
}

/// Every day's entries, keyed by ISO date string ("2026-09-13").
///
/// A `BTreeMap` keeps the file sorted by date on save, so a hand-inspection
/// reads top-to-bottom in calendar order. Days with no entries simply aren't
/// present - an empty list is never written, it's removed.
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
    /// rather than taking the applet down - the panel keeping its clock matters
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
    /// mid-write can't leave a half-written file where the real one was - the
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

    /// The entries visible on one day, after recurrence expansion, in a stable
    /// order: entries anchored on this day first (in stored order), then
    /// recurring entries from earlier days whose rule lands here (in date, then
    /// stored, order - which the BTreeMap iteration gives for free). Never
    /// `None`: a day with nothing yields an empty `Vec`.
    ///
    /// Returns owned `DayItem`s rather than a borrowed slice because the list is
    /// computed, not stored.
    pub fn day(&self, date: Date) -> Vec<DayItem> {
        let key = date.to_string();
        let mut out = Vec::new();

        // Pass 1: entries anchored on this exact day, recurring or not. These
        // read first so a day's "own" items stay at the top where the user put
        // them.
        if let Some(entries) = self.days.get(&key) {
            for (index, entry) in entries.iter().enumerate() {
                out.push(DayItem {
                    text: entry.text.clone(),
                    done: entry.is_done_on(date),
                    at_minute: entry.at_minute,
                    repeat: entry.repeat,
                    source_date: key.clone(),
                    source_index: index,
                });
            }
        }

        // Pass 2: recurring entries anchored on EARLIER days that recur onto
        // this one. Skipping the anchor day itself (handled in pass 1) avoids
        // showing a recurring entry twice on its start date.
        for (anchor_key, entries) in &self.days {
            if anchor_key == &key {
                continue;
            }
            let Ok(anchor) = anchor_key.parse::<Date>() else {
                continue;
            };
            // Only earlier anchors can recur forward onto `date`.
            if anchor >= date {
                continue;
            }
            for (index, entry) in entries.iter().enumerate() {
                if entry.repeat == Repeat::None {
                    continue;
                }
                if entry.repeat.occurs_on(anchor, date) {
                    out.push(DayItem {
                        text: entry.text.clone(),
                        done: entry.is_done_on(date),
                        at_minute: entry.at_minute,
                        repeat: entry.repeat,
                        source_date: anchor_key.clone(),
                        source_index: index,
                    });
                }
            }
        }

        out
    }

    /// Whether a day shows any entries - this is what the calendar dot keys off.
    /// Mirrors `day()`'s expansion: a stored entry on this day, or a recurring
    /// entry from earlier that lands here. Short-circuits on the first match.
    pub fn has_entries(&self, date: Date) -> bool {
        let key = date.to_string();
        if self.days.get(&key).is_some_and(|v| !v.is_empty()) {
            return true;
        }
        for (anchor_key, entries) in &self.days {
            if anchor_key == &key {
                continue;
            }
            let Ok(anchor) = anchor_key.parse::<Date>() else {
                continue;
            };
            if anchor >= date {
                continue;
            }
            if entries
                .iter()
                .any(|e| e.repeat != Repeat::None && e.repeat.occurs_on(anchor, date))
            {
                return true;
            }
        }
        false
    }

    /// Add a line to a day, optionally tagged with a time (minutes since
    /// midnight) and a repeat rule. The day passed is the entry's anchor/start
    /// date. Blank input is ignored so an empty text box plus Enter doesn't
    /// create a phantom entry.
    pub fn add(&mut self, date: Date, text: String, at_minute: Option<u16>, repeat: Repeat) {
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        self.days
            .entry(date.to_string())
            .or_default()
            .push(Entry::new(text.to_owned(), at_minute, repeat));
        self.save();
    }

    /// Toggle the done state of one occurrence. `anchor` is the entry's stored
    /// day (its map key), `index` its position there, and `occurrence` the day
    /// actually being viewed. For a one-off, flips the single `done`. For a
    /// recurring entry, adds/removes `occurrence` from its completed set, so
    /// ticking this Monday leaves next Monday open. Out-of-range is a no-op.
    pub fn toggle(&mut self, anchor: Date, index: usize, occurrence: Date) {
        let Some(entries) = self.days.get_mut(&anchor.to_string()) else {
            return;
        };
        let Some(entry) = entries.get_mut(index) else {
            return;
        };
        match entry.repeat {
            Repeat::None => entry.done = !entry.done,
            _ => {
                let occ = occurrence.to_string();
                if let Some(pos) = entry.done_dates.iter().position(|d| d == &occ) {
                    entry.done_dates.remove(pos);
                } else {
                    entry.done_dates.push(occ);
                }
            }
        }
        self.save();
    }

    /// Remove one entry by its anchor day and position. For a recurring entry
    /// this deletes the WHOLE series (every future occurrence goes with it),
    /// which is the agreed behavior - skip-this-day isn't offered. When the
    /// last entry of a day goes, the day's key is dropped so the file never
    /// accrues empty lists (and the calendar dot clears on its own).
    pub fn remove(&mut self, anchor: Date, index: usize) {
        let key = anchor.to_string();
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

    /// Every reminder ready to fire as of `now`: an occurrence not done, not yet
    /// notified, whose reminder moment has arrived. The moment is `lead_minutes`
    /// before the occurrence's time - the entry's explicit `at_minute`, or
    /// `default_minute` when it has none.
    ///
    /// For recurring entries this only ever considers TODAY's occurrence: a
    /// weekly to-do should remind you this week, not fire a backlog of every
    /// past week at once. One-offs keep the original "no upper bound" behavior,
    /// so a one-off whose time passed while the machine was off still fires on
    /// the next check. Firing once is enforced per-occurrence via `notified` /
    /// `notified_dates`.
    pub fn due_reminders(
        &self,
        now: &jiff::Zoned,
        lead_minutes: u16,
        default_minute: u16,
    ) -> Vec<DueReminder> {
        let today = now.date();
        let mut out = Vec::new();
        for (key, entries) in &self.days {
            let Ok(anchor) = key.parse::<Date>() else {
                continue;
            };
            for (index, entry) in entries.iter().enumerate() {
                if entry.done {
                    // Only meaningful for one-offs; recurring entries leave this
                    // false and gate per-occurrence below.
                    continue;
                }
                let at_minute = entry.at_minute.unwrap_or(default_minute);

                match entry.repeat {
                    Repeat::None => {
                        // One-off: as before. Anchor must be today or past.
                        if anchor > today || entry.notified {
                            continue;
                        }
                        if reminder_reached(now, anchor, at_minute, lead_minutes) {
                            out.push(DueReminder {
                                date: key.clone(),
                                index,
                                occurrence: key.clone(),
                                text: entry.text.clone(),
                                at_minute,
                            });
                        }
                    }
                    _ => {
                        // Recurring: consider only today's occurrence, if there
                        // is one, and only if this day isn't already done or
                        // notified.
                        if !entry.repeat.occurs_on(anchor, today) {
                            continue;
                        }
                        let occ = today.to_string();
                        if entry.done_dates.contains(&occ)
                            || entry.notified_dates.contains(&occ)
                        {
                            continue;
                        }
                        if reminder_reached(now, today, at_minute, lead_minutes) {
                            out.push(DueReminder {
                                date: key.clone(),
                                index,
                                occurrence: occ,
                                text: entry.text.clone(),
                                at_minute,
                            });
                        }
                    }
                }
            }
        }
        out
    }

    /// Mark one occurrence's reminder as sent, and persist. Called by the app
    /// after `notify::send` so it never repeats. For a one-off, sets `notified`;
    /// for a recurring entry, records the occurrence date in `notified_dates`.
    /// `anchor` is the entry's map key, `occurrence` the day the reminder was
    /// for (equal to `anchor` for a one-off).
    pub fn mark_notified(&mut self, anchor: &str, index: usize, occurrence: &str) {
        let Some(entries) = self.days.get_mut(anchor) else {
            return;
        };
        let Some(entry) = entries.get_mut(index) else {
            return;
        };
        match entry.repeat {
            Repeat::None => {
                if !entry.notified {
                    entry.notified = true;
                    self.save();
                }
            }
            _ => {
                if !entry.notified_dates.iter().any(|d| d == occurrence) {
                    entry.notified_dates.push(occurrence.to_owned());
                    self.save();
                }
            }
        }
    }
}

/// Whether `now` has reached the reminder moment for an entry due at
/// `at_minute` (minutes since midnight) on `date` - `lead_minutes` before that
/// time. Built in `now`'s time zone with checked arithmetic; anything
/// unbuildable yields "not reached" rather than firing spuriously or panicking.
fn reminder_reached(now: &jiff::Zoned, date: Date, at_minute: u16, lead_minutes: u16) -> bool {
    use jiff::ToSpan;
    let hour = (at_minute / 60) as i8;
    let minute = (at_minute % 60) as i8;
    let due_civil = date.at(hour, minute, 0, 0);
    let Ok(due_zoned) = due_civil.to_zoned(now.time_zone().clone()) else {
        return false;
    };
    let Ok(reminder_at) = due_zoned.checked_sub(i64::from(lead_minutes).minutes()) else {
        return false;
    };
    *now >= reminder_at
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