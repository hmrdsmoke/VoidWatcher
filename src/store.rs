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
//
// Invites (v3): entries can also come from .ics files (see ics.rs), which is
// what makes Void Watcher a real choice as the system's default calendar. An
// imported entry carries the invite's identity - its UID and sequence number -
// so opening an UPDATE for the same meeting rewrites the existing entry instead
// of adding a twin, and a CANCEL removes it. Invites also bring rule details a
// typed to-do never had: an end date, every-Nth-period intervals, skipped
// occurrences, and two monthly shapes. All of it is optional on disk, so v1 and
// v2 files load untouched and a typed to-do still saves exactly as it did.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use jiff::civil::{Date, Weekday};
use serde::{Deserialize, Serialize};

/// How an entry repeats. `None` is a one-off on its stored date; the others
/// recur from the stored date forward, with the stored date as the anchor.
///
/// The two Monthly shapes are IMPORT-ONLY: the day view's pill still cycles
/// None -> Daily -> Weekly -> Yearly, so a typed to-do never gets one. They
/// exist because invites carry them, and the file's rule decides what they
/// mean. The 28/30/31 question that kept Monthly out of v2 is answered by the
/// iCalendar standard (RFC 5545): an occurrence that doesn't exist in a month
/// is skipped, so a "31st" meeting lands in Jan, Mar, May and misses Feb. We
/// follow that rule rather than invent our own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Repeat {
    #[default]
    None,
    Daily,
    Weekly,
    Yearly,
    /// Import-only: the same day-of-month each month (1-31). Months without
    /// that day are skipped.
    Monthly { day: i8 },
    /// Import-only: the nth `weekday` of each month - `nth` 1..=5 from the
    /// start, or -1 for the last (2 + Tuesday = "second Tuesday", -1 + Friday =
    /// "last Friday"). `weekday` is Monday=1 .. Sunday=7, stored as a number
    /// because jiff's `Weekday` has no serde form.
    MonthlyWeekday { nth: i8, weekday: i8 },
}

impl Repeat {
    /// Cycle to the next option, for the day view's tap-to-advance pill:
    /// None -> Daily -> Weekly -> Yearly -> None. The import-only Monthly
    /// shapes can't be reached from the pill; if one is ever stepped (it
    /// isn't today), it drops back to None.
    pub fn next(self) -> Self {
        match self {
            Repeat::None => Repeat::Daily,
            Repeat::Daily => Repeat::Weekly,
            Repeat::Weekly => Repeat::Yearly,
            Repeat::Yearly => Repeat::None,
            Repeat::Monthly { .. } | Repeat::MonthlyWeekday { .. } => Repeat::None,
        }
    }

    /// Short label for the recurrence pill.
    pub fn label(self) -> &'static str {
        match self {
            Repeat::None => "Once",
            Repeat::Daily => "Daily",
            Repeat::Weekly => "Weekly",
            Repeat::Yearly => "Yearly",
            Repeat::Monthly { .. } | Repeat::MonthlyWeekday { .. } => "Monthly",
        }
    }

    /// Whether an entry anchored at `start` and repeating by `self` every
    /// `interval` periods has an occurrence on `date`. `None` occurs only on
    /// its exact stored date; the recurring variants occur on `start` and
    /// every matching day after it, never before. `interval` 1 (or 0, treated
    /// as 1) is every period; 2 is every other week/month/year. All
    /// arithmetic is on civil dates, so it's DST-agnostic.
    pub fn occurs_on(self, start: Date, date: Date, interval: u8) -> bool {
        if date < start {
            return false;
        }
        match self {
            Repeat::None => date == start,
            // From the anchor forward, every `interval` days.
            Repeat::Daily => every((date - start).get_days(), interval),
            // From the anchor forward, same weekday, every `interval` weeks.
            Repeat::Weekly => {
                date.weekday() == start.weekday()
                    && every((date - start).get_days() / 7, interval)
            }
            // From the anchor forward, same month and day-of-month, every
            // `interval` years. A Feb-29 anchor therefore lands only in leap
            // years (there is no Feb 29 to match otherwise) - which is the
            // correct, unsurprising behavior.
            Repeat::Yearly => {
                date.month() == start.month()
                    && date.day() == start.day()
                    && every(i32::from(date.year()) - i32::from(start.year()), interval)
            }
            // Same day-of-month, every `interval` months. A month that has no
            // such day simply produces no matching date - the RFC 5545 skip.
            Repeat::Monthly { day } => {
                date.day() == day && every(months_between(start, date), interval)
            }
            // The nth weekday of the month, every `interval` months. jiff does
            // the "second Tuesday" / "last Friday" math; a month where that
            // doesn't exist (a fifth Monday, say) yields an error, i.e. no
            // occurrence.
            Repeat::MonthlyWeekday { nth, weekday } => {
                let Ok(weekday) = Weekday::from_monday_one_offset(weekday) else {
                    return false;
                };
                every(months_between(start, date), interval)
                    && date.nth_weekday_of_month(nth, weekday).is_ok_and(|d| d == date)
            }
        }
    }

    /// The last day of a recurrence that runs `steps` periods past `start`:
    /// the date of the (steps + 1)th occurrence, so one step from a Friday
    /// weekly is the next Friday. `None` for no steps, for a one-off, or for a
    /// rule with no further occurrence to land on. What the day view's Ends
    /// pill shows and what a typed to-do saves as `until`.
    pub fn end_after(self, start: Date, steps: u32) -> Option<Date> {
        if steps == 0 || self == Repeat::None {
            return None;
        }
        self.nth_occurrence(start, 1, steps + 1)
    }

    /// The date of the `n`th occurrence (1-based) of this rule from `start`,
    /// walking day by day. Used to turn an invite's "COUNT=6" into an end
    /// date. Bounded to a couple of centuries so a rule that never matches
    /// (a Monthly on day 31 with an interval that only ever lands on short
    /// months, say) gives `None` instead of spinning.
    pub fn nth_occurrence(self, start: Date, interval: u8, n: u32) -> Option<Date> {
        if n == 0 {
            return None;
        }
        let mut date = start;
        let mut seen = 0u32;
        for _ in 0..(200 * 366) {
            if self.occurs_on(start, date, interval) {
                seen += 1;
                if seen == n {
                    return Some(date);
                }
            }
            date = date.tomorrow().ok()?;
        }
        None
    }
}

/// `n` periods past the anchor is an occurrence when it's a whole number of
/// `interval`s. Interval 0 is nonsense from a hand-edited file; read it as 1.
fn every(n: i32, interval: u8) -> bool {
    n >= 0 && n % i32::from(interval.max(1)) == 0
}

/// Whole months from `start`'s month to `date`'s month (day-of-month ignored).
fn months_between(start: Date, date: Date) -> i32 {
    (i32::from(date.year()) - i32::from(start.year())) * 12
        + (i32::from(date.month()) - i32::from(start.month()))
}

/// Serde default for `Entry::interval`: every period.
fn default_interval() -> u8 {
    1
}

fn is_one(v: &u8) -> bool {
    *v == 1
}

fn is_zero(v: &u32) -> bool {
    *v == 0
}

/// One to-do line on a given day (its stored/anchor day, for recurring items).
///
/// The v3 invite fields all default and are skipped on save when they hold
/// their default, so a typed to-do writes exactly the same JSON it did in v2 -
/// the file only grows the extra keys on entries that actually came from an
/// invite or carry an end date.
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
    /// The invite's UID - the meeting's permanent identity across every
    /// version of it. `None` for a typed to-do. This is what lets an update
    /// find the entry it replaces and a cancel find the entry it removes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uid: Option<String>,
    /// The invite's SEQUENCE - its version counter. An incoming invite with a
    /// lower sequence than the stored one is stale and ignored.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub sequence: u32,
    /// Last day the recurrence applies, inclusive. `None` = forever. Set by an
    /// invite's UNTIL/COUNT, or by the day view's Ends pill.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until: Option<Date>,
    /// Occurrence dates that don't happen ("weekly, except the 25th"). From an
    /// invite's EXDATE, or from a single moved occurrence.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exdates: Vec<Date>,
    /// Recur every `interval` periods: 1 = every week, 2 = every other week.
    /// From an invite's INTERVAL. Always 1 for a typed to-do.
    #[serde(default = "default_interval", skip_serializing_if = "is_one")]
    pub interval: u8,
}

impl Entry {
    fn new(text: String, at_minute: Option<u16>, repeat: Repeat, until: Option<Date>) -> Self {
        Self {
            text,
            done: false,
            at_minute,
            notified: false,
            repeat,
            done_dates: Vec::new(),
            notified_dates: Vec::new(),
            uid: None,
            sequence: 0,
            until,
            exdates: Vec::new(),
            interval: 1,
        }
    }

    /// Build an entry from an imported invite item. Fresh completion and
    /// reminder state; the caller carries over what should survive an update.
    fn from_import(item: &Imported) -> Self {
        Self {
            text: item.text.clone(),
            done: item.done,
            at_minute: item.at_minute,
            notified: false,
            repeat: item.repeat,
            done_dates: Vec::new(),
            notified_dates: Vec::new(),
            uid: item.uid.clone(),
            sequence: item.sequence,
            until: item.until,
            exdates: item.exdates.clone(),
            interval: item.interval.max(1),
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

    /// Whether this RECURRING entry, anchored at `anchor`, has an occurrence
    /// on `date`: the rule says so, the end date (if any) hasn't passed, and
    /// the date isn't an excluded one. Always false for a one-off - those are
    /// found by their anchor key, not by expansion.
    fn recurs_on(&self, anchor: Date, date: Date) -> bool {
        if self.repeat == Repeat::None {
            return false;
        }
        if self.until.is_some_and(|until| date > until) {
            return false;
        }
        if self.exdates.contains(&date) {
            return false;
        }
        self.repeat.occurs_on(anchor, date, self.interval)
    }

    /// Whether this entry should show on its own anchor day. A one-off
    /// always does. A recurring entry does unless that first occurrence was
    /// excluded (an invite that moved the very first meeting does exactly
    /// that: EXDATE on the start date, one-off on the new one).
    fn shows_on_anchor(&self, anchor: Date) -> bool {
        self.repeat == Repeat::None || !self.exdates.contains(&anchor)
    }
}

/// One item handed to `Store::import` by the .ics parser. The parser turns a
/// file into these; the store decides what each one means against what's
/// already saved (add, replace, ignore, remove). Field names mirror `Entry`
/// where they overlap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Imported {
    /// The invite's UID, if it had one. Files without UIDs import as plain
    /// entries with duplicate protection instead of identity.
    pub uid: Option<String>,
    /// The invite's SEQUENCE (0 when absent).
    pub sequence: u32,
    /// The entry's date: the anchor for a recurring item, the day for a
    /// one-off.
    pub date: Date,
    /// The entry text (the invite's SUMMARY).
    pub text: String,
    /// Due time as minutes since midnight, local; `None` for an all-day item.
    pub at_minute: Option<u16>,
    pub repeat: Repeat,
    pub interval: u8,
    pub until: Option<Date>,
    pub exdates: Vec<Date>,
    /// Already completed (a VTODO with STATUS:COMPLETED).
    pub done: bool,
    /// This item cancels the series with its UID (METHOD:CANCEL or
    /// STATUS:CANCELLED) rather than adding anything.
    pub cancel: bool,
    /// The file's rule couldn't be expressed, so this is the meeting's first
    /// date as a one-off. Reported so the user knows.
    pub fallback: bool,
    /// For a single moved occurrence (RECURRENCE-ID): the parent series' UID
    /// and the original date to exclude from it. The item itself is the
    /// moved occurrence as a one-off.
    pub excludes_from: Option<(String, Date)>,
}

/// What `Store::import` did, in the entry texts, for the notification.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportReport {
    /// New entries.
    pub added: Vec<String>,
    /// Existing entries rewritten by a newer version of the same invite.
    pub updated: Vec<String>,
    /// Entries removed by a cancellation.
    pub cancelled: Vec<String>,
    /// Items imported as a one-off because their rule isn't supported.
    pub fallback: Vec<String>,
    /// Items that changed nothing: stale versions, exact duplicates, or
    /// cancellations for meetings that weren't stored.
    pub skipped: usize,
}

impl ImportReport {
    /// Whether anything on disk changed.
    pub fn changed(&self) -> bool {
        !self.added.is_empty() || !self.updated.is_empty() || !self.cancelled.is_empty()
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
    /// Whether the source entry came from an invite (so the row can show an
    /// invite glyph: this one follows the organizer's updates).
    pub imported: bool,
    /// ISO key of the day the source entry is stored under.
    pub source_date: String,
    /// Index of the source entry within its stored day.
    pub source_index: usize,
}

impl DayItem {
    fn from_entry(entry: &Entry, occurrence: Date, source_date: &str, source_index: usize) -> Self {
        Self {
            text: entry.text.clone(),
            done: entry.is_done_on(occurrence),
            at_minute: entry.at_minute,
            repeat: entry.repeat,
            imported: entry.uid.is_some(),
            source_date: source_date.to_owned(),
            source_index,
        }
    }
}

/// Every day's entries, keyed by ISO date string ("2026-09-13").
///
/// A `BTreeMap` keeps the file sorted by date on save, so a hand-inspection
/// reads top-to-bottom in calendar order. Days with no entries simply aren't
/// present - an empty list is never written, it's removed.
///
/// The file path rides along (not serialized) so `save` knows where to write.
/// A store built without one - `Store::default()`, which is what the tests
/// use - has nowhere to save and never touches disk. `seen_mtime` is the
/// file's timestamp as of our last read or write; when the file on disk is
/// newer than that, some other process wrote it (an .ics import runs as its
/// own short-lived process) and the applet reloads.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Store {
    days: BTreeMap<String, Vec<Entry>>,
    #[serde(skip)]
    path: Option<PathBuf>,
    #[serde(skip)]
    seen_mtime: Option<SystemTime>,
}

impl Store {
    /// Load the to-do file, or start empty.
    ///
    /// A missing file is the first-run case, not an error. A file that fails to
    /// parse (hand-edited into invalid JSON, say) also falls back to empty
    /// rather than taking the applet down - the panel keeping its clock matters
    /// more than surfacing a parse error nobody asked for.
    pub fn load() -> Self {
        let path = data_path();
        let mut store: Store = path
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        store.path = path;
        store.seen_mtime = store.mtime();
        store
    }

    /// Write the file back to disk, creating the directory on first save.
    ///
    /// Writes to a sibling temp file and renames over the target, so a crash
    /// mid-write can't leave a half-written file where the real one was - the
    /// rename either happens or it doesn't. A failed save is swallowed on
    /// purpose: losing one edit is better than a panic in the panel process.
    pub fn save(&mut self) {
        let Some(path) = &self.path else {
            return;
        };
        if let Some(dir) = path.parent()
            && std::fs::create_dir_all(dir).is_err()
        {
            return;
        }
        let Ok(text) = serde_json::to_string_pretty(self) else {
            return;
        };
        write_atomic(path, text.as_bytes());
        self.seen_mtime = self.mtime();
    }

    /// Whether the file on disk has been written since we last read or wrote
    /// it - by another process, since our own writes update `seen_mtime`. The
    /// applet asks this once a tick and reloads when it's true, which is how
    /// an .ics import shows up in the running panel without a restart.
    pub fn changed_on_disk(&self) -> bool {
        self.path.is_some() && self.mtime() != self.seen_mtime
    }

    /// When the to-do file was last written, or `None` if there isn't one.
    fn mtime(&self) -> Option<SystemTime> {
        let path = self.path.as_ref()?;
        std::fs::metadata(path).ok()?.modified().ok()
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
                if entry.shows_on_anchor(date) {
                    out.push(DayItem::from_entry(entry, date, &key, index));
                }
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
                if entry.recurs_on(anchor, date) {
                    out.push(DayItem::from_entry(entry, date, anchor_key, index));
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
        if self
            .days
            .get(&key)
            .is_some_and(|v| v.iter().any(|e| e.shows_on_anchor(date)))
        {
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
            if entries.iter().any(|e| e.recurs_on(anchor, date)) {
                return true;
            }
        }
        false
    }

    /// Add a line to a day, optionally tagged with a time (minutes since
    /// midnight), a repeat rule and the last day that rule applies. The day
    /// passed is the entry's anchor/start date. Blank input is ignored so an
    /// empty text box plus Enter doesn't create a phantom entry.
    pub fn add(
        &mut self,
        date: Date,
        text: String,
        at_minute: Option<u16>,
        repeat: Repeat,
        until: Option<Date>,
    ) {
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        self.days
            .entry(date.to_string())
            .or_default()
            .push(Entry::new(text.to_owned(), at_minute, repeat, until));
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
        if self.take(&anchor.to_string(), index).is_some() {
            self.save();
        }
    }

    /// Pull one entry out of the map by key and position, dropping the day's
    /// key if it was the last one. No save - the callers decide when to write.
    fn take(&mut self, key: &str, index: usize) -> Option<Entry> {
        let entries = self.days.get_mut(key)?;
        if index >= entries.len() {
            return None;
        }
        let entry = entries.remove(index);
        if entries.is_empty() {
            self.days.remove(key);
        }
        Some(entry)
    }

    /// Where the entry with this invite UID lives: its anchor key and index.
    fn find_uid(&self, uid: &str) -> Option<(String, usize)> {
        for (key, entries) in &self.days {
            if let Some(index) = entries.iter().position(|e| e.uid.as_deref() == Some(uid)) {
                return Some((key.clone(), index));
            }
        }
        None
    }

    /// Whether a day already holds an entry that reads the same - same text,
    /// same time. Duplicate protection for files that carry no UID, so opening
    /// the same invite twice doesn't double it.
    fn has_twin(&self, date: Date, text: &str, at_minute: Option<u16>) -> bool {
        self.days.get(&date.to_string()).is_some_and(|entries| {
            entries
                .iter()
                .any(|e| e.text == text && e.at_minute == at_minute)
        })
    }

    /// Apply a parsed .ics file. Each item is judged against what's stored:
    ///
    /// - a single moved occurrence first excludes its original date from the
    ///   parent series, then imports as its own one-off;
    /// - a cancellation removes the entry with that UID (the whole series) and
    ///   is otherwise a no-op;
    /// - a known UID with a sequence at least as new REPLACES the stored entry
    ///   in place (moving it to a new day if the date changed), keeping the
    ///   occurrences already ticked off; an older sequence is stale and
    ///   ignored;
    /// - anything else is added, unless the day already holds its twin.
    ///
    /// Saves once at the end if anything changed. Returns what happened, in
    /// entry texts, so the caller can say so in a notification.
    pub fn import(&mut self, items: &[Imported]) -> ImportReport {
        let mut report = ImportReport::default();
        let mut dirty = false;

        for item in items {
            // A moved or cancelled single occurrence: punch the hole in the
            // parent series first. Remember the parent's text - if this item
            // is a cancellation of just that occurrence, the hole IS the
            // change, and that's what gets reported.
            let mut punched: Option<String> = None;
            if let Some((parent_uid, original)) = &item.excludes_from
                && let Some((key, index)) = self.find_uid(parent_uid)
                && let Some(parent) = self.days.get_mut(&key).and_then(|v| v.get_mut(index))
                && !parent.exdates.contains(original)
            {
                parent.exdates.push(*original);
                punched = Some(parent.text.clone());
                dirty = true;
            }

            if item.cancel {
                let removed = item
                    .uid
                    .as_deref()
                    .and_then(|uid| self.find_uid(uid))
                    .and_then(|(key, index)| self.take(&key, index));
                match (removed, punched) {
                    (Some(entry), _) => report.cancelled.push(entry.text),
                    (None, Some(parent_text)) => report.cancelled.push(parent_text),
                    (None, None) => report.skipped += 1,
                }
                continue;
            }

            let existing = item.uid.as_deref().and_then(|uid| self.find_uid(uid));
            match existing {
                Some((key, index)) => {
                    let stored_sequence = self.days[&key][index].sequence;
                    if item.sequence < stored_sequence {
                        // Stale: an older version arriving after a newer one.
                        report.skipped += 1;
                        continue;
                    }
                    let Some(old) = self.take(&key, index) else {
                        report.skipped += 1;
                        continue;
                    };
                    let mut new = Entry::from_import(item);
                    // Completed occurrences survive an update; dates that no
                    // longer occur simply never match again. A one-off keeps
                    // its done/notified state only if it didn't move - a
                    // rescheduled meeting should remind you again.
                    new.done_dates = old.done_dates;
                    new.notified_dates = old.notified_dates;
                    let same_slot = old.at_minute == item.at_minute && key == item.date.to_string();
                    if same_slot && item.repeat == Repeat::None {
                        new.done = old.done || item.done;
                        new.notified = old.notified;
                    }
                    self.days.entry(item.date.to_string()).or_default().push(new);
                    report.updated.push(item.text.clone());
                }
                None => {
                    if item.uid.is_none() && self.has_twin(item.date, &item.text, item.at_minute) {
                        report.skipped += 1;
                        continue;
                    }
                    self.days
                        .entry(item.date.to_string())
                        .or_default()
                        .push(Entry::from_import(item));
                    report.added.push(item.text.clone());
                }
            }
            if item.fallback {
                report.fallback.push(item.text.clone());
            }
        }

        if dirty || report.changed() {
            self.save();
        }
        report
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
                        // notified. The anchor day itself counts too, unless
                        // that first occurrence was excluded.
                        let occurs_today = if anchor == today {
                            entry.shows_on_anchor(today)
                        } else {
                            entry.recurs_on(anchor, today)
                        };
                        if !occurs_today {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> Date {
        s.parse().unwrap()
    }

    /// A store that never touches disk: built in memory, saved nowhere.
    fn store() -> Store {
        Store::default()
    }

    fn item(uid: Option<&str>, seq: u32, date: &str, text: &str) -> Imported {
        Imported {
            uid: uid.map(str::to_owned),
            sequence: seq,
            date: d(date),
            text: text.to_owned(),
            at_minute: Some(600),
            repeat: Repeat::None,
            interval: 1,
            until: None,
            exdates: Vec::new(),
            done: false,
            cancel: false,
            fallback: false,
            excludes_from: None,
        }
    }

    #[test]
    fn v2_json_loads_and_saves_unchanged_shape() {
        // A v2 entry has no invite keys; it must come back byte-for-byte in
        // shape (the new keys stay out of the file while they're default).
        let json = r#"{"2026-09-25":[{"text":"Dentist","done":false,"at_minute":600,"notified":false,"repeat":"Weekly","done_dates":[],"notified_dates":[]}]}"#;
        let store: Store = serde_json::from_str(json).unwrap();
        let back = serde_json::to_string(&store).unwrap();
        assert_eq!(back, json);
        // And a v1 entry (no repeat at all) still loads as a one-off.
        let v1 = r#"{"2026-09-25":[{"text":"Old","done":true}]}"#;
        let store: Store = serde_json::from_str(v1).unwrap();
        assert_eq!(store.days["2026-09-25"][0].repeat, Repeat::None);
        assert_eq!(store.days["2026-09-25"][0].interval, 1);
    }

    #[test]
    fn monthly_variants_round_trip() {
        let e = Entry {
            repeat: Repeat::MonthlyWeekday { nth: 2, weekday: 2 },
            until: Some(d("2027-01-01")),
            exdates: vec![d("2026-10-13")],
            interval: 2,
            uid: Some("abc".into()),
            sequence: 3,
            ..Entry::new("Board".into(), None, Repeat::None, None)
        };
        let json = serde_json::to_string(&e).unwrap();
        let back: Entry = serde_json::from_str(&json).unwrap();
        assert_eq!(back, e);
    }

    #[test]
    fn weekly_every_other_week() {
        let start = d("2026-09-25"); // Friday
        assert!(Repeat::Weekly.occurs_on(start, d("2026-09-25"), 2));
        assert!(!Repeat::Weekly.occurs_on(start, d("2026-10-02"), 2));
        assert!(Repeat::Weekly.occurs_on(start, d("2026-10-09"), 2));
        assert!(!Repeat::Weekly.occurs_on(start, d("2026-09-18"), 2)); // before start
    }

    #[test]
    fn monthly_by_day_skips_short_months() {
        let start = d("2026-01-31");
        let r = Repeat::Monthly { day: 31 };
        assert!(r.occurs_on(start, d("2026-01-31"), 1));
        assert!(!r.occurs_on(start, d("2026-02-28"), 1));
        assert!(r.occurs_on(start, d("2026-03-31"), 1));
        assert!(!r.occurs_on(start, d("2026-04-30"), 1));
        assert!(r.occurs_on(start, d("2026-05-31"), 1));
    }

    #[test]
    fn monthly_second_tuesday_and_last_friday() {
        let start = d("2026-10-13"); // second Tuesday of Oct 2026
        let r = Repeat::MonthlyWeekday { nth: 2, weekday: 2 };
        assert!(r.occurs_on(start, d("2026-10-13"), 1));
        assert!(r.occurs_on(start, d("2026-11-10"), 1));
        assert!(!r.occurs_on(start, d("2026-11-03"), 1)); // first Tuesday
        assert!(!r.occurs_on(start, d("2026-11-17"), 1)); // third Tuesday
        let start = d("2026-10-30"); // last Friday of Oct 2026
        let r = Repeat::MonthlyWeekday { nth: -1, weekday: 5 };
        assert!(r.occurs_on(start, d("2026-10-30"), 1));
        assert!(r.occurs_on(start, d("2026-11-27"), 1));
        assert!(!r.occurs_on(start, d("2026-11-20"), 1));
    }

    #[test]
    fn end_after_steps_lands_on_real_occurrences() {
        let fri = d("2026-09-25");
        assert_eq!(Repeat::Weekly.end_after(fri, 0), None);
        assert_eq!(Repeat::None.end_after(fri, 3), None);
        assert_eq!(Repeat::Weekly.end_after(fri, 1), Some(d("2026-10-02")));
        assert_eq!(Repeat::Weekly.end_after(fri, 3), Some(d("2026-10-16")));
        assert_eq!(Repeat::Daily.end_after(fri, 2), Some(d("2026-09-27")));
        assert_eq!(Repeat::Yearly.end_after(d("2024-02-29"), 1), Some(d("2028-02-29")));
        let mut s = store();
        s.add(fri, "Standup".into(), None, Repeat::Weekly, Repeat::Weekly.end_after(fri, 2));
        assert!(s.has_entries(d("2026-10-09")));
        assert!(!s.has_entries(d("2026-10-16")));
    }

    #[test]
    fn nth_occurrence_gives_count_end_dates() {
        assert_eq!(Repeat::Weekly.nth_occurrence(d("2026-09-25"), 1, 6), Some(d("2026-10-30")));
        assert_eq!(Repeat::Daily.nth_occurrence(d("2026-09-25"), 1, 1), Some(d("2026-09-25")));
        assert_eq!(Repeat::Yearly.nth_occurrence(d("2024-02-29"), 1, 2), Some(d("2028-02-29")));
        assert_eq!(Repeat::Weekly.nth_occurrence(d("2026-09-25"), 1, 0), None);
    }

    #[test]
    fn until_and_exdates_are_honored() {
        let mut s = store();
        let mut it = item(Some("u1"), 0, "2026-09-25", "Standup");
        it.repeat = Repeat::Weekly;
        it.until = Some(d("2026-10-09"));
        it.exdates = vec![d("2026-10-02")];
        s.import(&[it]);
        assert_eq!(s.day(d("2026-09-25")).len(), 1);
        assert_eq!(s.day(d("2026-10-02")).len(), 0); // excluded
        assert_eq!(s.day(d("2026-10-09")).len(), 1); // last one
        assert_eq!(s.day(d("2026-10-16")).len(), 0); // past until
        assert!(s.has_entries(d("2026-10-09")));
        assert!(!s.has_entries(d("2026-10-16")));
    }

    #[test]
    fn update_replaces_and_stale_is_ignored() {
        let mut s = store();
        s.import(&[item(Some("m1"), 0, "2026-09-29", "Dentist")]);
        assert_eq!(s.day(d("2026-09-29")).len(), 1);

        // Moved to Thursday, sequence 1: Tuesday empties, Thursday fills.
        let r = s.import(&[item(Some("m1"), 1, "2026-10-01", "Dentist")]);
        assert_eq!(r.updated, vec!["Dentist"]);
        assert_eq!(s.day(d("2026-09-29")).len(), 0);
        assert_eq!(s.day(d("2026-10-01")).len(), 1);
        assert!(s.day(d("2026-10-01"))[0].imported);

        // The old sequence 0 arriving late changes nothing.
        let r = s.import(&[item(Some("m1"), 0, "2026-09-29", "Dentist")]);
        assert_eq!(r.skipped, 1);
        assert!(!r.changed());
        assert_eq!(s.day(d("2026-09-29")).len(), 0);
        assert_eq!(s.day(d("2026-10-01")).len(), 1);
    }

    #[test]
    fn update_keeps_done_when_slot_unchanged_and_resets_when_moved() {
        let mut s = store();
        s.import(&[item(Some("m2"), 0, "2026-09-29", "Dentist")]);
        s.toggle(d("2026-09-29"), 0, d("2026-09-29"));
        assert!(s.day(d("2026-09-29"))[0].done);
        // Same slot, new text: stays done.
        s.import(&[item(Some("m2"), 1, "2026-09-29", "Dentist (cleaning)")]);
        assert!(s.day(d("2026-09-29"))[0].done);
        // Moved: not done any more.
        s.import(&[item(Some("m2"), 2, "2026-10-01", "Dentist (cleaning)")]);
        assert!(!s.day(d("2026-10-01"))[0].done);
    }

    #[test]
    fn cancel_removes_series_and_unknown_cancel_is_noop() {
        let mut s = store();
        let mut it = item(Some("w1"), 0, "2026-09-25", "Standup");
        it.repeat = Repeat::Weekly;
        s.import(&[it]);
        assert!(s.has_entries(d("2026-10-16")));
        let mut c = item(Some("w1"), 1, "2026-09-25", "Standup");
        c.cancel = true;
        let r = s.import(&[c]);
        assert_eq!(r.cancelled, vec!["Standup"]);
        assert!(!s.has_entries(d("2026-09-25")));
        assert!(!s.has_entries(d("2026-10-16")));
        let mut c2 = item(Some("nope"), 0, "2026-09-25", "Ghost");
        c2.cancel = true;
        let r = s.import(&[c2]);
        assert_eq!(r.skipped, 1);
    }

    #[test]
    fn uidless_twin_is_skipped() {
        let mut s = store();
        s.import(&[item(None, 0, "2026-09-25", "Lunch")]);
        let r = s.import(&[item(None, 0, "2026-09-25", "Lunch")]);
        assert_eq!(r.skipped, 1);
        assert_eq!(s.day(d("2026-09-25")).len(), 1);
        // Different time is a different item.
        let mut it = item(None, 0, "2026-09-25", "Lunch");
        it.at_minute = Some(720);
        s.import(&[it]);
        assert_eq!(s.day(d("2026-09-25")).len(), 2);
    }

    #[test]
    fn moved_occurrence_punches_hole_and_adds_one_off() {
        let mut s = store();
        let mut series = item(Some("s1"), 0, "2026-09-25", "Standup");
        series.repeat = Repeat::Weekly;
        s.import(&[series]);
        // Oct 2's standup moved to Oct 3.
        let mut moved = item(Some("s1#2026-10-02"), 0, "2026-10-03", "Standup");
        moved.excludes_from = Some(("s1".into(), d("2026-10-02")));
        let r = s.import(&[moved]);
        assert_eq!(r.added, vec!["Standup"]);
        assert_eq!(s.day(d("2026-10-02")).len(), 0);
        assert_eq!(s.day(d("2026-10-03")).len(), 1);
        assert_eq!(s.day(d("2026-10-09")).len(), 1); // series continues
    }

    #[test]
    fn excluded_first_occurrence_hides_anchor_day() {
        let mut s = store();
        let mut series = item(Some("s2"), 0, "2026-09-25", "Standup");
        series.repeat = Repeat::Weekly;
        series.exdates = vec![d("2026-09-25")];
        s.import(&[series]);
        assert_eq!(s.day(d("2026-09-25")).len(), 0);
        assert!(!s.has_entries(d("2026-09-25")));
        assert_eq!(s.day(d("2026-10-02")).len(), 1);
    }

    #[test]
    fn fallback_is_reported() {
        let mut s = store();
        let mut it = item(Some("f1"), 0, "2026-09-25", "Odd rule");
        it.fallback = true;
        let r = s.import(&[it]);
        assert_eq!(r.added, vec!["Odd rule"]);
        assert_eq!(r.fallback, vec!["Odd rule"]);
    }
}
