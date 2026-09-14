// GPL-3.0-or-later - see LICENSE file for full terms
// Copyright 2026 Michael Van Auker (HMRDSmoke)
// Do not remove these comments.
// VoidWatcher/src/app.rs
// src/app.rs
// COSMIC panel applet - application model, update loop, and view.

use std::time::Duration;

use cosmic::app::{Core, Task};
use cosmic::iced::{Alignment, Length, Limits, Rectangle, Subscription, window};
use cosmic::widget::rectangle_tracker::{
    RectangleTracker, RectangleUpdate, rectangle_tracker_subscription,
};
use cosmic::widget::{Column, button, column, container, row, space, text};
use cosmic::{Element, surface};
use jiff::Zoned;
use jiff::civil::{Date, Weekday};
use jiff::fmt::strtime;

use crate::calendar::{self, MonthView};
use crate::config::{TIME_CONFIG_ID, TimeAppletConfig};

/// The popup's fixed inner size, shared by every screen so switching views
/// never resizes or repositions the popup. Width is the calendar grid's exact
/// width (7 cells + 6 gaps). Height is a little over the calendar's natural
/// stack (three-line header + 6-row grid + Today button + spacing) so the
/// calendar fills it without clipping and other screens get the same roomy box.
/// Nudge `POPUP_HEIGHT` if the calendar clips or leaves too much empty space.
const POPUP_WIDTH: f32 = 380.0;
const POPUP_HEIGHT: f32 = 490.0;

/// The application model stores app-specific state used to describe its
/// interface and drive its logic.
pub struct AppModel {
    /// Application state which is managed by the COSMIC runtime.
    core: Core,
    /// The popup id, while the popup is open.
    popup: Option<window::Id>,
    /// The stock time applet's settings, mirrored live from Settings.
    config: TimeAppletConfig,
    /// Wall clock, refreshed once a second by the tick subscription.
    now: Zoned,
    /// Any day within the month currently shown by the grid.
    visible: Date,
    /// The day highlighted by a left-click in the grid.
    selected: Option<Date>,
    /// Handle to the panel's rectangle tracker, delivered once at startup.
    rectangle_tracker: Option<RectangleTracker<u32>>,
    /// The panel button's true on-screen rectangle, reported by the tracker.
    rectangle: Rectangle,
}

/// Messages emitted by the application and its widgets.
#[derive(Debug, Clone)]
pub enum Message {
    /// The panel button was pressed - open or close the popup.
    TogglePopup,
    /// An update from the panel's rectangle tracker.
    Rectangle(RectangleUpdate<u32>),
    PopupClosed(window::Id),
    Tick,
    ConfigChanged(TimeAppletConfig),
    /// A day was left-clicked in the grid: highlight it.
    HighlightDay(Date),
    PrevMonth,
    NextMonth,
    /// Jump the grid back to the current month.
    ThisMonth,
}

impl cosmic::Application for AppModel {
    type Executor = cosmic::executor::Default;
    type Flags = ();
    type Message = Message;

    const APP_ID: &'static str = "com.github.hmrdsmoke.void-watcher";

    fn core(&self) -> &Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut Core {
        &mut self.core
    }

    fn init(core: Core, _flags: Self::Flags) -> (Self, Task<Message>) {
        let now = Zoned::now();
        let app = AppModel {
            core,
            popup: None,
            config: TimeAppletConfig::load(),
            visible: now.date(),
            now,
            selected: None,
            rectangle_tracker: None,
            rectangle: Rectangle::default(),
        };

        (app, Task::none())
    }

    fn on_close_requested(&self, id: window::Id) -> Option<Message> {
        Some(Message::PopupClosed(id))
    }

    fn view(&self) -> Element<'_, Message> {
        let horizontal = self.core.applet.is_horizontal();

        let label: Element<'_, Message> = if horizontal {
            let fill_height = (self.core.applet.suggested_size(true).1
                + 2 * self.core.applet.suggested_padding(true).1)
                as f32;
            row(vec![
                self.core.applet.text(self.panel_label()).into(),
                container(space::vertical().height(Length::Fixed(fill_height))).into(),
            ])
            .align_y(Alignment::Center)
            .into()
        } else {
            self.stacked_label()
        };

        let (along, _across) = self.core.applet.suggested_padding(true);
        let padding = if horizontal { [0, along] } else { [along, 0] };

        let button = button::custom(label)
            .class(cosmic::theme::Button::AppletIcon)
            .padding(padding)
            .on_press_down(Message::TogglePopup);

        let content: Element<'_, Message> = match self.rectangle_tracker.as_ref() {
            Some(tracker) => tracker.container(0, button).ignore_bounds(true).into(),
            None => button.into(),
        };

        self.core.applet.autosize_window(content).into()
    }

    /// The popup, showing the month grid at a fixed size (see POPUP_WIDTH/HEIGHT).
    fn view_window(&self, _id: window::Id) -> Element<'_, Message> {
        let content = container(self.month_screen())
            .width(Length::Fixed(POPUP_WIDTH))
            .height(Length::Fixed(POPUP_HEIGHT))
            .padding([0, 4]);
        self.core.applet.popup_container(content).into()
    }

    fn subscription(&self) -> Subscription<Message> {
        Subscription::batch(vec![
            self.core
                .watch_config::<TimeAppletConfig>(TIME_CONFIG_ID)
                .map(|update| Message::ConfigChanged(update.config)),
            cosmic::iced::time::every(Duration::from_secs(1)).map(|_| Message::Tick),
            rectangle_tracker_subscription(0).map(|update| Message::Rectangle(update.1)),
        ])
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::TogglePopup => {
                if let Some(id) = self.popup.take() {
                    return surface::surface_task(surface::action::destroy_popup(id));
                }

                self.visible = self.now.date();
                self.selected = None;

                return surface::surface_task(surface::action::app_popup(
                    |_| Default::default(),
                    move |app: &mut Self| {
                        let parent = app
                            .core
                            .main_window_id()
                            .expect("applet main window exists before its popup opens");
                        let id = window::Id::unique();
                        app.popup = Some(id);

                        let mut settings =
                            app.core.applet.get_popup_settings(parent, id, None, None, None);

                        let Rectangle {
                            x,
                            y,
                            width,
                            height,
                        } = app.rectangle;
                        settings.positioner.anchor_rect = Rectangle::<i32> {
                            x: x.max(1.0) as i32,
                            y: y.max(1.0) as i32,
                            width: width.max(1.0) as i32,
                            height: height.max(1.0) as i32,
                        };

                        settings.positioner.size_limits = Limits::NONE
                            .min_width(POPUP_WIDTH)
                            .max_width(POPUP_WIDTH)
                            .min_height(POPUP_HEIGHT)
                            .max_height(POPUP_HEIGHT);
                        settings.positioner.size = None;
                        settings
                    },
                    None,
                ));
            }
            Message::PopupClosed(id) => {
                if self.popup == Some(id) {
                    self.popup = None;
                }
            }
            Message::Rectangle(update) => match update {
                RectangleUpdate::Rectangle((_, rect)) => {
                    self.rectangle = rect;
                }
                RectangleUpdate::Init(tracker) => {
                    self.rectangle_tracker = Some(tracker);
                }
            },
            Message::Tick => {
                self.now = Zoned::now();
            }
            Message::ConfigChanged(config) => {
                self.config = config;
            }
            Message::HighlightDay(date) => {
                self.visible = date;
                self.selected = Some(date);
            }
            Message::PrevMonth => {
                self.visible = month_step(self.visible, -1);
            }
            Message::NextMonth => {
                self.visible = month_step(self.visible, 1);
            }
            Message::ThisMonth => {
                self.visible = self.now.date();
            }
        }

        Task::none()
    }

    fn style(&self) -> Option<cosmic::iced::theme::Style> {
        Some(cosmic::applet::style())
    }
}

impl AppModel {
    fn header<'a>(
        &'a self,
        date: Date,
        left: Option<Element<'a, Message>>,
        right: Option<Element<'a, Message>>,
    ) -> Element<'a, Message> {
        let spacing = cosmic::theme::active().cosmic().spacing;

        let date_line = strtime::format("%B %-d, %Y", date).unwrap_or_default();
        let weekday_line = strtime::format("%A", date).unwrap_or_default();
        let time_line = strtime::format(self.time_format(), &self.now).unwrap_or_default();

        let center = column::with_capacity(3)
            .push(text(date_line).size(20))
            .push(text(weekday_line).size(14))
            .push(text(time_line).size(14))
            .align_x(Alignment::Center)
            .spacing(spacing.space_xxxs);

        cosmic::widget::row::with_capacity(3)
            .push(left.unwrap_or_else(|| space::horizontal().width(Length::Fixed(16.0)).into()))
            .push(center.width(Length::Fill))
            .push(right.unwrap_or_else(|| space::horizontal().width(Length::Fixed(16.0)).into()))
            .align_y(Alignment::Center)
            .spacing(spacing.space_xxs)
            .into()
    }

    fn time_format(&self) -> &'static str {
        match (self.config.military_time, self.config.show_seconds) {
            (true, true) => "%H:%M:%S",
            (true, false) => "%H:%M",
            (false, true) => "%-I:%M:%S %p",
            (false, false) => "%-I:%M %p",
        }
    }

    /// The month-grid screen: header with month-nav arrows, the grid, a Today button.
    fn month_screen(&self) -> Element<'_, Message> {
        let spacing = cosmic::theme::active().cosmic().spacing;
        let today = self.now.date();

        let focus = self.selected.unwrap_or(today);

        let prev = button::icon(cosmic::widget::icon::from_name("go-previous-symbolic").size(16))
            .on_press(Message::PrevMonth)
            .into();
        let next = button::icon(cosmic::widget::icon::from_name("go-next-symbolic").size(16))
            .on_press(Message::NextMonth)
            .into();
        let header = self.header(focus, Some(prev), Some(next));

        let grid = calendar::month(
            MonthView {
                visible: self.visible,
                today,
                selected: self.selected,
                first_weekday: self.first_weekday(),
            },
            Message::HighlightDay,
        );

        let today_button = button::text(crate::fl!("today")).on_press(Message::ThisMonth);

        column::with_capacity(3)
            .push(header)
            .push(grid)
            .push(today_button)
            .spacing(spacing.space_s)
            .align_x(Alignment::Center)
            .into()
    }

    fn panel_label(&self) -> String {
        if !self.config.format_strftime.is_empty() {
            if let Ok(text) = strtime::format(&self.config.format_strftime, &self.now) {
                return text;
            }
        }

        let mut format = String::with_capacity(24);

        if self.config.show_date_in_top_panel {
            if self.config.show_weekday {
                format.push_str("%a ");
            }
            format.push_str("%b %-d  ");
        }

        format.push_str(if self.config.military_time {
            "%H:%M"
        } else {
            "%-I:%M"
        });
        if self.config.show_seconds {
            format.push_str(":%S");
        }
        if !self.config.military_time {
            format.push_str(" %p");
        }

        strtime::format(&format, &self.now).unwrap_or_default()
    }

    fn stacked_label(&self) -> Element<'_, Message> {
        let label = self.panel_label();
        let lines = label
            .split_whitespace()
            .map(|word| self.core.applet.text(word.to_owned()).into())
            .collect::<Vec<Element<'_, Message>>>();

        let stacked = Column::with_children(lines)
            .align_x(Alignment::Center)
            .spacing(4);

        let fill_width = (self.core.applet.suggested_size(true).0
            + 2 * self.core.applet.suggested_padding(true).1)
            as f32;

        column(vec![
            stacked.into(),
            space::horizontal().width(Length::Fixed(fill_width)).into(),
        ])
        .align_x(Alignment::Center)
        .into()
    }

    fn first_weekday(&self) -> Weekday {
        i8::try_from(self.config.first_weekday_monday_zero())
            .ok()
            .and_then(|offset| Weekday::from_monday_zero_offset(offset).ok())
            .unwrap_or(Weekday::Sunday)
    }
}

/// Move `date` by `months` whole months, landing on the 1st so day-of-month
/// clamping (Jan 31 -> Feb) never trips us up.
fn month_step(date: Date, months: i32) -> Date {
    date.first_of_month()
        .checked_add(jiff::Span::new().months(months))
        .unwrap_or(date)
}