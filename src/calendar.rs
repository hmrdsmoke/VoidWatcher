// GPL-3.0-or-later - see LICENSE file for full terms
// Copyright 2026 Michael Van Auker (HMRDSmoke)
// Do not remove these comments.
// VoidWatcher/src/calendar.rs
// src/calendar.rs
// The month grid shown in the popup.
//
// This is Void Watcher's own grid rather than libcosmic's `calendar` widget,
// because the built-in one can't do the two things this applet is about:
// mark days that have to-dos, and open a day on click. So the grid is built
// by hand out of a `grid` of day cells, each a `button` tile that highlights
// on hover. Layout is the only real work; jiff does all the date arithmetic.

use cosmic::iced::{Alignment, Background, Border, Length};
use cosmic::widget::{Grid, button, container, grid, text};
use cosmic::{Element, theme};
use jiff::civil::{Date, Weekday};

use crate::store::Store;


/// Six rows of seven always covers a month: the longest case is a 31-day month
/// whose 1st falls on the last column, needing 37 cells → 6 rows.
const ROWS: usize = 6;
const COLS: usize = 7;

/// Everything the grid needs to draw itself, so the view code stays declarative.
pub struct MonthView<'a> {
    /// Any day within the month being shown (day-of-month is ignored).
    pub visible: Date,
    /// The real today, drawn with an accent ring.
    pub today: Date,
    /// The day the user has open, drawn filled - `None` on the month view.
    pub selected: Option<Date>,
    /// Which weekday sits in column one, from the user's Date & Time setting.
    pub first_weekday: Weekday,
    /// To-do data, read only to decide which days get a dot.
    pub store: &'a Store,
}

/// One weekday-header + day grid for the visible month.
///
/// Two intents, two clicks: left-click highlights a day (browsing the month),
/// right-click opens that day's to-do list. Cells for days in the previous or
/// next month are dimmed but still respond, so interacting with the greyed "1"
/// of next month just carries you there.
pub fn month<'a, Message: Clone + 'static>(
    view: MonthView<'a>,
    on_highlight: impl Fn(Date) -> Message + 'a,
    on_open: impl Fn(Date) -> Message + 'a,
) -> Element<'a, Message> {
    // The date sitting in the top-left cell: walk back from the 1st of the
    // visible month to the most recent `first_weekday`. `since` gives the
    // signed day-count between two weekdays, which is exactly that offset.
    let first_of_month = view.visible.first_of_month();
    let lead = first_of_month.weekday().since(view.first_weekday).rem_euclid(7);
    let start = first_of_month
        .checked_sub(jiff::Span::new().days(i64::from(lead)))
        .unwrap_or(first_of_month);

    // 1px spacing is the gridline itself: the cells below paint an opaque base
    // color, and this gap lets the line-colored backdrop (see the wrapping
    // container at the end) show through as a uniform 1px rule between every
    // cell. Uniform because it's one backdrop, not per-cell borders doubling up.
    let mut grid: Grid<'a, Message> = grid()
        .column_spacing(1)
        .row_spacing(1);

    // Header row: weekday initials, in the user's chosen order.
    for col in 0..COLS {
        let weekday = view.first_weekday.wrapping_add(col as i8);
        grid = grid.push(weekday_heading(weekday_abbr(weekday)));
    }
    grid = grid.insert_row();

    // Six weeks of day cells.
    let mut day = start;
    for row in 0..ROWS {
        if row > 0 {
            grid = grid.insert_row();
        }
        for _ in 0..COLS {
            let in_month = day.month() == view.visible.month()
                && day.year() == view.visible.year();
            let cell = DayCell {
                date: day,
                in_month,
                is_today: day == view.today,
                is_selected: view.selected == Some(day),
                has_dot: view.store.has_entries(day),
            };
            grid = grid.push(
                cosmic::widget::mouse_area(day_button(cell, on_highlight(day)))
                    .on_right_press(on_open(day)),
            );
            day = day
                .tomorrow()
                .unwrap_or(day);
        }
    }

    // Wrap the grid in a container tinted with the line color, padded 1px so the
    // outer edge of the grid gets the same rule as the internal gaps. The cells
    // sit on top painting the base color, so what shows between and around them
    // is this backdrop: a closed, uniform 1px grid frame with no doubled lines.
    container(grid)
        .padding(1)
        .class(cosmic::style::Container::custom(|t| {
            cosmic::iced::widget::container::Style {
                background: Some(Background::Color(t.cosmic().palette.neutral_5.into())),
                ..Default::default()
            }
        }))
        .into()
}

/// Fields that decide how a single day cell looks.
struct DayCell {
    date: Date,
    in_month: bool,
    is_today: bool,
    is_selected: bool,
    has_dot: bool,
}

/// A weekday initial in the header row, dimmed and centered.
fn weekday_heading<'a, Message: 'a>(label: &'static str) -> Element<'a, Message> {
    container(text(label).size(12).class(cosmic::style::Text::Custom(|t| {
        cosmic::iced::widget::text::Style {
            color: Some(t.cosmic().palette.neutral_6.into()),
            ..Default::default()
        }
    })))
    .width(Length::Fixed(CELL))
    .center_x(Length::Fixed(CELL))
    // Opaque base like the day cells, so the header row reads as part of the
    // ruled grid rather than showing the backdrop through as gaps.
    .class(cosmic::style::Container::custom(|t| {
        cosmic::iced::widget::container::Style {
            background: Some(Background::Color(t.cosmic().palette.neutral_1.into())),
            ..Default::default()
        }
    }))
    .into()
}

/// Fixed cell size keeps the grid square and predictable across months. Wide
/// enough for the three-letter weekday headings (Mon, Tue, …) to sit centered
/// without crowding.
const CELL: f32 = 48.0;

/// One day as a clickable button tile: the number with a marker slot beneath,
/// wrapped in a `button::custom` so it gets real hover and press feedback. The
/// background/ring for today and the selected day live in the button's state
/// closures (see `day_button_class`); the number's *text* color still rides on
/// the text widget, since off-month muting depends on data the button class
/// doesn't see.
fn day_button<'a, Message: Clone + 'a>(
    cell: DayCell,
    on_press: Message,
) -> Element<'a, Message> {
    let number = text(cell.date.day().to_string()).size(14).class(number_style(&cell));

    // A tiny filled circle, or an equal-sized spacer when there's nothing, so
    // every cell is the same height whether or not it carries a dot.
    let marker: Element<'a, Message> = if cell.has_dot {
        container(
            cosmic::widget::Space::new()
                .width(Length::Fixed(5.0))
                .height(Length::Fixed(5.0)),
        )
            .class(cosmic::style::Container::custom(move |t| {
                let accent = t.cosmic().accent_color();
                cosmic::iced::widget::container::Style {
                    background: Some(Background::Color(accent.into())),
                    border: Border {
                        radius: 2.5.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                }
            }))
            .into()
    } else {
        cosmic::widget::Space::new()
            .width(Length::Fixed(5.0))
            .height(Length::Fixed(5.0))
            .into()
    };

    let inner = cosmic::widget::column::with_capacity(2)
        .push(number)
        .push(marker)
        .spacing(2)
        .align_x(Alignment::Center);

    button::custom(inner)
        .width(Length::Fixed(CELL))
        .height(Length::Fixed(CELL))
        .padding(0)
        .on_press(on_press)
        .class(day_button_class(&cell))
        .into()
}

/// Text color: normal in-month, muted off-month, accent on today, on-accent
/// when the day is the open one (so it reads against the filled background).
///
/// `Text::Custom` is a non-capturing `fn` pointer here, so the color is
/// resolved against the active theme up front and handed back as `Text::Color`.
fn number_style(cell: &DayCell) -> cosmic::style::Text {
    let theme = theme::active();
    let cosmic = theme.cosmic();
    let color = if cell.is_selected {
        cosmic.on_accent_color()
    } else if cell.is_today {
        cosmic.accent_text_color()
    } else if cell.in_month {
        cosmic.palette.neutral_9
    } else {
        cosmic.palette.neutral_5
    };
    cosmic::style::Text::Color(color.into())
}

/// The button styling for a day tile, across its interaction states.
///
/// For the ruled-grid look every cell paints an OPAQUE background (so the
/// line-colored backdrop only shows in the 1px gaps, not through the cell) and
/// uses SQUARE corners (rounded tiles would leave the backdrop peeking at the
/// corners where four cells meet, breaking the grid). Resting cells are the
/// theme base; hover/press wash a neutral over it; the selected day is filled
/// accent; today keeps an accent ring. All square, all opaque.
fn day_button_class(cell: &DayCell) -> cosmic::theme::Button {
    let is_today = cell.is_today;
    let is_selected = cell.is_selected;

    cosmic::theme::Button::Custom {
        active: Box::new(move |_selected, t| {
            let cosmic = t.cosmic();
            if is_selected {
                cosmic::widget::button::Style {
                    background: Some(Background::Color(cosmic.accent_color().into())),
                    ..Default::default()
                }
            } else if is_today {
                cosmic::widget::button::Style {
                    background: Some(Background::Color(cosmic.palette.neutral_1.into())),
                    border_width: 1.0,
                    border_color: cosmic.accent_color().into(),
                    ..Default::default()
                }
            } else {
                // Opaque base so the backdrop shows only in the gaps.
                cosmic::widget::button::Style {
                    background: Some(Background::Color(cosmic.palette.neutral_1.into())),
                    ..Default::default()
                }
            }
        }),
        hovered: Box::new(move |_selected, t| {
            let cosmic = t.cosmic();
            // Neutral wash over the base. Square, opaque, no radius.
            let bg = cosmic::iced::Color::from(cosmic.palette.neutral_4);
            let bg = cosmic::iced::Color { a: 0.35, ..bg };
            if is_selected {
                cosmic::widget::button::Style {
                    background: Some(Background::Color(cosmic.accent_color().into())),
                    ..Default::default()
                }
            } else {
                cosmic::widget::button::Style {
                    background: Some(Background::Color(bg)),
                    border_width: if is_today { 1.0 } else { 0.0 },
                    border_color: cosmic.accent_color().into(),
                    ..Default::default()
                }
            }
        }),
        pressed: Box::new(move |_selected, t| {
            let cosmic = t.cosmic();
            let bg = cosmic::iced::Color::from(cosmic.palette.neutral_5);
            let bg = cosmic::iced::Color { a: 0.45, ..bg };
            if is_selected {
                cosmic::widget::button::Style {
                    background: Some(Background::Color(cosmic.accent_color().into())),
                    ..Default::default()
                }
            } else {
                cosmic::widget::button::Style {
                    background: Some(Background::Color(bg)),
                    border_width: if is_today { 1.0 } else { 0.0 },
                    border_color: cosmic.accent_color().into(),
                    ..Default::default()
                }
            }
        }),
        disabled: Box::new(|_t| cosmic::widget::button::Style::default()),
    }
}

/// Three-letter weekday abbreviation for the header row. The order still comes
/// from the user's first-day-of-week setting.
fn weekday_abbr(weekday: Weekday) -> &'static str {
    match weekday {
        Weekday::Monday => "Mon",
        Weekday::Tuesday => "Tue",
        Weekday::Wednesday => "Wed",
        Weekday::Thursday => "Thu",
        Weekday::Friday => "Fri",
        Weekday::Saturday => "Sat",
        Weekday::Sunday => "Sun",
    }
}