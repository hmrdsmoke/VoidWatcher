// GPL-3.0-or-later - see LICENSE file for full terms
// Copyright 2026 Michael Van Auker (HMRDSmoke)
// Do not remove these comments.
// VoidWatcher/src/settings.rs
// src/settings.rs
// Void Watcher's OWN preferences - the first config this applet writes rather
// than borrows. Unlike `config.rs` (which read-only mirrors System76's time
// applet), this namespace belongs to Void Watcher, so it both loads and saves.
//
// Two knobs to start, both about reminders:
//   - default_reminder_minute: when a to-do with no explicit time nudges you
//     (minutes since midnight; 540 = 9:00 AM). The "you've got something today"
//     morning ping.
//   - reminder_lead_minutes: how long before a to-do's time the notification
//     fires (0 = at the time, 30 = half an hour before).
//
// Field names ARE the on-disk keys - don't rename them without a version bump.

use cosmic::cosmic_config::{
    self, Config, CosmicConfigEntry, cosmic_config_derive::CosmicConfigEntry,
};

/// cosmic-config namespace owned by Void Watcher.
pub const CONFIG_ID: &str = "com.github.hmrdsmoke.void-watcher";

/// Void Watcher's saved preferences.
#[derive(Debug, Clone, PartialEq, Eq, CosmicConfigEntry)]
#[version = 1]
pub struct Settings {
    /// Minutes since midnight (0-1439) at which a to-do with no explicit time
    /// reminds you. Default 540 (9:00 AM).
    pub default_reminder_minute: u16,
    /// Minutes before a to-do's time to fire its reminder. Default 30.
    pub reminder_lead_minutes: u16,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            default_reminder_minute: 540,
            reminder_lead_minutes: 30,
        }
    }
}

impl Settings {
    /// Load saved preferences, falling back per-field to the defaults. Never
    /// fails: a missing namespace, missing keys, or a version mismatch all
    /// degrade to defaults rather than taking the applet down.
    pub fn load() -> Self {
        match Config::new(CONFIG_ID, Self::VERSION) {
            Ok(config) => Self::get_entry(&config).unwrap_or_else(|(_, partial)| partial),
            Err(_) => Self::default(),
        }
    }

    /// A handle to this namespace for writing, or `None` if it can't be opened.
    /// The settings UI uses this with `config.set("key", value)` to persist a
    /// single changed field.
    pub fn config_handle() -> Option<Config> {
        Config::new(CONFIG_ID, Self::VERSION).ok()
    }
}
