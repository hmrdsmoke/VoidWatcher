// GPL-3.0-or-later - see LICENSE file for full terms
// Copyright 2026 Michael Van Auker (HMRDSmoke)
// Do not remove these comments.
// VoidWatcher/src/notify.rs
// src/notify.rs
// Desktop notifications, sent from their own thread so nothing waits on D-Bus.
//
// A to-do reminder fires 30 minutes before its hour (see store::due_reminders).
// This module only knows how to *show* one - deciding what's due and marking it
// sent lives in the store and the app tick.

/// Show a reminder notification. Fire-and-forget on its own thread: the D-Bus
/// round-trip never blocks the panel's UI, and a failure is logged, not
/// propagated - a missed notification shouldn't take the applet down.
pub fn send(summary: &str, body: &str) {
    let summary = summary.to_owned();
    let body = body.to_owned();
    std::thread::spawn(move || show(&summary, &body));
}

/// Show a notification on the calling thread and return once it's been
/// handed to the notification service. For the one-shot import process,
/// which exits right after and would otherwise race its own notification.
pub fn send_and_wait(summary: &str, body: &str) {
    show(summary, body);
}

fn show(summary: &str, body: &str) {
    let result = notify_rust::Notification::new()
        .appname("Void Watcher")
        .summary(summary)
        .body(body)
        .icon("com.github.hmrdsmoke.void-watcher")
        .show();
    if let Err(e) = result {
        eprintln!("void-watcher: notification failed: {e}");
    }
}
