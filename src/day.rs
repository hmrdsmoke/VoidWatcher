// GPL-3.0-or-later - see LICENSE file for full terms
// Copyright 2026 Michael Van Auker (HMRDSmoke)
// Do not remove these comments.
// VoidWatcher/src/day.rs
// src/day.rs
// The day view: the popup's second screen, one day's to-do list.
//
// Reached by right-clicking a day in the month grid. Layout, top to bottom:
// a header row ("To Do List" with a back button on the left), a text box to
// add an item, a -/+ stepper that tags the new item with an hour (or "None"),
// an Add button, then the day's existing items as checkbox / text / time /
// delete rows. Builds widgets only - every edit goes back to the app as a
// message, which mutates the Store.

use std::sync::LazyLock;

use cosmic::iced::{Alignment, Length};
use cosmic::widget::{button, checkbox, column, container, icon, row, text, text_input};
use cosmic::{Element, theme};
use jiff::civil::Date;

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
    /// Step the draft hour up one (None -> 0:00 -> 1:00 -> ... -> 23:00).
    HourUp,
    /// Step the draft hour down one (... -> 0:00 -> None).
    HourDown,
    /// Commit the current draft as a new entry.
    Submit,
    /// Toggle the done flag on the entry at this index.
    Toggle(usize),
    /// Delete the entry at this index.
    Delete(usize),
}

/// Step an optional hour up by one: `None` becomes 0, 23 saturates (stays 23).
pub fn hour_step_up(hour: Option<u8>) -> Option<u8> {
    match hour {
        None => Some(0),
        Some(h) if h < 23 => Some(h + 1),
        Some(_) => Some(23),
    }
}

/// Step an optional hour down by one: 0 becomes `None`, `None` stays `None`.
pub fn hour_step_down(hour: Option<u8>) -> Option<u8> {
    match hour {
        None => None,
        Some(0) => None,
        Some(h) => Some(h - 1),
    }
}

/// Human label for an hour value. `None` reads "None"; a set hour reads as a
/// 12-hour clock time like "2:00 PM", built via jiff so it matches the rest of
/// the UI. Falls back to a bare "H:00" only if the hour is out of range.
pub fn hour_label(hour: Option<u8>) -> String {
    match hour {
        None => "None".to_owned(),
        Some(h) => match jiff::civil::Time::new(h as i8, 0, 0, 0) {
            Ok(t) => jiff::fmt::strtime::format("%-I:%M %p", t)
                .unwrap_or_else(|_| format!("{h}:00")),
            Err(_) => format!("{h}:00"),
        },
    }
}

/// Build the day view for `date`, reading entries from `store` and showing
/// `draft` in the add box with `draft_hour` in the stepper.
pub fn view<'a>(
    date: Date,
    store: &'a Store,
    draft: &'a str,
    draft_hour: Option<u8>,
) -> Element<'a, DayMessage> {
    let spacing = theme::active().cosmic().spacing;

    // Header row: back button on the left, "To Do List" centered. Both side
    // slots are the SAME fixed width so the centered title sits dead center in
    // the popup; the back button is centered within its slot.
    let back = button::icon(icon::from_name("go-previous-symbolic").size(16))
        .on_press(DayMessage::Back);
    let side = 44.0;
    let header = row::with_capacity(3)
        .push(container(back).width(Length::Fixed(side)))
        .push(
            container(text("To Do List").size(20))
                .width(Length::Fill)
                .center_x(Length::Fill),
        )
        .push(cosmic::widget::space::horizontal().width(Length::Fixed(side)))
        .align_y(Alignment::Center);

    // Add box.
    let input = text_input("Add a to-do...", draft)
        .id(INPUT_ID.clone())
        .on_input(DayMessage::Input)
        .on_submit(|_| DayMessage::Submit)
        .width(Length::Fill);

    // -/+ hour stepper: minus, the current label, plus. Two plain icon buttons
    // around a centered label; no popover, nothing to get clipped in the popup.
    let minus = button::icon(icon::from_name("list-remove-symbolic").size(16))
        .on_press(DayMessage::HourDown);
    let plus = button::icon(icon::from_name("list-add-symbolic").size(16))
        .on_press(DayMessage::HourUp);
    let stepper = row::with_capacity(3)
        .push(minus)
        .push(
            container(text(hour_label(draft_hour)).size(14))
                .width(Length::Fill)
                .center_x(Length::Fill),
        )
        .push(plus)
        .spacing(spacing.space_xs)
        .align_y(Alignment::Center);

    // Add button.
    let add_button = button::text("Add").on_press(DayMessage::Submit).width(Length::Fill);

    // The existing items, one row each.
    let entries = store.day(date);
    let mut list = column::with_capacity(entries.len().max(1)).spacing(spacing.space_xxs);
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

    column::with_capacity(5)
        .push(header)
        .push(input)
        .push(stepper)
        .push(add_button)
        .push(list)
        .spacing(spacing.space_s)
        .width(Length::Fill)
        .into()
}

/// One entry row: checkbox toggles done, text (with its hour label if any),
/// trash deletes.
fn entry_row<'a>(
    index: usize,
    label: &'a str,
    done: bool,
    hour: Option<u8>,
) -> Element<'a, DayMessage> {
    let spacing = theme::active().cosmic().spacing;

    let check = checkbox(done).on_toggle(move |_| DayMessage::Toggle(index));

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