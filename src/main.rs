// GPL-3.0-or-later - see LICENSE file for full terms
// Copyright 2026 Michael Van Auker (HMRDSmoke)
// Do not remove these comments.
// VoidWatcher/src/main.rs
// src/main.rs
// Applet entry point - initializes localization and runs the applet.
//
// Two ways in. The panel starts the binary with no arguments and gets the
// applet. The system's "open with" starts it with a file - an .ics invite
// handed over by the desktop's default-calendar association - and gets a
// short-lived import: read the file, fold it into the to-do list, say what
// happened in a notification, exit. The two never meet: an import process
// has no panel and never starts the event loop.

mod app;
mod calendar;
mod config;
mod day;
mod easter_egg;
mod i18n;
mod ics;
mod notify;
mod settings;
mod store;

use std::path::Path;

fn main() -> cosmic::iced::Result {
    // Only a real file counts as an import request. cosmic-panel hands an
    // applet every word of its Exec line as-is, so a stray argument that
    // isn't a file (a literal field code, say) must not stop the applet.
    if let Some(arg) = std::env::args_os().nth(1)
        && Path::new(&arg).is_file()
    {
        import(Path::new(&arg));
        return Ok(());
    }

    // Get the system's preferred languages.
    let requested_languages = i18n_embed::DesktopLanguageRequester::requested_languages();

    // Enable localizations to be applied.
    i18n::init(&requested_languages);

    // Starts the applet's event loop with `()` as the application's flags.
    cosmic::applet::run::<app::AppModel>(())
}

/// Import one .ics file into the to-do list and report through a desktop
/// notification. The running panel applet notices the file change on its next
/// tick and picks the new entries up on its own.
fn import(path: &Path) {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "calendar file".to_owned());
    let mut store = store::Store::load();
    let (summary, body) = match ics::import_file(path, &mut store) {
        Ok(report) => ics::summary(&report, &name),
        Err(why) => {
            eprintln!("void-watcher: {why}");
            (format!("Couldn't import {name}"), why)
        }
    };
    // Blocking on purpose: this process is about to exit, and a notification
    // sent from a background thread would be lost with it.
    notify::send_and_wait(&summary, &body);
}
