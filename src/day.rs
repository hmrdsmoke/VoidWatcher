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
    /// Step the hour up/down by one (wraps 23<->0). From None, starts a time.
    HourUp,
    HourDown,
    /// Step the minute up/down by 5 (wraps 55<->0). From None, starts a time.
    MinuteUp,
    MinuteDown,
    /// Clear the time back to None (no explicit time; uses the daily default).
    ClearTime,
    /// Commit the current draft as a new entry.
    Submit,
    /// Toggle the done flag on the entry at this index.
    Toggle(usize),
    /// Delete the entry at this index.
    Delete(usize),
}

/// Minutes a new time starts at when stepping up from `None`. 9:00 AM.
const START_MINUTE: u16 = 540;

/// Step the hour part of a time by `dir` (+1 or -1), keeping the minute part,
/// wrapping 23<->0. `None` starts a time at `START_MINUTE` (either direction),
/// so a first press gives you something to adjust rather than doing nothing.
pub fn step_hour(at: Option<u16>, dir: i32) -> Option<u16> {
    let Some(m) = at else {
        return Some(START_MINUTE);
    };
    let hour = (m / 60) as i32;
    let minute = m % 60;
    let hour = (hour + dir).rem_euclid(24) as u16;
    Some(hour * 60 + minute)
}

/// Step the minute part of a time by `dir` (+1 or -1) in 5-minute increments,
/// keeping the hour part, wrapping 55<->0. `None` starts a time at
/// `START_MINUTE`.
pub fn step_minute(at: Option<u16>, dir: i32) -> Option<u16> {
    let Some(m) = at else {
        return Some(START_MINUTE);
    };
    let hour = m / 60;
    let minute = (m % 60) as i32;
    // Round to the current 5-min slot, then step. rem_euclid keeps 0..=55.
    let slot = minute / 5;
    let slot = (slot + dir).rem_euclid(12);
    Some(hour * 60 + (slot as u16) * 5)
}

/// Human label for a time given as minutes since midnight, honoring the user's
/// 12/24-hour setting (`military`). `None` reads "None". 24-hour reads like
/// "13:30" / "00:00"; 12-hour reads like "1:30 PM" / "12:00 AM". Built via jiff
/// so it matches the panel clock; falls back to a zero-padded 24-hour "HH:MM"
/// only if the time can't be built.
pub fn time_label(at: Option<u16>, military: bool) -> String {
    match at {
        None => "None".to_owned(),
        Some(m) => {
            let hour = (m / 60) as i8;
            let minute = (m % 60) as i8;
            let fmt = if military { "%H:%M" } else { "%-I:%M %p" };
            match jiff::civil::Time::new(hour, minute, 0, 0) {
                Ok(t) => jiff::fmt::strtime::format(fmt, t)
                    .unwrap_or_else(|_| format!("{hour:02}:{minute:02}")),
                Err(_) => format!("{hour:02}:{minute:02}"),
            }
        }
    }
}

/// Build the day view for `date`, reading entries from `store` and showing
/// `draft` in the add box with `draft_minute` in the time picker.
pub fn view<'a>(
    date: Date,
    store: &'a Store,
    draft: &'a str,
    draft_minute: Option<u16>,
    military: bool,
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

    // Time picker, styled as one compact control inside a bordered pill: an
    // hour stepper and a minute stepper (each a -/value/+ group) with a clear
    // button, centered in the row. The value shows "None" until a step sets a
    // time; the fixed value width keeps the pill from jumping as it changes.
    let hour_minus = button::icon(icon::from_name("list-remove-symbolic").size(14))
        .on_press(DayMessage::HourDown);
    let hour_plus = button::icon(icon::from_name("list-add-symbolic").size(14))
        .on_press(DayMessage::HourUp);
    let minute_minus = button::icon(icon::from_name("list-remove-symbolic").size(14))
        .on_press(DayMessage::MinuteDown);
    let minute_plus = button::icon(icon::from_name("list-add-symbolic").size(14))
        .on_press(DayMessage::MinuteUp);
    let clear = button::icon(icon::from_name("edit-clear-symbolic").size(14))
        .on_press(DayMessage::ClearTime);

    // The two steppers share one value label (the whole time reads together, so
    // "None" and "2:30 PM" both make sense) flanked by hour controls on the
    // left and minute controls on the right.
    let stepper_inner = row::with_capacity(6)
        .push(hour_minus)
        .push(hour_plus)
        .push(
            container(text(time_label(draft_minute, military)).size(14))
                .width(Length::Fixed(84.0))
                .center_x(Length::Fixed(84.0)),
        )
        .push(minute_minus)
        .push(minute_plus)
        .push(clear)
        .spacing(spacing.space_xxs)
        .align_y(Alignment::Center);
    let stepper_pill = container(stepper_inner)
        .padding([2, 6])
        .class(cosmic::style::Container::custom(|t| {
            let cosmic = t.cosmic();
            cosmic::iced::widget::container::Style {
                border: cosmic::iced::Border {
                    radius: cosmic.corner_radii.radius_m.into(),
                    width: 1.0,
                    color: cosmic.palette.neutral_5.into(),
                },
                ..Default::default()
            }
        }));
    // Center the pill in the full-width row so it floats in the middle.
    let stepper = container(stepper_pill)
        .width(Length::Fill)
        .center_x(Length::Fill);

    // Add button.
    // Add button: the primary action, so a filled "suggested" (accent) button
    // rather than flat text. Reads clearly as the thing to press.
    let add_button = button::suggested("Add")
        .on_press(DayMessage::Submit)
        .width(Length::Fill);

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
            list = list.push(entry_row(index, &entry.text, entry.done, entry.at_minute, military));
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
    at_minute: Option<u16>,
    military: bool,
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

    if at_minute.is_some() {
        r = r.push(
            text(time_label(at_minute, military))
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