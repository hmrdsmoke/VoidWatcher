// src/app.rs
// COSMIC panel applet - application model, update loop, and view.

use std::time::Duration;

use cosmic::app::{Core, Task};
use cosmic::iced::{Alignment, Length, Limits, Rectangle, Subscription, window};
use cosmic::widget::rectangle_tracker::{
    RectangleTracker, RectangleUpdate, rectangle_tracker_subscription,
};
use cosmic::widget::{Column, button, column, container, row, space, text, text_input};
use cosmic::{Element, surface};
use jiff::Zoned;
use jiff::civil::{Date, Weekday};
use jiff::fmt::strtime;

use crate::calendar::{self, MonthView};
use crate::config::{TIME_CONFIG_ID, TimeAppletConfig};
use crate::day::{self, DayMessage};
use crate::store::Store;

/// Which screen the popup is showing. The popup is a single surface that swaps
/// between the month grid and one day's to-do list, because an applet can't put
/// a second real window on screen — the panel is a nested compositor and would
/// swallow any toplevel it opened. A view swap gets the same two-pane feel
/// without fighting that.
#[derive(Debug, Clone)]
enum Screen {
    /// The month grid.
    Month,
    /// One day's to-do list.
    Day(Date),
}

/// The application model stores app-specific state used to describe its
/// interface and drive its logic.
pub struct AppModel {
    /// Application state which is managed by the COSMIC runtime.
    core: Core,
    /// The popup id, while the popup is open.
    popup: Option<window::Id>,
    /// The stock time applet's settings, mirrored live from Settings › Date & Time.
    config: TimeAppletConfig,
    /// Wall clock, refreshed once a second by the tick subscription.
    now: Zoned,
    /// The to-do entries, loaded once and saved on every edit.
    store: Store,
    /// Which screen the popup shows.
    screen: Screen,
    /// Any day within the month currently shown by the grid.
    visible: Date,
    /// The day highlighted by a left-click in the grid — `None` until the user
    /// picks one, and reset each time the popup opens.
    selected: Option<Date>,
    /// Text sitting in the day view's "add" box, held here so the input stays
    /// controlled across redraws.
    draft: String,
    /// Handle to the panel's rectangle tracker, delivered once at startup.
    rectangle_tracker: Option<RectangleTracker<u32>>,
    /// The panel button's true on-screen rectangle, reported by the tracker.
    /// The popup anchors to this so it lands against the panel edge. Nested
    /// inside autosize with `ignore_bounds(true)` so it doesn't feed the
    /// autosize layout and cause a redraw loop.
    rectangle: Rectangle,
}

/// Messages emitted by the application and its widgets.
#[derive(Debug, Clone)]
pub enum Message {
    /// The panel button was pressed — open or close the popup.
    TogglePopup,
    /// An update from the panel's rectangle tracker.
    Rectangle(RectangleUpdate<u32>),
    PopupClosed(window::Id),
    Tick,
    ConfigChanged(TimeAppletConfig),
    /// A day was left-clicked in the grid: highlight it.
    HighlightDay(Date),
    /// A day was right-clicked in the grid: open its to-do list.
    OpenDay(Date),
    PrevMonth,
    NextMonth,
    /// Jump the grid back to the current month.
    ThisMonth,
    /// A message from the open day view.
    Day(DayMessage),
}

impl cosmic::Application for AppModel {
    /// The async executor that will be used to run your application's commands.
    type Executor = cosmic::executor::Default;

    /// Data that your application receives to its init method.
    type Flags = ();

    /// Messages which the application and its widgets will emit.
    type Message = Message;

    /// Unique identifier in RDNN (reverse domain name notation) format.
    const APP_ID: &'static str = "com.github.hmrdsmoke.void-watcher";

    fn core(&self) -> &Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut Core {
        &mut self.core
    }

    /// Initializes the application with any given flags and startup commands.
    fn init(core: Core, _flags: Self::Flags) -> (Self, Task<Message>) {
        let now = Zoned::now();
        let app = AppModel {
            core,
            popup: None,
            config: TimeAppletConfig::load(),
            visible: now.date(),
            now,
            store: Store::load(),
            screen: Screen::Month,
            selected: None,
            draft: String::new(),
            rectangle_tracker: None,
            rectangle: Rectangle::default(),
        };

        (app, Task::none())
    }

    fn on_close_requested(&self, id: window::Id) -> Option<Message> {
        Some(Message::PopupClosed(id))
    }

    /// The panel button: date and time text, formatted per the user's settings.
    fn view(&self) -> Element<'_, Message> {
        let horizontal = self.core.applet.is_horizontal();

        let label: Element<'_, Message> = if horizontal {
            // Pair the clock text with an invisible spacer forced to the full
            // panel thickness (cross-axis suggested size + both paddings). This
            // makes the button fill the panel top to bottom; without it the
            // button is only as tall as the glyphs, so the rectangle tracker
            // reports a short rect and the popup anchors partway up the panel
            // and overlaps it. This is exactly what stock cosmic-applet-time
            // does in its horizontal_layout.
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

        // Pad along the panel's long axis only; the panel already sets the
        // button's thickness across the short axis.
        let (along, _across) = self.core.applet.suggested_padding(true);
        let padding = if horizontal { [0, along] } else { [along, 0] };

        // The button reports its own layout when clicked: `bounds` is where it
        // was laid out, `offset` is any virtual scroll offset applied on top.
        // Subtracting gives the surface-relative rectangle the popup anchors to.
        let button = button::custom(label)
            .class(cosmic::theme::Button::AppletIcon)
            .padding(padding)
            .on_press_down(Message::TogglePopup);

        // Wrap the button in the panel's rectangle tracker so we learn its true
        // on-screen position for anchoring the popup. `ignore_bounds(true)` is
        // essential: it stops the tracker feeding its bounds back into the
        // autosize layout, which would loop redraws and get the process killed.
        // This is exactly how the stock cosmic-applet-time does it.
        let content: Element<'_, Message> = match self.rectangle_tracker.as_ref() {
            Some(tracker) => tracker.container(0, button).ignore_bounds(true).into(),
            None => button.into(),
        };

        // Lets the panel re-measure us when the label changes width (9:59 -> 10:00).
        self.core.applet.autosize_window(content).into()
    }

    /// The popup, showing whichever screen is current.
    fn view_window(&self, _id: window::Id) -> Element<'_, Message> {
        let content = match self.screen {
            Screen::Month => self.month_screen(),
            Screen::Day(date) => {
                let time = strtime::format(self.time_format(), &self.now).unwrap_or_default();
                day::view(date, &self.store, &self.draft, time).map(Message::Day)
            }
        };
        self.core.applet.popup_container(content).into()
    }

    /// Register subscriptions for this application.
    fn subscription(&self) -> Subscription<Message> {
        Subscription::batch(vec![
            // Follow Settings › Date & Time live; no restart needed.
            self.core
                .watch_config::<TimeAppletConfig>(TIME_CONFIG_ID)
                .map(|update| Message::ConfigChanged(update.config)),
            // One-second tick keeps the label at most a second stale, with or
            // without seconds showing. Cheap enough not to bother aligning.
            cosmic::iced::time::every(Duration::from_secs(1)).map(|_| Message::Tick),
            // Learn the panel button's on-screen rectangle for popup anchoring.
            rectangle_tracker_subscription(0).map(|update| Message::Rectangle(update.1)),
        ])
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::TogglePopup => {
                if let Some(id) = self.popup.take() {
                    return surface::surface_task(surface::action::destroy_popup(id));
                }

                // A fresh open always lands on the month grid for the current
                // month, with nothing highlighted yet.
                self.screen = Screen::Month;
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

                        // Anchor to the button's true on-screen rectangle from
                        // the tracker. This is what makes the popup sit against
                        // the panel edge instead of overlapping it. Straight
                        // through with a 1px floor, exactly as the stock applet.
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

                        // Size limits give the popup a real box; clearing `size`
                        // lets it size within those limits from content rather
                        // than being pinned to a fixed size.
                        settings.positioner.size_limits = Limits::NONE
                            .min_width(380.0)
                            .max_width(380.0)
                            .min_height(200.0)
                            .max_height(1080.0);
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
                // Left-click just marks the day and, if it's from an adjacent
                // month's greyed cell, brings that month into view.
                self.visible = date;
                self.selected = Some(date);
            }
            Message::OpenDay(date) => {
                // Right-click opens the day. Following a greyed day from an
                // adjacent month keeps the grid framing the day you opened.
                self.visible = date;
                self.selected = Some(date);
                self.draft.clear();
                self.screen = Screen::Day(date);
                return text_input::focus(day::INPUT_ID.clone());
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
            Message::Day(day_message) => return self.update_day(day_message),
        }

        Task::none()
    }

    fn style(&self) -> Option<cosmic::iced::theme::Style> {
        Some(cosmic::applet::style())
    }
}

impl AppModel {
    /// The three-line header block shared by both screens: the full date, the
    /// weekday under it, and the live time under that. `date` is whatever is
    /// currently in focus — the selected day on the month screen, or the open
    /// day on the day screen. `left` and `right` are the flanking controls
    /// (month-nav arrows, or the day-view back button), so the header row is
    /// identical in both places.
    fn header<'a>(
        &'a self,
        date: Date,
        left: Option<Element<'a, Message>>,
        right: Option<Element<'a, Message>>,
    ) -> Element<'a, Message> {
        let spacing = cosmic::theme::active().cosmic().spacing;

        // Date: "September 13, 2026". Weekday: "Sunday". Both from the focused
        // date, so they track whatever the user has clicked.
        let date_line = strtime::format("%B %-d, %Y", date).unwrap_or_default();
        let weekday_line = strtime::format("%A", date).unwrap_or_default();

        // Time follows the same 12/24h and seconds settings as the panel clock,
        // built live from the current instant.
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

    /// The strftime pattern for the header/panel time, honoring the user's
    /// 12/24h and show-seconds settings.
    fn time_format(&self) -> &'static str {
        match (self.config.military_time, self.config.show_seconds) {
            (true, true) => "%H:%M:%S",
            (true, false) => "%H:%M",
            (false, true) => "%-I:%M:%S %p",
            (false, false) => "%-I:%M %p",
        }
    }

    /// The month-grid screen: the shared header (with month-nav arrows), the
    /// weekday row and grid, then a Today button.
    fn month_screen(&self) -> Element<'_, Message> {
        let spacing = cosmic::theme::active().cosmic().spacing;
        let today = self.now.date();

        // The header shows the selected day, falling back to today when nothing
        // is highlighted yet (a fresh open).
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
                store: &self.store,
            },
            Message::HighlightDay,
            Message::OpenDay,
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

    /// Apply a day-view message to the store or navigate back.
    fn update_day(&mut self, message: DayMessage) -> Task<Message> {
        let Screen::Day(date) = self.screen else {
            return Task::none();
        };
        match message {
            DayMessage::Back => {
                self.screen = Screen::Month;
                self.draft.clear();
            }
            DayMessage::Input(text) => {
                self.draft = text;
            }
            DayMessage::Submit => {
                self.store.add(date, std::mem::take(&mut self.draft));
                return text_input::focus(day::INPUT_ID.clone());
            }
            DayMessage::Toggle(index) => {
                self.store.toggle(date, index);
            }
            DayMessage::Delete(index) => {
                self.store.remove(date, index);
            }
        }
        Task::none()
    }

    /// The panel label, built from the user's Date & Time settings.
    ///
    /// A user-set strftime overrides everything, exactly as the stock clock
    /// does, so anyone with a custom format sees identical text here.
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

    /// The label for a vertical panel: one line per word, since a vertical
    /// panel is only as wide as it is thick.
    fn stacked_label(&self) -> Element<'_, Message> {
        let label = self.panel_label();
        let lines = label
            .split_whitespace()
            .map(|word| self.core.applet.text(word.to_owned()).into())
            .collect::<Vec<Element<'_, Message>>>();

        let stacked = Column::with_children(lines)
            .align_x(Alignment::Center)
            .spacing(4);

        // Mirror of the horizontal case for a vertical panel: an invisible
        // spacer forced to the full panel width (cross-axis suggested size +
        // both paddings) makes the button fill the panel edge to edge, so the
        // rectangle tracker reports correct geometry and the popup anchors to
        // the panel edge. This is what stock cosmic-applet-time does in its
        // vertical_layout.
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

    /// First day of the week for the calendar grid, from the user's setting.
    fn first_weekday(&self) -> Weekday {
        i8::try_from(self.config.first_weekday_monday_zero())
            .ok()
            .and_then(|offset| Weekday::from_monday_zero_offset(offset).ok())
            .unwrap_or(Weekday::Sunday)
    }
}

/// Move `date` by `months` whole months, landing on the 1st so day-of-month
/// clamping (Jan 31 → Feb) never trips us up — the grid only cares which month
/// is shown, and `first_of_month` normalizes it anyway.
fn month_step(date: Date, months: i32) -> Date {
    date.first_of_month()
        .checked_add(jiff::Span::new().months(months))
        .unwrap_or(date)
}