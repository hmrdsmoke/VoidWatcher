# Void Watcher

Date, time, and to-do calendar applet for the COSMIC panel. Follows your COSMIC Date and Time settings.

## Features

Shows the date and time in your panel. Click it to open a calendar popup with month navigation. Each day is a button — left-click to highlight it, right-click to open that day's to-do list.

On a day, you can add to-do items, check them off, and delete them. Give an item a time with the built-in picker (hours, and minutes in five-minute steps), or leave it without one. Days that have items are marked in the month view, so you can see what you've got at a glance.

Items with a time send you a desktop notification before they're due. How far before is up to you. An item with no set time reminds you once that day at a default time (9:00 AM out of the box). Missed a reminder because your machine was off? It fires once when you come back, so nothing slips by.

Clock formatting — twelve or twenty-four hour, seconds, first day of the week — follows your COSMIC Date and Time settings, so the applet matches the rest of your desktop. The time picker respects your twelve/twenty-four hour choice too.

## Settings

There's a Settings button under the calendar. It opens a screen where you can set the daily reminder time (when items with no specific time nudge you) and how long before an item's time its reminder fires. Changes save right away.

## Building

You'll need Rust and the COSMIC development dependencies.

Clone the repo, then run `make` for a debug build, or `make release` for a release build. Install with `sudo make install`, which puts the binary, desktop file, metainfo, and icon in place and updates the icon cache. Then open COSMIC Panel settings and add Void Watcher to your panel.

To remove it, run `sudo make uninstall`.

## License

GPL-3.0