// GPL-3.0-or-later - see LICENSE file for full terms
// Copyright 2026 Michael Van Auker (HMRDSmoke)
// Do not remove these comments.
// VoidWatcher/src/day.rs
// src/day.rs
// The day view: the popup's second screen, one day's to-do list.
//
// Reached by right-clicking a day in the month grid. Layout, top to bottom:
// a header row ("To Do List" with a back button on the left), a text box to
// add an item, a picker (a -/+ time stepper, and under it a tap-to-advance
// repeat pill beside a -/+ "Ends" pill), an Add button, then the day's existing
// items as checkbox / text / time / glyphs / delete rows. Builds widgets only -
// every edit goes back to the app as a message, which mutates the Store.
//
// Recurrence (v2): the picker gained a repeat pill (Once/Daily/Weekly/Yearly)
// that cycles on tap, matching the hand-built stepper style. Rows for recurring
// items show a small repeat glyph. Because a day now shows items that may be
// anchored on other days (expand-on-read), each row's Toggle/Delete carries its
// position in the expanded list; the app maps that back to the source entry.
//
// End dates and invites (v3): an "Ends" pill sits beside the repeat pill,
// greyed out until a repeat is chosen. Each plus adds one period of that
// repeat and the label reads the DATE it lands on ("Ends: Oct 2"), not a
// count, so you see the month and day the recurrence stops. Rows that came
// from a calendar invite (see ics.rs) show a small envelope glyph: that one
// follows the organizer's updates.

use std::sync::LazyLock;

use cosmic::cosmic_theme::Spacing;
use cosmic::iced::{Alignment, Length};
use cosmic::widget::{button, checkbox, column, container, icon, row, text, text_input};
use cosmic::Element;
use jiff::civil::Date;

use crate::store::{Repeat, Store};

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
    /// Advance the draft repeat rule (Once -> Daily -> Weekly -> Yearly -> Once).
    CycleRepeat,
    /// Push the draft's end date out by one period of its repeat, or pull it
    /// back one (down to no end). Ignored while the repeat is Once.
    EndsUp,
    EndsDown,
    /// Commit the current draft as a new entry.
    Submit,
    /// Toggle the done flag for the occurrence at this row.
    Toggle(usize),
    /// Delete the entry backing this row (whole series if it recurs).
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

/// Label for the Ends pill: "Never", or the last day the recurrence applies,
/// as month and day - with the year only when it isn't this year (a yearly
/// repeat's end always is: "Sep 3, 2028").
pub fn end_label(end: Option<Date>, today: Date) -> String {
    match end {
        None => "Ends: Never".to_owned(),
        Some(date) => {
            let fmt = if date.year() == today.year() { "%b %-d" } else { "%b %-d, %Y" };
            let when = jiff::fmt::strtime::format(fmt, date).unwrap_or_else(|_| date.to_string());
            format!("Ends: {when}")
        }
    }
}

/// Build the day view for `date`, reading entries from `store` and showing
/// `draft` in the add box with `draft_minute` in the time picker,
/// `draft_repeat` in the repeat pill and `draft_end` in the Ends pill.
/// `today` decides whether the end date needs its year spelled out.
pub fn view<'a>(
    date: Date,
    today: Date,
    store: &'a Store,
    draft: &'a str,
    draft_minute: Option<u16>,
    draft_repeat: Repeat,
    draft_end: Option<Date>,
    military: bool,
    spacing: Spacing,
) -> Element<'a, DayMessage> {

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

    // Repeat pill: a single tap-to-advance control cycling Once/Daily/Weekly/
    // Yearly, styled to match the time pill. A leading repeat glyph makes its
    // purpose read at a glance; the fixed label width keeps it from resizing as
    // the word changes. The whole pill is one button so tapping anywhere on it
    // advances.
    let repeat_inner = row::with_capacity(2)
        .push(icon::from_name("media-playlist-repeat-symbolic").size(14))
        .push(
            container(text(draft_repeat.label()).size(14))
                .width(Length::Fixed(56.0))
                .center_x(Length::Fixed(56.0)),
        )
        .spacing(spacing.space_xxs)
        .align_y(Alignment::Center);
    let repeat_pill = button::custom(container(repeat_inner).padding([2, 6]))
        .padding(0)
        .on_press(DayMessage::CycleRepeat)
        .class(repeat_pill_class());

    // Ends pill: a -/+ stepper in the same bordered pill as the time stepper.
    // Greyed out - no presses, muted label - until a repeat is chosen, rather
    // than hidden, so nothing jumps when a repeat is picked and the option is
    // visible before it's usable. Each plus adds one period of the repeat; the
    // label is the date that lands on. Minus is dead at "Never" - nothing to
    // pull back to. The fixed label width keeps the pill from resizing as the
    // date changes.
    let ends_enabled = draft_repeat != Repeat::None;
    let mut ends_minus = button::icon(icon::from_name("list-remove-symbolic").size(14));
    let mut ends_plus = button::icon(icon::from_name("list-add-symbolic").size(14));
    if ends_enabled {
        ends_plus = ends_plus.on_press(DayMessage::EndsUp);
        if draft_end.is_some() {
            ends_minus = ends_minus.on_press(DayMessage::EndsDown);
        }
    }
    let ends_text = text(end_label(draft_end, today)).size(14).class(if ends_enabled {
        cosmic::style::Text::Default
    } else {
        cosmic::style::Text::Custom(|t| cosmic::iced::widget::text::Style {
            color: Some(t.cosmic().palette.neutral_6.into()),
            ..Default::default()
        })
    });
    let ends_inner = row::with_capacity(3)
        .push(ends_minus)
        .push(
            container(ends_text)
                .width(Length::Fixed(118.0))
                .center_x(Length::Fixed(118.0)),
        )
        .push(ends_plus)
        .spacing(spacing.space_xxs)
        .align_y(Alignment::Center);
    let ends_pill = container(ends_inner)
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

    // Picker: time stepper on top; under it the repeat pill and the Ends pill
    // side by side. The stepper and a pill together overflow the popup's fixed
    // width (POPUP_WIDTH is pinned to the calendar grid), but the two small
    // pills fit one row with room to spare, and the pair reads as one control:
    // how often, and until when.
    let pills = row::with_capacity(2)
        .push(repeat_pill)
        .push(ends_pill)
        .spacing(spacing.space_xs)
        .align_y(Alignment::Center);
    let picker_inner = column::with_capacity(2)
        .push(stepper_pill)
        .push(pills)
        .spacing(spacing.space_xs)
        .align_x(Alignment::Center);
    let picker = container(picker_inner)
        .width(Length::Fill)
        .center_x(Length::Fill);

    // Add button.
    // Add button: the primary action, so a filled "suggested" (accent) button
    // rather than flat text. Reads clearly as the thing to press.
    let add_button = button::suggested("Add")
        .on_press(DayMessage::Submit)
        .width(Length::Fill);

    // The existing items, one row each. `day()` expands recurrence, so this
    // mixes items anchored here with recurring items landing here; each row's
    // index is its position in this expanded list, which is what Toggle/Delete
    // carry back (the app maps index -> source entry).
    let items = store.day(date);
    let mut list = column::with_capacity(items.len().max(1)).spacing(spacing.space_xxs);
    if items.is_empty() {
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
        for (index, item) in items.into_iter().enumerate() {
            list = list.push(entry_row(
                index,
                item.text,
                item.done,
                item.at_minute,
                item.repeat,
                item.imported,
                military,
                spacing,
            ));
        }
    }

    column::with_capacity(5)
        .push(header)
        .push(input)
        .push(picker)
        .push(add_button)
        .push(list)
        .spacing(spacing.space_s)
        .width(Length::Fill)
        .into()
}

/// Styling for the repeat pill button: a bordered pill that mirrors the time
/// stepper's look, with a subtle neutral wash on hover/press so it reads as
/// tappable. Non-capturing closures - the theme is queried inside each.
fn repeat_pill_class() -> cosmic::theme::Button {
    cosmic::theme::Button::Custom {
        active: Box::new(|_selected, t| {
            let cosmic = t.cosmic();
            cosmic::widget::button::Style {
                border_radius: cosmic.corner_radii.radius_m.into(),
                border_width: 1.0,
                border_color: cosmic.palette.neutral_5.into(),
                ..Default::default()
            }
        }),
        hovered: Box::new(|_selected, t| {
            let cosmic = t.cosmic();
            let bg = cosmic::iced::Color::from(cosmic.palette.neutral_4);
            let bg = cosmic::iced::Color { a: 0.35, ..bg };
            cosmic::widget::button::Style {
                background: Some(cosmic::iced::Background::Color(bg)),
                border_radius: cosmic.corner_radii.radius_m.into(),
                border_width: 1.0,
                border_color: cosmic.palette.neutral_5.into(),
                ..Default::default()
            }
        }),
        pressed: Box::new(|_selected, t| {
            let cosmic = t.cosmic();
            let bg = cosmic::iced::Color::from(cosmic.palette.neutral_5);
            let bg = cosmic::iced::Color { a: 0.45, ..bg };
            cosmic::widget::button::Style {
                background: Some(cosmic::iced::Background::Color(bg)),
                border_radius: cosmic.corner_radii.radius_m.into(),
                border_width: 1.0,
                border_color: cosmic.palette.neutral_5.into(),
                ..Default::default()
            }
        }),
        disabled: Box::new(|_t| cosmic::widget::button::Style::default()),
    }
}

/// One entry row: checkbox toggles done, text (with its hour label if any),
/// an optional repeat glyph for recurring items, an optional envelope glyph
/// for items that came from an invite, trash deletes.
fn entry_row<'a>(
    index: usize,
    label: String,
    done: bool,
    at_minute: Option<u16>,
    repeat: Repeat,
    imported: bool,
    military: bool,
    spacing: Spacing,
) -> Element<'a, DayMessage> {
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

    let mut r = row::with_capacity(6).push(check).push(text_widget);

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

    // Small repeat glyph for recurring items, so a filled/greyed row still
    // reads as "this one comes back". Muted to sit quietly next to the time.
    if repeat != Repeat::None {
        r = r.push(
            container(icon::from_name("media-playlist-repeat-symbolic").size(12))
                .padding([0, spacing.space_xxxs as u16]),
        );
    }

    // Small envelope for items that came from a calendar invite: this row
    // follows the organizer - an update moves it, a cancellation removes it.
    if imported {
        r = r.push(
            container(icon::from_name("mail-unread-symbolic").size(12))
                .padding([0, spacing.space_xxxs as u16]),
        );
    }

    r.push(delete)
        .spacing(spacing.space_xxs)
        .align_y(Alignment::Center)
        .into()
}