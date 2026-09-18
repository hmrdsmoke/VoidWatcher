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
use cosmic::widget::{Column, button, column, container, row, space, text, text_input};
use cosmic::{Element, surface};
use jiff::Zoned;
use jiff::civil::{Date, Weekday};
use jiff::fmt::strtime;

use crate::calendar::{self, MonthView};
use crate::config::{TIME_CONFIG_ID, TimeAppletConfig};
use crate::settings::Settings;
use cosmic::cosmic_config::ConfigSet;
use cosmic::cosmic_theme::Spacing;
use crate::day::{self, DayMessage};
use crate::easter_egg;
use crate::store::{Repeat, Store};

/// The popup's fixed inner width - the calendar grid's exact width (7 cells +
/// 6 gaps), shared by every screen so switching views never resizes the popup.
const POPUP_WIDTH: f32 = 380.0;

/// Base popup height, sized for the calendar's natural stack (three-line header
/// + 6-row grid + Today/Settings buttons) at COMPACT widget padding: a little
/// over what that stack needs so nothing clips and the box isn't loose. This is
/// the height when the system is Compact; taller densities add to it (see
/// `popup_height`). Nudge this if the calendar clips or leaves empty space.
const POPUP_HEIGHT_BASE: f32 = 520.0;

/// The popup height for the current SYSTEM interface density.
///
/// The one thing our own config can't shrink is the widgets' internal padding -
/// buttons and text inputs read libcosmic's process-global `CosmicTk`, which an
/// always-on disk watcher rewrites from the system config on any change, so a
/// value we seed never holds. On Standard/Spacious those widgets render taller,
/// and a fixed height would push the bottom row (the repeat pill on the day
/// view, the Settings button on the calendar) off the popup's edge. So instead
/// of fighting the widgets we measure them: read the system density and give
/// the box the extra room that density's inflation needs. Our own gaps stay
/// tight (see `popup_spacing`); this only grows the outer box to hold the
/// widgets the system sized.
fn popup_height() -> f32 {
    match cosmic::config::interface_density() {
        cosmic::cosmic_theme::Density::Compact => POPUP_HEIGHT_BASE,
        cosmic::cosmic_theme::Density::Standard => POPUP_HEIGHT_BASE + 80.0,
        cosmic::cosmic_theme::Density::Spacious => POPUP_HEIGHT_BASE + 180.0,
    }
}

/// The spacing Void Watcher lays out at: our OWN config's density, not the
/// system's. Kept in our namespace so nothing overwrites it, and deliberately
/// tight (defaults Compact) so the popup stays dense regardless of the system
/// setting. Every screen's gap values come from here.
fn popup_spacing(settings: &Settings) -> Spacing {
    settings.spacing()
}

/// Which screen the popup is showing: the month grid, or one day's to-do list.
/// The popup is a single surface that swaps between them, because an applet
/// can't put a second real window on screen - the panel is a nested compositor
/// and would swallow any toplevel it opened. A view swap gets the two-pane feel
/// without fighting that.
#[derive(Debug, Clone)]
enum Screen {
    Month,
    Day(Date),
    Settings,
    /// The hidden origin screen (see `easter_egg`).
    EasterEgg,
}

/// The application model stores app-specific state used to describe its
/// interface and drive its logic.
pub struct AppModel {
    /// Application state which is managed by the COSMIC runtime.
    core: Core,
    /// The popup id, while the popup is open.
    popup: Option<window::Id>,
    /// The stock time applet's settings, mirrored live from Settings.
    config: TimeAppletConfig,
    /// Void Watcher's own preferences (reminder defaults), loaded and saved by
    /// this applet. Kept live via a config subscription.
    settings: Settings,
    /// Wall clock, refreshed once a second by the tick subscription.
    now: Zoned,
    /// Any day within the month currently shown by the grid.
    visible: Date,
    /// The day highlighted by a left-click in the grid.
    selected: Option<Date>,
    /// The to-do entries, loaded once and saved on every edit.
    store: Store,
    /// Which screen the popup shows.
    screen: Screen,
    /// Text sitting in the day view's add box, held here so the input stays
    /// controlled across redraws.
    draft: String,
    /// Optional time sitting in the day view's picker (minutes since midnight),
    /// paired with `draft`. `None` = no explicit time (uses the daily default).
    /// Reset with `draft`.
    draft_minute: Option<u16>,
    /// The repeat rule sitting in the day view's picker, paired with `draft`.
    /// `Repeat::None` = a one-off. Reset with `draft`.
    draft_repeat: Repeat,
    /// How many periods of `draft_repeat` the draft's end date sits past its
    /// start: 0 = never ends. Kept as a step count rather than a date so
    /// switching the repeat (weekly to daily, say) keeps "three taps" meaning
    /// three periods of the new rule. Reset with `draft`.
    draft_end_steps: u32,
    /// Consecutive-tap counter for the hidden origin screen.
    egg: easter_egg::Trigger,
    /// The minute (0-59) the reminder check last ran, so it runs once a minute
    /// rather than every one-second tick. `None` until the first check.
    last_reminder_minute: Option<i8>,
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
    /// Void Watcher's own settings changed on disk (e.g. from the settings UI).
    SettingsChanged(Settings),
    /// A day was left-clicked in the grid: highlight it.
    HighlightDay(Date),
    /// A day was right-clicked in the grid: open its to-do list.
    OpenDay(Date),
    /// A message from the open day view.
    Day(DayMessage),
    /// Open the settings screen.
    OpenSettings,
    /// Leave the settings screen back to the calendar.
    CloseSettings,
    /// Leave the hidden origin screen back to the calendar.
    CloseEgg,
    /// Step the default reminder time's hour by +/-1 (wraps).
    SettingsHour(i32),
    /// Step the default reminder time's minute by +/-1 five-minute slot (wraps).
    SettingsMinute(i32),
    /// Step the reminder lead by +/-5 minutes (clamped at 0).
    SettingsLead(i32),
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
            settings: Settings::load(),
            visible: now.date(),
            now,
            selected: None,
            store: Store::load(),
            screen: Screen::Month,
            draft: String::new(),
            draft_minute: None,
            draft_repeat: Repeat::None,
            draft_end_steps: 0,
            egg: easter_egg::Trigger::default(),
            last_reminder_minute: None,
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

    /// The popup, showing whichever screen is current, at a fixed size
    /// (see POPUP_WIDTH/HEIGHT) so switching views never resizes the popup.
    fn view_window(&self, _id: window::Id) -> Element<'_, Message> {
        let screen: Element<'_, Message> = match self.screen {
            Screen::Month => self.month_screen(),
            Screen::Day(date) => {
                day::view(
                    date,
                    self.now.date(),
                    &self.store,
                    &self.draft,
                    self.draft_minute,
                    self.draft_repeat,
                    self.draft_repeat.end_after(date, self.draft_end_steps),
                    self.config.military_time,
                    popup_spacing(&self.settings),
                )
                .map(Message::Day)
            }
            Screen::Settings => self.settings_screen(),
            Screen::EasterEgg => {
                easter_egg::view(popup_spacing(&self.settings), Message::CloseEgg)
            }
        };
        let content = container(screen)
            .width(Length::Fixed(POPUP_WIDTH))
            .height(Length::Fixed(popup_height()))
            .padding([0, 4]);
        self.core.applet.popup_container(content).into()
    }

    fn subscription(&self) -> Subscription<Message> {
        Subscription::batch(vec![
            self.core
                .watch_config::<TimeAppletConfig>(TIME_CONFIG_ID)
                .map(|update| Message::ConfigChanged(update.config)),
            // Follow Void Watcher's own preferences live, so a settings change
            // takes effect without a restart.
            self.core
                .watch_config::<Settings>(crate::settings::CONFIG_ID)
                .map(|update| Message::SettingsChanged(update.config)),
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

                        let height = popup_height();
                        settings.positioner.size_limits = Limits::NONE
                            .min_width(POPUP_WIDTH)
                            .max_width(POPUP_WIDTH)
                            .min_height(height)
                            .max_height(height);
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
                // An .ics import runs as its own short-lived process and writes
                // the to-do file behind our back; when the file on disk is newer
                // than what we last read or wrote, pick it up. One stat a
                // second, and nothing else changes the file but us.
                if self.store.changed_on_disk() {
                    self.store = Store::load();
                }
                // Reminders fire on minute boundaries, so only scan when the
                // minute changes - not every one-second tick.
                let minute = self.now.minute();
                if self.last_reminder_minute != Some(minute) {
                    self.last_reminder_minute = Some(minute);
                    self.fire_due_reminders();
                }
            }
            Message::ConfigChanged(config) => {
                self.config = config;
            }
            Message::SettingsChanged(settings) => {
                self.settings = settings;
            }
            Message::HighlightDay(date) => {
                self.visible = date;
                self.selected = Some(date);
                if self.egg.tap(date) {
                    self.screen = Screen::EasterEgg;
                }
            }
            Message::OpenDay(date) => {
                self.visible = date;
                self.selected = Some(date);
                self.draft.clear();
                self.draft_minute = None;
                self.draft_repeat = Repeat::None;
                self.draft_end_steps = 0;
                self.screen = Screen::Day(date);
                return text_input::focus(day::INPUT_ID.clone());
            }
            Message::Day(day_message) => return self.update_day(day_message),
            Message::OpenSettings => {
                self.screen = Screen::Settings;
            }
            Message::CloseSettings => {
                self.screen = Screen::Month;
            }
            Message::CloseEgg => {
                self.screen = Screen::Month;
            }
            Message::SettingsHour(dir) => {
                let now = self.settings.default_reminder_minute;
                let hour = (now / 60) as i32;
                let minute = now % 60;
                let hour = (hour + dir).rem_euclid(24) as u16;
                self.set_default_reminder(hour * 60 + minute);
            }
            Message::SettingsMinute(dir) => {
                let now = self.settings.default_reminder_minute;
                let hour = now / 60;
                let slot = (now % 60) as i32 / 5;
                let slot = (slot + dir).rem_euclid(12);
                self.set_default_reminder(hour * 60 + (slot as u16) * 5);
            }
            Message::SettingsLead(dir) => {
                let cur = self.settings.reminder_lead_minutes as i32;
                let next = (cur + dir * 5).clamp(0, 720) as u16;
                self.set_reminder_lead(next);
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
        let spacing = popup_spacing(&self.settings);

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
        let spacing = popup_spacing(&self.settings);
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
                store: &self.store,
            },
            Message::HighlightDay,
            Message::OpenDay,
        );

        let today_button = button::text(crate::fl!("today")).on_press(Message::ThisMonth);
        let settings_button = button::text("Settings").on_press(Message::OpenSettings);

        // The month being browsed, always shown so navigating months is legible
        // without having to click a day. Tracks `visible` (the grid on screen),
        // not the selected day - that's the whole point, since the header's
        // date line follows the selection, not the navigation.
        let month_label = strtime::format("%B %Y", self.visible).unwrap_or_default();

        // Header pinned top, buttons pinned bottom, grid fills the middle. The
        // month grid is a fixed six rows tall; stacking everything at natural
        // height pushed the Settings button past the popup's fixed bottom edge.
        // Letting the grid area flex keeps both buttons on-screen regardless of
        // the active theme's spacing.
        column::with_capacity(5)
            .push(header)
            .push(text(month_label).size(14))
            .push(container(grid).height(Length::Fill).center_y(Length::Fill))
            .push(today_button)
            .push(settings_button)
            .spacing(spacing.space_s)
            .align_x(Alignment::Center)
            .into()
    }

    /// Persist the default reminder time (minutes since midnight), updating the
    /// in-memory copy immediately and writing it to Void Watcher's config.
    fn set_default_reminder(&mut self, minute: u16) {
        self.settings.default_reminder_minute = minute;
        if let Some(config) = crate::settings::Settings::config_handle() {
            let _ = config.set("default_reminder_minute", minute);
        }
    }

    /// Persist the reminder lead (minutes before a to-do's time).
    fn set_reminder_lead(&mut self, minutes: u16) {
        self.settings.reminder_lead_minutes = minutes;
        if let Some(config) = crate::settings::Settings::config_handle() {
            let _ = config.set("reminder_lead_minutes", minutes);
        }
    }

    /// The settings screen: a centered "Settings" header with a back button on
    /// the left, then the default reminder time (a two-stepper picker with no
    /// clear - it's always a time) and the reminder lead (a +/-5 stepper).
    fn settings_screen(&self) -> Element<'_, Message> {
        let spacing = popup_spacing(&self.settings);

        // Header: back button left, "Settings" centered, matched spacer right.
        let back = button::icon(cosmic::widget::icon::from_name("go-previous-symbolic").size(16))
            .on_press(Message::CloseSettings);
        let side = 44.0;
        let header = row::with_capacity(3)
            .push(container(back).width(Length::Fixed(side)))
            .push(
                container(text("Settings").size(20))
                    .width(Length::Fill)
                    .center_x(Length::Fill),
            )
            .push(space::horizontal().width(Length::Fixed(side)))
            .align_y(Alignment::Center);

        // Default reminder time: label + two-stepper (hour, minute) in a pill.
        let default_time = self.settings.default_reminder_minute;
        let time_inner = row::with_capacity(5)
            .push(
                button::icon(cosmic::widget::icon::from_name("list-remove-symbolic").size(14))
                    .on_press(Message::SettingsHour(-1)),
            )
            .push(
                button::icon(cosmic::widget::icon::from_name("list-add-symbolic").size(14))
                    .on_press(Message::SettingsHour(1)),
            )
            .push(
                container(text(day::time_label(Some(default_time), self.config.military_time)).size(14))
                    .width(Length::Fixed(84.0))
                    .center_x(Length::Fixed(84.0)),
            )
            .push(
                button::icon(cosmic::widget::icon::from_name("list-remove-symbolic").size(14))
                    .on_press(Message::SettingsMinute(-1)),
            )
            .push(
                button::icon(cosmic::widget::icon::from_name("list-add-symbolic").size(14))
                    .on_press(Message::SettingsMinute(1)),
            )
            .spacing(spacing.space_xxs)
            .align_y(Alignment::Center);
        let time_pill = container(
            container(time_inner)
                .padding([2, 6])
                .class(pill_class()),
        )
        .width(Length::Fill)
        .center_x(Length::Fill);
        let time_row = column::with_capacity(2)
            .push(text("Daily reminder time").size(14))
            .push(time_pill)
            .spacing(spacing.space_xxs)
            .align_x(Alignment::Center);

        // Reminder lead: label + +/-5 stepper showing "N min", in a pill.
        let lead = self.settings.reminder_lead_minutes;
        let lead_inner = row::with_capacity(3)
            .push(
                button::icon(cosmic::widget::icon::from_name("list-remove-symbolic").size(14))
                    .on_press(Message::SettingsLead(-1)),
            )
            .push(
                container(text(format!("{lead} min")).size(14))
                    .width(Length::Fixed(84.0))
                    .center_x(Length::Fixed(84.0)),
            )
            .push(
                button::icon(cosmic::widget::icon::from_name("list-add-symbolic").size(14))
                    .on_press(Message::SettingsLead(1)),
            )
            .spacing(spacing.space_xxs)
            .align_y(Alignment::Center);
        let lead_pill = container(
            container(lead_inner)
                .padding([2, 6])
                .class(pill_class()),
        )
        .width(Length::Fill)
        .center_x(Length::Fill);
        let lead_row = column::with_capacity(2)
            .push(text("Remind me this long before").size(14))
            .push(lead_pill)
            .spacing(spacing.space_xxs)
            .align_x(Alignment::Center);

        column::with_capacity(3)
            .push(header)
            .push(time_row)
            .push(lead_row)
            .spacing(spacing.space_m)
            .align_x(Alignment::Center)
            .into()
    }

    /// Send desktop notifications for every to-do whose reminder moment has
    /// arrived, and mark each so it fires exactly once. Called at most once a
    /// minute from the tick; the store decides what's due (see
    /// `Store::due_reminders`), including reminders missed while off.
    fn fire_due_reminders(&mut self) {
        for due in self.store.due_reminders(
            &self.now,
            self.settings.reminder_lead_minutes,
            self.settings.default_reminder_minute,
        ) {
            let body = format!(
                "Due at {}",
                day::time_label(Some(due.at_minute), self.config.military_time)
            );
            crate::notify::send(&due.text, &body);
            self.store
                .mark_notified(&due.date, due.index, &due.occurrence);
        }
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
                self.draft_minute = None;
                self.draft_repeat = Repeat::None;
                self.draft_end_steps = 0;
            }
            DayMessage::Input(text) => {
                self.draft = text;
            }
            DayMessage::HourUp => {
                self.draft_minute = day::step_hour(self.draft_minute, 1);
            }
            DayMessage::HourDown => {
                self.draft_minute = day::step_hour(self.draft_minute, -1);
            }
            DayMessage::MinuteUp => {
                self.draft_minute = day::step_minute(self.draft_minute, 1);
            }
            DayMessage::MinuteDown => {
                self.draft_minute = day::step_minute(self.draft_minute, -1);
            }
            DayMessage::ClearTime => {
                self.draft_minute = None;
            }
            DayMessage::CycleRepeat => {
                self.draft_repeat = self.draft_repeat.next();
                // Back to a one-off: an end date means nothing, drop it so it
                // can't ride along onto nothing.
                if self.draft_repeat == Repeat::None {
                    self.draft_end_steps = 0;
                }
            }
            DayMessage::EndsUp => {
                // Only step where the rule actually lands somewhere; a rule that
                // never occurs again has nowhere further to go.
                let next = self.draft_end_steps + 1;
                if self.draft_repeat.end_after(date, next).is_some() {
                    self.draft_end_steps = next;
                }
            }
            DayMessage::EndsDown => {
                self.draft_end_steps = self.draft_end_steps.saturating_sub(1);
            }
            DayMessage::Submit => {
                let at_minute = self.draft_minute;
                let repeat = self.draft_repeat;
                let until = repeat.end_after(date, self.draft_end_steps);
                self.store
                    .add(date, std::mem::take(&mut self.draft), at_minute, repeat, until);
                self.draft_minute = None;
                self.draft_repeat = Repeat::None;
                self.draft_end_steps = 0;
                return text_input::focus(day::INPUT_ID.clone());
            }
            // Toggle/Delete carry the row's position in the EXPANDED day list
            // (see day::view). A row may be a recurring item anchored on another
            // day, so resolve the row back to its source entry before mutating:
            // `source_date` is the entry's map key, `source_index` its slot
            // there, and `date` (the day being viewed) is the occurrence that
            // per-day completion keys on.
            DayMessage::Toggle(row) => {
                if let Some(item) = self.store.day(date).get(row) {
                    if let Ok(anchor) = item.source_date.parse::<Date>() {
                        self.store.toggle(anchor, item.source_index, date);
                    }
                }
            }
            DayMessage::Delete(row) => {
                if let Some(item) = self.store.day(date).get(row) {
                    if let Ok(anchor) = item.source_date.parse::<Date>() {
                        self.store.remove(anchor, item.source_index);
                    }
                }
            }
        }
        Task::none()
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

/// The subtle bordered-pill container class shared by the settings steppers,
/// matching the day view's time-picker pill. Returns the cosmic container class
/// (not a whole widget), so it composes without naming generic widget types.
fn pill_class() -> cosmic::style::Container<'static> {
    cosmic::style::Container::custom(|t| {
        let cosmic = t.cosmic();
        cosmic::iced::widget::container::Style {
            border: cosmic::iced::Border {
                radius: cosmic.corner_radii.radius_m.into(),
                width: 1.0,
                color: cosmic.palette.neutral_5.into(),
            },
            ..Default::default()
        }
    })
}

/// Move `date` by `months` whole months, landing on the 1st so day-of-month
/// clamping (Jan 31 -> Feb) never trips us up.
fn month_step(date: Date, months: i32) -> Date {
    date.first_of_month()
        .checked_add(jiff::Span::new().months(months))
        .unwrap_or(date)
}