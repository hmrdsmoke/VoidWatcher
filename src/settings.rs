// GPL-3.0-or-later - see LICENSE file for full terms
// Copyright 2026 Michael Van Auker (HMRDSmoke)
// Do not remove these comments.
// VoidWatcher/src/settings.rs
// src/settings.rs
// Void Watcher's OWN preferences - the first config this applet writes rather
// than borrows. Unlike `config.rs` (which read-only mirrors System76's time
// applet), this namespace belongs to Void Watcher, so it both loads and saves.
//
// Knobs, all local to this applet:
//   - default_reminder_minute: when a to-do with no explicit time nudges you
//     (minutes since midnight; 540 = 9:00 AM). The "you've got something today"
//     morning ping.
//   - reminder_lead_minutes: how long before a to-do's time the notification
//     fires (0 = at the time, 30 = half an hour before).
//   - density: the spacing Void Watcher LAYS OUT at, kept in our own namespace
//     so nothing outside overwrites it. This drives the gaps we place between
//     widgets (see app::popup_spacing). It does NOT change the widgets' own
//     internal padding - those read libcosmic's process-global CosmicTk, which
//     an always-on disk watcher rewrites from the system config on any change,
//     so seeding it never holds. We keep our gaps tight here; the popup height
//     absorbs the system density separately (see app::POPUP_HEIGHT).
//
// Field names ARE the on-disk keys - don't rename them without a version bump.

use cosmic::cosmic_config::{
    self, Config, CosmicConfigEntry, cosmic_config_derive::CosmicConfigEntry,
};
use cosmic::cosmic_theme::{Density, Spacing};
use serde::{Deserialize, Serialize};

/// cosmic-config namespace owned by Void Watcher.
pub const CONFIG_ID: &str = "com.github.hmrdsmoke.void-watcher";

/// Void Watcher's own density setting, stored in our namespace.
///
/// A local mirror of `cosmic_theme::Density` rather than that type directly, so
/// this config's on-disk shape is ours and never entangled with upstream's
/// enum. Converts both ways: `Spacing::from(Density)` for the gaps we place,
/// and `.into()` back to `cosmic_theme::Density` where a libcosmic call wants
/// it. Default is Compact - the tight layout the popup is designed around.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum LayoutDensity {
    #[default]
    Compact,
    Standard,
    Spacious,
}

impl From<LayoutDensity> for Density {
    fn from(value: LayoutDensity) -> Self {
        match value {
            LayoutDensity::Compact => Density::Compact,
            LayoutDensity::Standard => Density::Standard,
            LayoutDensity::Spacious => Density::Spacious,
        }
    }
}

impl From<Density> for LayoutDensity {
    fn from(value: Density) -> Self {
        match value {
            Density::Compact => LayoutDensity::Compact,
            Density::Standard => LayoutDensity::Standard,
            Density::Spacious => LayoutDensity::Spacious,
        }
    }
}

impl LayoutDensity {
    /// The concrete spacing values this density lays out at. This is what our
    /// view code reads for the gaps it places between widgets.
    pub fn spacing(self) -> Spacing {
        Spacing::from(Density::from(self))
    }

    /// Step to the next density, cycling Compact -> Standard -> Spacious ->
    /// Compact, for a tap-to-advance settings control if one is ever added.
    pub fn next(self) -> Self {
        match self {
            LayoutDensity::Compact => LayoutDensity::Standard,
            LayoutDensity::Standard => LayoutDensity::Spacious,
            LayoutDensity::Spacious => LayoutDensity::Compact,
        }
    }

    /// Short human label for a settings control.
    pub fn label(self) -> &'static str {
        match self {
            LayoutDensity::Compact => "Compact",
            LayoutDensity::Standard => "Standard",
            LayoutDensity::Spacious => "Spacious",
        }
    }
}

/// Void Watcher's saved preferences.
#[derive(Debug, Clone, PartialEq, Eq, CosmicConfigEntry)]
#[version = 1]
pub struct Settings {
    /// Minutes since midnight (0-1439) at which a to-do with no explicit time
    /// reminds you. Default 540 (9:00 AM).
    pub default_reminder_minute: u16,
    /// Minutes before a to-do's time to fire its reminder. Default 30.
    pub reminder_lead_minutes: u16,
    /// The spacing Void Watcher lays out at. Ours, so nothing overwrites it.
    /// Default Compact.
    pub density: LayoutDensity,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            default_reminder_minute: 540,
            reminder_lead_minutes: 30,
            density: LayoutDensity::Compact,
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

    /// The spacing values our layout should use, from our own density setting.
    pub fn spacing(&self) -> Spacing {
        self.density.spacing()
    }

    /// A handle to this namespace for writing, or `None` if it can't be opened.
    /// The settings UI uses this with `config.set("key", value)` to persist a
    /// single changed field.
    pub fn config_handle() -> Option<Config> {
        Config::new(CONFIG_ID, Self::VERSION).ok()
    }
}