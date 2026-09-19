# Void Watcher

Date, time, and to-do calendar applet for the COSMIC panel. Follows your COSMIC Date and Time settings.

## Features

Shows the date and time in your panel. Click it to open a calendar popup with month navigation. Each day is a button — left-click to highlight it, right-click to open that day's to-do list.

On a day, you can add to-do items, check them off, and delete them. Give an item a time with the built-in picker (hours, and minutes in five-minute steps), or leave it without one. Days that have items are marked in the month view, so you can see what you've got at a glance.

Items with a time send you a desktop notification before they're due. How far before is up to you. An item with no set time reminds you once that day at a default time (9:00 AM out of the box). Missed a reminder because your machine was off? It fires once when you come back, so nothing slips by.

A to-do can repeat — tap the repeat pill when adding it to cycle daily, weekly, or yearly — and can be given a last day with the Ends pill beside it, which shows the date each tap lands on. Every occurrence is checked off on its own; deleting one removes the whole series.

Clock formatting — twelve or twenty-four hour, seconds, first day of the week — follows your COSMIC Date and Time settings, so the applet matches the rest of your desktop. The time picker respects your twelve/twenty-four hour choice too.

## Calendar invites

Void Watcher can be your default calendar app. Open COSMIC Settings › Applications › Default Applications and pick Void Watcher under Calendar. From then on, opening a calendar file — the `.ics` attached to a meeting invite from Outlook, Google Calendar, Zoom, or Teams — adds the meeting to that day's to-do list with its time and repeat rule, and a notification tells you what landed. The running applet picks it up on its own; nothing to restart.

Invites carry their own identity, so Void Watcher knows which meeting is which. When a meeting is rescheduled, open the update that arrives and the item moves rather than doubling. Open a cancellation and it's removed. Items that came from an invite show a small envelope, so you know they follow the organizer.

What it reads: the title; the date and time, whether all-day, local, UTC, or in a named time zone; and the repeat rule — daily, weekly, monthly (by day of the month or by rules like "second Tuesday"), or yearly, with every-Nth intervals, end dates, and skipped occurrences. What it leaves alone: end times, location, description, attendees. A to-do is one line and one time; open the invite itself for the rest.

Known limits:

- Nothing happens on its own. Void Watcher doesn't read your mail. You open the attachment, and that click does the right thing.
- A repeat rule it can't express (Monday, Wednesday and Friday, say) imports as a one-off on the first date, and the notification says so.
- A time zone it doesn't recognize is read as your local time. Standard zone names and Outlook's Windows names are recognized.

## Settings

There's a Settings button under the calendar. It opens a screen where you can set the daily reminder time (when items with no specific time nudge you) and how long before an item's time its reminder fires. Changes save right away.

## Building

You'll need Rust and the COSMIC development dependencies.

Clone the repo, then run `make` for a debug build, or `make release` for a release build. Install with `sudo make install`, which puts the binary, desktop files, metainfo, and icon in place and updates the icon cache. Then open COSMIC Panel settings and add Void Watcher to your panel. To have it open calendar invites, pick it under Calendar in Settings › Applications › Default Applications.

`cargo test` checks the invite parser and the to-do store against the sample calendar files in `tests/ics`.

To remove it, run `sudo make uninstall`.

## License

GPL-3.0