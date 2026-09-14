// GPL-3.0-or-later - see LICENSE file for full terms
// Copyright 2026 Michael Van Auker (HMRDSmoke)
// Do not remove these comments.
// VoidWatcher/src/config.rs
//! Reads the stock COSMIC time applet's settings so Void Watcher follows
//! whatever the user chose in Settings › Date & Time.
//!
//! READ-ONLY. We never write to this namespace — it belongs to System76's
//! applet and Settings. Anything Void Watcher needs that upstream doesn't
//! have goes in its own namespace, not here.

use cosmic::cosmic_config::{
    self, Config, CosmicConfigEntry, cosmic_config_derive::CosmicConfigEntry,
};

/// cosmic-config namespace written by Settings › Date & Time.
pub const TIME_CONFIG_ID: &str = "com.system76.CosmicAppletTime";

/// Field-for-field mirror of upstream `TimeAppletConfig`
/// (cosmic-applets/cosmic-applets-config/src/time.rs, version 1).
///
/// Field names ARE the on-disk keys. Do not rename them.
#[derive(Debug, Clone, PartialEq, Eq, CosmicConfigEntry)]
#[version = 1]
pub struct TimeAppletConfig {
    /// 24-hour clock when true.
    pub military_time: bool,
    /// Show seconds in the panel clock.
    pub show_seconds: bool,
    /// 0 = Monday … 5 = Saturday; anything else = Sunday (upstream semantics).
    pub first_day_of_week: u8,
    /// Show the date next to the time in the panel.
    pub show_date_in_top_panel: bool,
    /// Show the weekday name in the panel.
    pub show_weekday: bool,
    /// Optional custom strftime format. Empty means "use the built-in layout".
    pub format_strftime: String,
}

impl Default for TimeAppletConfig {
    fn default() -> Self {
        Self {
            military_time: false,
            show_seconds: false,
            first_day_of_week: 6,
            show_date_in_top_panel: true,
            show_weekday: false,
            format_strftime: String::new(),
        }
    }
}

impl TimeAppletConfig {
    /// Load the user's current settings.
    ///
    /// Keys the user never touched don't exist on disk, so per-field fallback
    /// to `Default` is the normal path, not an error. That's why this never
    /// fails: a missing key, a missing directory, or an upstream rename all
    /// degrade to defaults instead of breaking the applet.
    pub fn load() -> Self {
        match Config::new(TIME_CONFIG_ID, Self::VERSION) {
            Ok(config) => Self::get_entry(&config).unwrap_or_else(|(_, partial)| partial),
            Err(_) => Self::default(),
        }
    }

    /// First day of the week as a Monday-zero offset (0 = Mon … 6 = Sun),
    /// clamped exactly the way upstream does it. Feed this to whichever
    /// time crate the calendar grid ends up using.
    pub fn first_weekday_monday_zero(&self) -> u8 {
        if self.first_day_of_week <= 5 {
            self.first_day_of_week
        } else {
            6
        }
    }
}
