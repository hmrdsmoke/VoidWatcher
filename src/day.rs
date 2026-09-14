// src/day.rs
// The day view: the popup's second screen, showing one day's to-do list.
//
// Reached by clicking a day in the month grid. Lays out a header (back arrow +
// the date), the day's entries as checkbox / label / delete rows, and a text
// box to add a new one. It only builds widgets — all edits go back to the app
// as messages, which mutate the Store.

use std::sync::LazyLock;

use cosmic::iced::{Alignment, Length};
use cosmic::widget::{button, checkbox, column, container, icon, row, text, text_input};
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
    /// Commit the current draft as a new entry.
    Submit,
    /// Toggle the done flag on the entry at this index.
    Toggle(usize),
    /// Delete the entry at this index.
    Delete(usize),
}

/// Build the day view for `date`, reading entries from `store` and showing
/// `draft` in the add box. `time` is the current time string, formatted by the
/// app to honor the 12/24h setting, so the header's third line matches the
/// month screen.
pub fn view<'a>(
    date: Date,
    store: &'a Store,
    draft: &'a str,
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
            list = list.push(entry_row(index, &entry.text, entry.done));
        }
    }

    let add = text_input("Add a to-do…", draft)
        .id(INPUT_ID.clone())
        .on_input(DayMessage::Input)
        .on_submit(|_| DayMessage::Submit)
        .width(Length::Fill);

    column::with_capacity(3)
        .push(header)
        .push(list)
        .push(add)
        .spacing(spacing.space_s)
        .width(Length::Fill)
        .into()
}

/// One entry row: checkbox toggles done, label shows the text, trash deletes.
fn entry_row<'a>(index: usize, label: &'a str, done: bool) -> Element<'a, DayMessage> {
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

    row::with_capacity(3)
        .push(check)
        .push(text_widget)
        .push(delete)
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