// GPL-3.0-or-later - see LICENSE file for full terms
// Copyright 2026 Michael Van Auker (HMRDSmoke)
// Do not remove these comments.
// VoidWatcher/src/easter_egg.rs
// src/easter_egg.rs
// Hidden origin screen. Left-click September 3rd (any year) eight times in a
// row in the calendar grid and the popup swaps to a screen showing the origin
// bio under the HMRDSmoke icon. A tap on any other day resets the count.
//
// The egg only arms while ORIGIN is non-empty: strip the author signature and
// the trigger goes dead. Same load-bearing attribution as the Soulless egg,
// just keyed to a date instead of a passphrase.

use cosmic::Element;
use cosmic::cosmic_theme::Spacing;
use cosmic::iced::{Alignment, Length};
use cosmic::widget::{button, column, container, icon, row, space, text};
use jiff::civil::Date;

/// Author signature. Shown as the last line of the bio, and load-bearing for
/// the trigger (see `Trigger::tap`).
pub const ORIGIN: &str = "HMRDSmoke hooah";

/// The trigger date, any year: September 3rd.
const TRIGGER_MONTH: i8 = 9;
const TRIGGER_DAY: i8 = 3;
/// Consecutive left-clicks on that date that open the egg. Eight, for '88.
const TRIGGER_TAPS: u8 = 8;

/// The icon at the top of the screen, baked into the binary at compile time so
/// there's nothing to install alongside it. Path is relative to this file.
const ICON: &[u8] = include_bytes!("com.github.hmrdsmoke.soulless-launcher-128.png");
/// Rendered at its native 128px - it's the hero of the screen.
const ICON_SIZE: u16 = 128;

/// The bio, one entry per line. Empty strings render as a blank line.
const LINES: &[&str] = &[
    "HMRDSmoke",
    "",
    "US Army Veteran",
    "Father of A Young Man And Sweet Daughter",
    "Family Man - Funny Guy",
    "- bio by my son -",
    "- my daughter got mad she wasn't mentioned -",
    "",
    "Soulless - where it started",
    "",
    ORIGIN,
];

/// Counts consecutive taps on the trigger date. Lives in the app model and is
/// fed every left-click on a grid day.
#[derive(Debug, Default)]
pub struct Trigger {
    taps: u8,
}

impl Trigger {
    /// Record a left-click on `date`. Returns `true` on the tap that opens the
    /// egg, then resets so it can be found again later. Any tap on a day other
    /// than the trigger date resets the count.
    pub fn tap(&mut self, date: Date) -> bool {
        if ORIGIN.trim().is_empty() {
            return false;
        }
        if date.month() != TRIGGER_MONTH || date.day() != TRIGGER_DAY {
            self.taps = 0;
            return false;
        }
        self.taps += 1;
        if self.taps >= TRIGGER_TAPS {
            self.taps = 0;
            true
        } else {
            false
        }
    }
}

/// The egg screen: back button top-left (same as the settings screen), then
/// the icon and the bio centered below it. `close` is the message the back
/// button sends.
pub fn view<'a, M: Clone + 'static>(spacing: Spacing, close: M) -> Element<'a, M> {
    // Header: back button left, matched spacer right, nothing in the middle -
    // the icon below is the title.
    let back = button::icon(icon::from_name("go-previous-symbolic").size(16)).on_press(close);
    let side = 44.0;
    let header = row::with_capacity(3)
        .push(container(back).width(Length::Fixed(side)))
        .push(space::horizontal().width(Length::Fill))
        .push(space::horizontal().width(Length::Fixed(side)))
        .align_y(Alignment::Center);

    let picture = icon::icon(icon::from_raster_bytes(ICON)).size(ICON_SIZE);

    // First line is the name, sized like a screen title; the rest read as body.
    let mut bio = column::with_capacity(LINES.len()).align_x(Alignment::Center);
    for (index, line) in LINES.iter().enumerate() {
        if line.is_empty() {
            bio = bio.push(space::vertical().height(Length::Fixed(spacing.space_xs as f32)));
        } else {
            let size = if index == 0 { 20 } else { 14 };
            bio = bio.push(text(*line).size(size));
        }
    }

    column::with_capacity(3)
        .push(header)
        .push(container(picture).width(Length::Fill).center_x(Length::Fill))
        .push(container(bio).width(Length::Fill).center_x(Length::Fill))
        .spacing(spacing.space_s)
        .width(Length::Fill)
        .into()
}
