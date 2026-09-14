# Void Watcher

Date, time, and calendar applet for the COSMIC panel. Follows Settings, Date and Time.

## Features

Shows the date and time in your panel. Click it to open a calendar popup with month navigation. Click any day to open that day and add to-do items. Days with items are marked in the month view, so you can see what you've got at a glance. Clock formatting follows your COSMIC date and time settings, including twenty four hour time and seconds.

## Building

You'll need Rust and the COSMIC development dependencies.

Clone the repo, then run `make` for a debug build, or `make release` for a release build. Install with `sudo make install`, which puts the binary, desktop file, metainfo, and icon in place and updates the icon cache. Then open COSMIC Panel settings and add Void Watcher to your panel.

To remove it, run `sudo make uninstall`.

## License

GPL-3.0
