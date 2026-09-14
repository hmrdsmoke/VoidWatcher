// GPL-3.0-or-later - see LICENSE file for full terms
// Copyright 2026 Michael Van Auker (HMRDSmoke)
// Do not remove these comments.
// VoidWatcher/src/day.rs
// src/day.rs
// The day view: the popup's second screen, showing one day's to-do list.
//
// Reached by clicking a day in the month grid. Lays out a header (back arrow +
// the date), the day's entries as checkbox / label / delete rows, and a row to
// add a new one: a text box, an hour picker, and an add button. It only builds
// widgets — all edits go back to the app as messages, which mutate the Store.

use std::sync::LazyLock;

use cosmic::iced::{Alignment, Length};
use cosmic::widget::{button, checkbox, column, container, icon, row, spin_button, text, text_input};
use cosmic::{Element, theme};
use jiff::civil::Date;
use jiff::fmt::strtime;

use crate::store::Store;

/// Stable id for the "add" box, so the app can focus it when the day opens.
pub static INPUT_ID: LazyLock<cosmic::widget::Id> =
    LazyLock::new(|| cosmic::widget::Id::new("void-watcher-day-input"));

/// What the day view emits. The app owns the Store and the draft text; this
/// module just names the user's intents.
#[derive(Debug, Clone)]
pub enum DayMessage {
    /// Return to the month grid.
    Back,
    /// The add-box text changed.
    Input(String),
    /// The hour picker changed. Carries the raw spin value (see `HOUR_ANYTIME`
    /// / `hour_from_spin`): 0 means "Anytime" (no reminder), 1..=24 map to
    /// hours 0..=23.
    HourSpin(i16),
    /// Commit the current draft as a new entry.
    Submit,
    /// Toggle the done flag on the entry at this index.
    Toggle(usize),
    /// Delete the entry at this index.
    Delete(usize),
}

/// The spin value that means "no hour, no reminder". Hours 0–23 are stored as
/// spin values 1–24 so that 0 can stand for "Anytime" at the bottom of the range.
pub const HOUR_ANYTIME: i16 = 0;

/// Convert an `Option<u8>` hour (as stored on an entry / held in the draft) to
/// the spin widget's raw value.
pub fn hour_to_spin(hour: Option<u8>) -> i16 {
    match hour {
        None => HOUR_ANYTIME,
        Some(h) => h as i16 + 1,
    }
}

/// Convert the spin widget's raw value back to an `Option<u8>` hour, clamping to
/// the valid range. Out-of-range or `HOUR_ANYTIME` becomes `None`.
pub fn hour_from_spin(value: i16) -> Option<u8> {
    if (1..=24).contains(&value) {
        Some((value - 1) as u8)
    } else {
        None
    }
}

/// Human label for an hour value, used both by the spin button and the entry
/// rows. `None` reads as "Anytime"; a set hour reads as a 12-hour clock time
/// like "2:00 PM", built via jiff so it matches the rest of the UI.
pub fn hour_label(hour: Option<u8>) -> String {
    match hour {
        None => "Anytime".to_owned(),
        Some(h) => {
            // Build a civil time at h:00 and format 12-hour; fall back to a bare
            // "H:00" if for some reason the hour is out of jiff's range.
            match jiff::civil::Time::new(h as i8, 0, 0, 0) {
                Ok(t) => strtime::format("%-I:%M %p", t).unwrap_or_else(|_| format!("{h}:00")),
                Err(_) => format!("{h}:00"),
            }
        }
    }
}

/// Build the day view for `date`, reading entries from `store` and showing
/// `draft` in the add box with `draft_hour` in the hour picker. `time` is the
/// current time string, formatted by the app to honor the 12/24h setting, so
/// the header's third line matches the month screen.
pub fn view<'a>(
    date: Date,
    store: &'a Store,
    draft: &'a str,
    draft_hour: Option<u8>,
    time: String,
) -> Element<'a, DayMessage> {
    let spacing = theme::active().cosmic().spacing;

    // Same three-line header as the month screen: date, weekday, time. The back
    // button sits on the left of the row, where the month screen's ‹ arrow is,
    // so the two screens line up. It's a real icon button (not a bare glyph) so
    // it has a proper hit target.
    let date_line = strtime::format("%B %-d, %Y", date).unwrap_or_default();
    let weekday_line = strtime::format("%A", date).unwrap_or_default();

    let center = column::with_capacity(3)
        .push(text(date_line).size(20))
        .push(text(weekday_line).size(14))
        .push(text(time).size(14))
        .align_x(Alignment::Center)
        .spacing(spacing.space_xxxs);

    // Fixed-width cells on both sides so the Fill center sits strictly between
    // them and can't drift over the back button. Equal widths keep the
    // three-line block centered. The earlier layout let the centered text span
    // the whole row and overlap the button, which ate clicks intermittently.
    let back_cell = container(back_button()).width(Length::Fixed(40.0));

    let header = row::with_capacity(3)
        .push(back_cell)
        .push(center.width(Length::Fill))
        .push(cosmic::widget::space::horizontal().width(Length::Fixed(40.0)))
        .spacing(spacing.space_xxs)
        .align_y(Alignment::Center);

    let entries = store.day(date);
    let mut list = column::with_capacity(entries.len() + 1).spacing(spacing.space_xxs);

    if entries.is_empty() {
        list = list.push(
            text("Nothing yet.")
                .size(13)
                .class(cosmic::style::Text::Custom(|t| {
                    cosmic::iced::widget::text::Style {
                        color: Some(t.cosmic().palette.neutral_6.into()),
                        ..Default::default()
                    }
                })),
        );
    } else {
        for (index, entry) in entries.iter().enumerate() {
            list = list.push(entry_row(index, &entry.text, entry.done, entry.hour));
        }
    }

    let input = text_input("Add a to-do…", draft)
        .id(INPUT_ID.clone())
        .on_input(DayMessage::Input)
        .on_submit(|_| DayMessage::Submit)
        .width(Length::Fill);

    // Hour picker: 0 ("Anytime") through 24, shown as a clock time or "Anytime".
    // No popover menu (unlike a dropdown), so nothing gets clipped inside the
    // applet popup. The label reflects the current draft hour.
    let hour_spin = spin_button(
        hour_label(draft_hour),
        "Reminder hour",
        hour_to_spin(draft_hour),
        1,
        HOUR_ANYTIME,
        24,
        DayMessage::HourSpin,
    );

    let add_button = button::icon(icon::from_name("list-add-symbolic").size(16))
        .on_press(DayMessage::Submit);

    let add_row = row::with_capacity(3)
        .push(input)
        .push(hour_spin)
        .push(add_button)
        .spacing(spacing.space_xxs)
        .align_y(Alignment::Center);

    column::with_capacity(3)
        .push(header)
        .push(list)
        .push(add_row)
        .spacing(spacing.space_s)
        .width(Length::Fill)
        .into()
}

/// One entry row: checkbox toggles done, label shows the text (with its hour, if
/// any), trash deletes.
fn entry_row<'a>(
    index: usize,
    label: &'a str,
    done: bool,
    hour: Option<u8>,
) -> Element<'a, DayMessage> {
    let spacing = theme::active().cosmic().spacing;

    let check = checkbox(done).on_toggle(move |_| DayMessage::Toggle(index));

    // Completed items read back muted, so a glance separates done from open.
    let text_widget = text(label).width(Length::Fill).class(if done {
        cosmic::style::Text::Custom(|t| cosmic::iced::widget::text::Style {
            color: Some(t.cosmic().palette.neutral_6.into()),
            ..Default::default()
        })
    } else {
        cosmic::style::Text::Default
    });

    let delete = button::icon(icon::from_name("user-trash-symbolic").size(16))
        .on_press(DayMessage::Delete(index));

    let mut r = row::with_capacity(4).push(check).push(text_widget);

    // Show the hour as a muted trailing label only when the entry has one.
    if hour.is_some() {
        r = r.push(
            text(hour_label(hour))
                .size(12)
                .class(cosmic::style::Text::Custom(|t| {
                    cosmic::iced::widget::text::Style {
                        color: Some(t.cosmic().palette.neutral_6.into()),
                        ..Default::default()
                    }
                })),
        );
    }

    r.push(delete)
        .spacing(spacing.space_xxs)
        .align_y(Alignment::Center)
        .into()
}

/// The back arrow: a real icon button, so it has a proper hit target. The bare
/// glyph it replaced was easy to miss, which made "back" feel unreliable.
fn back_button<'a>() -> Element<'a, DayMessage> {
    button::icon(icon::from_name("go-previous-symbolic").size(16))
        .on_press(DayMessage::Back)
        .into()
}