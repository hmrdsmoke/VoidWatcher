// GPL-3.0-or-later - see LICENSE file for full terms
// Copyright 2026 Michael Van Auker (HMRDSmoke)
// Do not remove these comments.
// VoidWatcher/src/ics.rs
// src/ics.rs
// iCalendar (.ics) import - the parser behind "open this invite with Void
// Watcher".
//
// An .ics file is plain text, one `NAME;PARAM=VALUE:VALUE` line per property,
// grouped into BEGIN/END blocks. A meeting invite from Outlook, Google, Zoom or
// Teams is one of these with a VEVENT inside. This module turns such a file
// into `store::Imported` items - the shape `Store::import` judges against what's
// already saved - and stays deliberately small: it reads the handful of
// properties a to-do needs (what, when, how often, which meeting this is) and
// ignores the rest. No crate; the format is simple enough by hand, and jiff
// does every bit of date and time-zone math.
//
// What it reads per VEVENT / VTODO:
//   UID, SEQUENCE       the meeting's identity and version, for updates/cancels
//   SUMMARY             the text
//   DTSTART / DUE       the date, and the time if it has one (four shapes: an
//                       all-day date; a floating local time; a UTC time with a
//                       trailing Z; a time in a named zone via TZID=)
//   RRULE               DAILY / WEEKLY / MONTHLY / YEARLY with INTERVAL, COUNT
//                       and UNTIL; the two monthly shapes (day-of-month, nth
//                       weekday); anything else falls back to a one-off
//   EXDATE              occurrences that don't happen
//   RECURRENCE-ID       "this one occurrence moved": exclude the original date
//                       from the series and import the new date as a one-off
//   STATUS / METHOD     CANCELLED / CANCEL remove; COMPLETED (VTODO) is done
//
// Everything else - DTEND, LOCATION, DESCRIPTION, attendees, alarms - is
// skipped on purpose. A to-do is one line and one time.

use std::path::Path;

use jiff::civil::{Date, DateTime};
use jiff::tz::TimeZone;

use crate::store::{ImportReport, Imported, Repeat, Store};

/// Read an .ics file, apply it to `store`, and say what happened. The one
/// error is an unreadable file; a file with nothing usable in it is an empty
/// report, not an error.
pub fn import_file(path: &Path, store: &mut Store) -> Result<ImportReport, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("couldn't read {}: {e}", path.display()))?;
    let items = parse(&text, &TimeZone::system());
    Ok(store.import(&items))
}

/// Notification text for an import: (summary, body). `file` is the file's
/// display name.
pub fn summary(report: &ImportReport, file: &str) -> (String, String) {
    fn list(texts: &[String]) -> String {
        const SHOW: usize = 3;
        let mut s = texts.iter().take(SHOW).cloned().collect::<Vec<_>>().join(", ");
        if texts.len() > SHOW {
            s.push_str(&format!(" +{} more", texts.len() - SHOW));
        }
        s
    }

    let mut lines = Vec::new();
    if !report.added.is_empty() {
        lines.push(format!("Added: {}", list(&report.added)));
    }
    if !report.updated.is_empty() {
        lines.push(format!("Updated: {}", list(&report.updated)));
    }
    if !report.cancelled.is_empty() {
        lines.push(format!("Cancelled: {}", list(&report.cancelled)));
    }
    if !report.fallback.is_empty() {
        lines.push(format!(
            "Imported as one-off (rule not supported): {}",
            list(&report.fallback)
        ));
    }

    let counts = [
        (report.added.len(), "Added"),
        (report.updated.len(), "Updated"),
        (report.cancelled.len(), "Cancelled"),
    ];
    let nonzero: Vec<_> = counts.iter().filter(|(n, _)| *n > 0).collect();
    let title = match nonzero.as_slice() {
        [] => {
            if report.skipped > 0 {
                return (format!("Nothing new in {file}"), "Already up to date.".to_owned());
            }
            return (format!("Nothing to import in {file}"), "No events found.".to_owned());
        }
        [(n, verb)] => format!("{verb} {n} from {file}"),
        _ => format!("Imported {file}"),
    };
    (title, lines.join("\n"))
}

/// Parse .ics text into import items. `local` is the zone the to-dos live in
/// (the system zone in practice; explicit so tests are deterministic). Items
/// with no usable date are dropped.
pub fn parse(text: &str, local: &TimeZone) -> Vec<Imported> {
    let mut cancel_all = false;
    let mut components: Vec<Component> = Vec::new();
    let mut current: Option<Component> = None;
    // Nesting depth inside the current component (a VALARM inside a VEVENT),
    // and inside blocks we skip entirely at the calendar level (VTIMEZONE and
    // its STANDARD/DAYLIGHT children).
    let mut inner_depth = 0usize;
    let mut skip_depth = 0usize;

    for line in unfold(text) {
        let Some(prop) = parse_line(&line) else {
            continue;
        };
        let upper_value = prop.value.trim().to_ascii_uppercase();
        match prop.name.as_str() {
            "BEGIN" => match upper_value.as_str() {
                "VCALENDAR" => {}
                "VEVENT" | "VTODO" if current.is_none() && skip_depth == 0 => {
                    current = Some(Component {
                        kind: if upper_value == "VEVENT" { Kind::Event } else { Kind::Todo },
                        props: Vec::new(),
                    });
                    inner_depth = 0;
                }
                _ => {
                    if current.is_some() {
                        inner_depth += 1;
                    } else {
                        skip_depth += 1;
                    }
                }
            },
            "END" => match upper_value.as_str() {
                "VCALENDAR" => {}
                "VEVENT" | "VTODO" if current.is_some() && inner_depth == 0 => {
                    if let Some(done) = current.take() {
                        components.push(done);
                    }
                }
                _ => {
                    if current.is_some() {
                        inner_depth = inner_depth.saturating_sub(1);
                    } else {
                        skip_depth = skip_depth.saturating_sub(1);
                    }
                }
            },
            "METHOD" if current.is_none() => cancel_all = upper_value == "CANCEL",
            _ => {
                if let Some(component) = current.as_mut()
                    && inner_depth == 0
                {
                    component.props.push(prop);
                }
            }
        }
    }

    components
        .iter()
        .filter_map(|component| convert(component, cancel_all, local))
        .collect()
}

/// Which block a component came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Event,
    Todo,
}

/// One VEVENT or VTODO: its own properties, nested blocks already stripped.
struct Component {
    kind: Kind,
    props: Vec<Prop>,
}

impl Component {
    fn get(&self, name: &str) -> Option<&Prop> {
        self.props.iter().find(|p| p.name == name)
    }

    fn all<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Prop> + 'a {
        self.props.iter().filter(move |p| p.name == name)
    }

    fn text(&self, name: &str) -> Option<String> {
        self.get(name)
            .map(|p| p.value.trim().to_owned())
            .filter(|s| !s.is_empty())
    }
}

/// One property line, unfolded and split: `NAME;PARAM=VALUE:VALUE`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Prop {
    /// Upper-cased.
    name: String,
    /// Upper-cased keys; values with surrounding quotes removed.
    params: Vec<(String, String)>,
    value: String,
}

impl Prop {
    fn param(&self, key: &str) -> Option<&str> {
        self.params
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }
}

/// A parsed date, with its time if it had one, already in the local zone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct When {
    date: Date,
    minute: Option<u16>,
}

/// Turn one component into an import item, or `None` if it has no date.
fn convert(c: &Component, cancel_all: bool, local: &TimeZone) -> Option<Imported> {
    let uid = c.text("UID");
    let sequence = c
        .text("SEQUENCE")
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(0);
    let text = c
        .get("SUMMARY")
        .map(|p| unescape(&p.value))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "(untitled)".to_owned());
    let status = c.text("STATUS").unwrap_or_default().to_ascii_uppercase();
    let cancel = cancel_all || status == "CANCELLED";
    let done = c.kind == Kind::Todo && status == "COMPLETED";

    let when_prop = match c.kind {
        Kind::Event => c.get("DTSTART"),
        Kind::Todo => c.get("DUE").or_else(|| c.get("DTSTART")),
    };
    let when = when_prop.and_then(|p| parse_when(&p.value, p, local))?;

    // A single occurrence of a series, moved or cancelled on its own: keyed
    // under the series' UID plus the original date so a later update to this
    // same occurrence finds it, and carrying the hole to punch in the series.
    if let (Some(series_uid), Some(original)) = (
        &uid,
        c.get("RECURRENCE-ID")
            .and_then(|p| parse_when(&p.value, p, local)),
    ) {
        return Some(Imported {
            uid: Some(format!("{series_uid}#{}", original.date)),
            sequence,
            date: when.date,
            text,
            at_minute: when.minute,
            repeat: Repeat::None,
            interval: 1,
            until: None,
            exdates: Vec::new(),
            done,
            cancel,
            fallback: false,
            excludes_from: Some((series_uid.clone(), original.date)),
        });
    }

    let rule = match c.get("RRULE") {
        Some(p) => parse_rrule(&p.value, when.date, local),
        None => Rule::once(),
    };
    let exdates = if rule.fallback {
        Vec::new()
    } else {
        c.all("EXDATE")
            .flat_map(|p| {
                p.value
                    .split(',')
                    .filter_map(|v| parse_when(v, p, local))
                    .map(|w| w.date)
                    .collect::<Vec<_>>()
            })
            .collect()
    };

    Some(Imported {
        uid,
        sequence,
        date: when.date,
        text,
        at_minute: when.minute,
        repeat: rule.repeat,
        interval: rule.interval,
        until: rule.until,
        exdates,
        done,
        cancel,
        fallback: rule.fallback,
        excludes_from: None,
    })
}

/// What an RRULE came out as.
struct Rule {
    repeat: Repeat,
    interval: u8,
    until: Option<Date>,
    /// The rule couldn't be expressed; `repeat` is `None` and the item is a
    /// one-off on its first date.
    fallback: bool,
}

impl Rule {
    fn once() -> Self {
        Self {
            repeat: Repeat::None,
            interval: 1,
            until: None,
            fallback: false,
        }
    }

    fn unsupported() -> Self {
        Self {
            fallback: true,
            ..Self::once()
        }
    }
}

/// Map an RRULE onto `Repeat`. `start` is the DTSTART date, which several
/// rule shapes lean on (a plain MONTHLY means "on DTSTART's day-of-month").
///
/// Supported: FREQ=DAILY/WEEKLY/YEARLY with no BY-parts beyond ones that just
/// restate DTSTART (Google puts BYDAY=FR on a Friday weekly; Outlook puts
/// BYMONTH/BYMONTHDAY on a yearly), FREQ=MONTHLY plain or with a single
/// BYMONTHDAY, or with one ordinal BYDAY ("2TU", "-1FR") - also spelled the
/// Outlook way as BYDAY=TU;BYSETPOS=2. INTERVAL, COUNT and UNTIL on any of
/// those. Everything else is unsupported and imports as a one-off.
fn parse_rrule(value: &str, start: Date, local: &TimeZone) -> Rule {
    let mut freq = String::new();
    let mut interval: u32 = 1;
    let mut count: Option<u32> = None;
    let mut until: Option<Date> = None;
    let mut byday: Vec<(Option<i8>, i8)> = Vec::new();
    let mut bymonthday: Vec<i8> = Vec::new();
    let mut bymonth: Vec<i8> = Vec::new();
    let mut bysetpos: Vec<i8> = Vec::new();

    for part in value.split(';') {
        let Some((key, val)) = part.split_once('=') else {
            continue;
        };
        let val = val.trim();
        match key.trim().to_ascii_uppercase().as_str() {
            "FREQ" => freq = val.to_ascii_uppercase(),
            "INTERVAL" => interval = val.parse().unwrap_or(0),
            "COUNT" => count = val.parse().ok(),
            "UNTIL" => until = parse_when_value(val, None, local).map(|w| w.date),
            "BYDAY" => {
                for token in val.split(',') {
                    match parse_byday(token) {
                        Some(day) => byday.push(day),
                        None => return Rule::unsupported(),
                    }
                }
            }
            "BYMONTHDAY" => bymonthday = val.split(',').filter_map(|v| v.parse().ok()).collect(),
            "BYMONTH" => bymonth = val.split(',').filter_map(|v| v.parse().ok()).collect(),
            "BYSETPOS" => bysetpos = val.split(',').filter_map(|v| v.parse().ok()).collect(),
            // Week start doesn't change any rule we support.
            "WKST" => {}
            // BYHOUR, BYMINUTE, BYSECOND, BYYEARDAY, BYWEEKNO, RSCALE, ...
            _ => return Rule::unsupported(),
        }
    }

    if !(1..=255).contains(&interval) {
        return Rule::unsupported();
    }
    let interval = interval as u8;
    let start_weekday = start.weekday().to_monday_one_offset();

    let repeat = match freq.as_str() {
        "DAILY" => {
            if !byday.is_empty() || !bymonthday.is_empty() || !bymonth.is_empty() {
                return Rule::unsupported();
            }
            Repeat::Daily
        }
        "WEEKLY" => {
            // A single BYDAY that just names DTSTART's weekday is the same rule.
            let restates_start = match byday.as_slice() {
                [] => true,
                [(None, weekday)] => *weekday == start_weekday,
                _ => false,
            };
            if !restates_start || !bymonthday.is_empty() || !bymonth.is_empty() {
                return Rule::unsupported();
            }
            Repeat::Weekly
        }
        "MONTHLY" => {
            if !bymonth.is_empty() {
                return Rule::unsupported();
            }
            match (byday.as_slice(), bymonthday.as_slice(), bysetpos.as_slice()) {
                ([], [], []) => Repeat::Monthly { day: start.day() },
                ([], [day], []) if (1..=31).contains(day) => Repeat::Monthly { day: *day },
                ([(Some(nth), weekday)], [], []) => Repeat::MonthlyWeekday {
                    nth: *nth,
                    weekday: *weekday,
                },
                ([(None, weekday)], [], [nth]) => Repeat::MonthlyWeekday {
                    nth: *nth,
                    weekday: *weekday,
                },
                _ => return Rule::unsupported(),
            }
        }
        "YEARLY" => {
            // BYMONTH / BYMONTHDAY that restate DTSTART are fine; anything
            // that would pick a different day (BYDAY, other months) isn't.
            let month_ok = bymonth.is_empty() || bymonth == [start.month()];
            let day_ok = bymonthday.is_empty() || bymonthday == [start.day()];
            if !byday.is_empty() || !month_ok || !day_ok || !bysetpos.is_empty() {
                return Rule::unsupported();
            }
            Repeat::Yearly
        }
        _ => return Rule::unsupported(),
    };

    // A rule with a positive nth must actually be reachable from DTSTART; a
    // sixth Tuesday never exists. Anything jiff can't express is unsupported.
    if let Repeat::MonthlyWeekday { nth, .. } = repeat
        && (nth == 0 || !(-5..=5).contains(&nth))
    {
        return Rule::unsupported();
    }

    // UNTIL wins if both are present (they shouldn't be). COUNT becomes the
    // date of the nth occurrence, walking the rule from DTSTART.
    let until = until.or_else(|| {
        count.and_then(|n| repeat.nth_occurrence(start, interval, n))
    });

    Rule {
        repeat,
        interval,
        until,
        fallback: false,
    }
}

/// One BYDAY token: an optional signed ordinal then a two-letter weekday,
/// e.g. "FR", "2TU", "-1FR". Weekday comes back Monday=1 .. Sunday=7.
fn parse_byday(token: &str) -> Option<(Option<i8>, i8)> {
    let token = token.trim().to_ascii_uppercase();
    if token.len() < 2 {
        return None;
    }
    let (ordinal, day) = token.split_at(token.len() - 2);
    let weekday = match day {
        "MO" => 1,
        "TU" => 2,
        "WE" => 3,
        "TH" => 4,
        "FR" => 5,
        "SA" => 6,
        "SU" => 7,
        _ => return None,
    };
    let ordinal = if ordinal.is_empty() {
        None
    } else {
        Some(ordinal.parse::<i8>().ok()?)
    };
    Some((ordinal, weekday))
}

/// Parse a date/time property value using the property's own parameters
/// (VALUE=DATE, TZID=...).
fn parse_when(value: &str, prop: &Prop, local: &TimeZone) -> Option<When> {
    parse_when_value(value, Some(prop), local)
}

/// The four shapes a date/time comes in, all landing in `local`:
///   20261005            all-day (or VALUE=DATE)      -> date, no time
///   20261005T143000     floating                    -> as written
///   20261005T183000Z    UTC                         -> converted to local
///   TZID=Europe/London  named zone (via the param)  -> converted to local
fn parse_when_value(value: &str, prop: Option<&Prop>, local: &TimeZone) -> Option<When> {
    let value = value.trim();
    let is_date_only = prop.and_then(|p| p.param("VALUE")) == Some("DATE") || value.len() == 8;
    if is_date_only {
        return Date::strptime("%Y%m%d", value).ok().map(|date| When { date, minute: None });
    }

    let (stamp, utc) = match value.strip_suffix(['Z', 'z']) {
        Some(s) => (s, true),
        None => (value, false),
    };
    let datetime = DateTime::strptime("%Y%m%dT%H%M%S", stamp).ok()?;

    let zone = if utc {
        Some(TimeZone::UTC)
    } else {
        prop.and_then(|p| p.param("TZID")).and_then(resolve_zone)
    };
    let datetime = match zone {
        // Anchor in its own zone, then read the wall clock in ours.
        Some(zone) => datetime.to_zoned(zone).ok()?.with_time_zone(local.clone()).datetime(),
        // Floating (or a zone we can't name): take it as local wall-clock time.
        None => datetime,
    };
    let time = datetime.time();
    Some(When {
        date: datetime.date(),
        minute: Some(u16::from(time.hour() as u8) * 60 + u16::from(time.minute() as u8)),
    })
}

/// Find the zone a TZID names. IANA names resolve directly; Outlook writes
/// Windows names ("Eastern Standard Time"), which map through a short table;
/// some producers prefix IANA names with a path ("/mozilla.org/.../America/
/// New_York"), so the last two components are tried too. Unknown zones give
/// `None`, and the time is then read as local - right for the common case of
/// an invite from your own zone, and the documented limit otherwise.
fn resolve_zone(tzid: &str) -> Option<TimeZone> {
    let tzid = tzid.trim().trim_matches('"');
    if let Ok(zone) = TimeZone::get(tzid) {
        return Some(zone);
    }
    if let Some(name) = WINDOWS_ZONES
        .iter()
        .find(|(windows, _)| windows.eq_ignore_ascii_case(tzid))
        .map(|(_, iana)| *iana)
    {
        return TimeZone::get(name).ok();
    }
    let parts: Vec<&str> = tzid.split('/').filter(|s| !s.is_empty()).collect();
    if parts.len() >= 2 {
        let tail = format!("{}/{}", parts[parts.len() - 2], parts[parts.len() - 1]);
        if let Ok(zone) = TimeZone::get(&tail) {
            return Some(zone);
        }
    }
    None
}

/// Windows time-zone names Outlook puts in TZID, and the IANA zone each
/// means. The common ones; an invite from an unlisted zone reads as local.
const WINDOWS_ZONES: &[(&str, &str)] = &[
    ("Eastern Standard Time", "America/New_York"),
    ("Central Standard Time", "America/Chicago"),
    ("Mountain Standard Time", "America/Denver"),
    ("US Mountain Standard Time", "America/Phoenix"),
    ("Pacific Standard Time", "America/Los_Angeles"),
    ("Alaskan Standard Time", "America/Anchorage"),
    ("Hawaiian Standard Time", "Pacific/Honolulu"),
    ("Atlantic Standard Time", "America/Halifax"),
    ("Newfoundland Standard Time", "America/St_Johns"),
    ("Canada Central Standard Time", "America/Regina"),
    ("Central Standard Time (Mexico)", "America/Mexico_City"),
    ("Central America Standard Time", "America/Guatemala"),
    ("SA Pacific Standard Time", "America/Bogota"),
    ("E. South America Standard Time", "America/Sao_Paulo"),
    ("Argentina Standard Time", "America/Argentina/Buenos_Aires"),
    ("UTC", "UTC"),
    ("Coordinated Universal Time", "UTC"),
    ("GMT Standard Time", "Europe/London"),
    ("W. Europe Standard Time", "Europe/Berlin"),
    ("Romance Standard Time", "Europe/Paris"),
    ("Central Europe Standard Time", "Europe/Budapest"),
    ("Central European Standard Time", "Europe/Warsaw"),
    ("E. Europe Standard Time", "Europe/Chisinau"),
    ("GTB Standard Time", "Europe/Athens"),
    ("FLE Standard Time", "Europe/Kiev"),
    ("Russian Standard Time", "Europe/Moscow"),
    ("Israel Standard Time", "Asia/Jerusalem"),
    ("Egypt Standard Time", "Africa/Cairo"),
    ("South Africa Standard Time", "Africa/Johannesburg"),
    ("Arabian Standard Time", "Asia/Dubai"),
    ("India Standard Time", "Asia/Kolkata"),
    ("Singapore Standard Time", "Asia/Singapore"),
    ("China Standard Time", "Asia/Shanghai"),
    ("Tokyo Standard Time", "Asia/Tokyo"),
    ("Korea Standard Time", "Asia/Seoul"),
    ("AUS Eastern Standard Time", "Australia/Sydney"),
    ("AUS Central Standard Time", "Australia/Darwin"),
    ("W. Australia Standard Time", "Australia/Perth"),
    ("New Zealand Standard Time", "Pacific/Auckland"),
];

/// Join folded lines back together. iCalendar wraps long lines by breaking
/// them and starting the continuation with a single space or tab; both CRLF
/// and LF endings are accepted. Has to happen before any line is read, or a
/// long SUMMARY leaves its tail looking like a property of its own.
fn unfold(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for raw in text.split('\n') {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        if let Some(rest) = line.strip_prefix([' ', '\t'])
            && let Some(last) = out.last_mut()
        {
            last.push_str(rest);
            continue;
        }
        if !line.is_empty() {
            out.push(line.to_owned());
        }
    }
    out
}

/// Split `NAME;PARAM=VALUE;PARAM2="quoted:value":VALUE` at the first colon
/// that isn't inside quotes. Lines without a colon aren't properties.
fn parse_line(line: &str) -> Option<Prop> {
    let mut in_quotes = false;
    let mut split_at = None;
    for (i, ch) in line.char_indices() {
        match ch {
            '"' => in_quotes = !in_quotes,
            ':' if !in_quotes => {
                split_at = Some(i);
                break;
            }
            _ => {}
        }
    }
    let colon = split_at?;
    let head = &line[..colon];
    let value = &line[colon + 1..];

    let mut pieces = head.split(';');
    let name = pieces.next()?.trim().to_ascii_uppercase();
    if name.is_empty() {
        return None;
    }
    let params = pieces
        .filter_map(|piece| {
            let (key, val) = piece.split_once('=')?;
            Some((
                key.trim().to_ascii_uppercase(),
                val.trim().trim_matches('"').to_owned(),
            ))
        })
        .collect();

    Some(Prop {
        name,
        params,
        value: value.to_owned(),
    })
}

/// Undo iCalendar text escaping: `\,` `\;` `\\` and `\n`/`\N`. A newline in a
/// summary becomes a space, since a to-do is one line.
fn unescape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('n') | Some('N') => out.push(' '),
            Some(escaped) => out.push(escaped),
            None => out.push('\\'),
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ny() -> TimeZone {
        TimeZone::get("America/New_York").unwrap()
    }

    fn d(s: &str) -> Date {
        s.parse().unwrap()
    }

    fn one(text: &str) -> Imported {
        let items = parse(text, &ny());
        assert_eq!(items.len(), 1, "expected one item, got {items:?}");
        items.into_iter().next().unwrap()
    }

    #[test]
    fn all_day() {
        let it = one(include_str!("../tests/ics/allday.ics"));
        assert_eq!(it.date, d("2026-10-05"));
        assert_eq!(it.at_minute, None);
        assert_eq!(it.text, "Inventory day");
        assert_eq!(it.uid.as_deref(), Some("allday-1@fixtures"));
        assert_eq!(it.repeat, Repeat::None);
        assert!(!it.cancel && !it.fallback && !it.done);
    }

    #[test]
    fn floating_time_is_local() {
        let it = one(include_str!("../tests/ics/timed_local.ics"));
        assert_eq!(it.date, d("2026-10-05"));
        assert_eq!(it.at_minute, Some(14 * 60 + 30));
    }

    #[test]
    fn utc_time_converts_to_local() {
        // 18:30Z on Oct 5 is 14:30 in New York (EDT).
        let it = one(include_str!("../tests/ics/timed_utc.ics"));
        assert_eq!(it.date, d("2026-10-05"));
        assert_eq!(it.at_minute, Some(14 * 60 + 30));
    }

    #[test]
    fn tzid_converts_to_local_and_own_zone_is_untouched() {
        let items = parse(include_str!("../tests/ics/timed_tzid.ics"), &ny());
        assert_eq!(items.len(), 2);
        // 19:30 London (BST) on Oct 5 is 14:30 New York.
        assert_eq!(items[0].at_minute, Some(14 * 60 + 30));
        // Quoted TZID, same zone as local: 9:00 stays 9:00.
        assert_eq!(items[1].at_minute, Some(9 * 60));
    }

    #[test]
    fn windows_zone_name_resolves() {
        // Read in New York: 14:30 stays 14:30. Read in UTC: it's 18:30.
        let it = one(include_str!("../tests/ics/windows_tzid.ics"));
        assert_eq!(it.at_minute, Some(14 * 60 + 30));
        let items = parse(include_str!("../tests/ics/windows_tzid.ics"), &TimeZone::UTC);
        assert_eq!(items[0].at_minute, Some(18 * 60 + 30));
    }

    #[test]
    fn weekly_with_exdate() {
        let it = one(include_str!("../tests/ics/weekly_exdate.ics"));
        assert_eq!(it.repeat, Repeat::Weekly);
        assert_eq!(it.interval, 1);
        assert_eq!(it.exdates, vec![d("2026-10-16")]);
        assert_eq!(it.until, None);
        assert!(!it.fallback);
    }

    #[test]
    fn monthly_by_day_plain_and_explicit() {
        let items = parse(include_str!("../tests/ics/monthly_byday.ics"), &ny());
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].repeat, Repeat::Monthly { day: 15 });
        assert_eq!(items[1].repeat, Repeat::Monthly { day: 15 });
    }

    #[test]
    fn monthly_second_tuesday() {
        let it = one(include_str!("../tests/ics/monthly_2tu.ics"));
        assert_eq!(it.repeat, Repeat::MonthlyWeekday { nth: 2, weekday: 2 });
        assert_eq!(it.at_minute, Some(18 * 60));
    }

    #[test]
    fn monthly_last_friday_both_spellings() {
        let items = parse(include_str!("../tests/ics/monthly_last_fri.ics"), &ny());
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].repeat, Repeat::MonthlyWeekday { nth: -1, weekday: 5 });
        assert_eq!(items[1].repeat, Repeat::MonthlyWeekday { nth: -1, weekday: 5 });
    }

    #[test]
    fn count_becomes_until() {
        // Six Fridays from Oct 2: Oct 2, 9, 16, 23, 30, Nov 6.
        let it = one(include_str!("../tests/ics/count.ics"));
        assert_eq!(it.repeat, Repeat::Weekly);
        assert_eq!(it.until, Some(d("2026-11-06")));
    }

    #[test]
    fn until_in_utc_lands_on_local_date() {
        // 03:59:59Z on Oct 31 is 23:59:59 on Oct 30 in New York.
        let it = one(include_str!("../tests/ics/until.ics"));
        assert_eq!(it.until, Some(d("2026-10-30")));
    }

    #[test]
    fn interval_every_other_week_and_every_third_month() {
        let items = parse(include_str!("../tests/ics/interval.ics"), &ny());
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].repeat, Repeat::Weekly);
        assert_eq!(items[0].interval, 2);
        assert_eq!(items[1].repeat, Repeat::Monthly { day: 1 });
        assert_eq!(items[1].interval, 3);
    }

    #[test]
    fn unsupported_rule_falls_back_to_one_off() {
        let it = one(include_str!("../tests/ics/exotic.ics"));
        assert!(it.fallback);
        assert_eq!(it.repeat, Repeat::None);
        assert_eq!(it.date, d("2026-10-05"));
        assert_eq!(it.until, None);
        assert!(it.exdates.is_empty());
    }

    #[test]
    fn moved_occurrence_carries_hole_and_new_date() {
        let items = parse(include_str!("../tests/ics/moved_occurrence.ics"), &ny());
        assert_eq!(items.len(), 2);
        let series = &items[0];
        assert_eq!(series.uid.as_deref(), Some("standup@fixtures"));
        assert_eq!(series.repeat, Repeat::Weekly);
        let moved = &items[1];
        assert_eq!(moved.uid.as_deref(), Some("standup@fixtures#2026-10-09"));
        assert_eq!(moved.date, d("2026-10-10"));
        assert_eq!(moved.at_minute, Some(15 * 60));
        assert_eq!(moved.repeat, Repeat::None);
        assert_eq!(moved.sequence, 1);
        assert_eq!(
            moved.excludes_from,
            Some(("standup@fixtures".to_owned(), d("2026-10-09")))
        );
    }

    #[test]
    fn vtodo_uses_due_and_completed_is_done() {
        let items = parse(include_str!("../tests/ics/vtodo.ics"), &ny());
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].date, d("2026-10-12"));
        assert_eq!(items[0].at_minute, None);
        assert!(!items[0].done);
        assert_eq!(items[1].date, d("2026-10-13"));
        assert_eq!(items[1].at_minute, Some(17 * 60));
        assert!(items[1].done);
    }

    #[test]
    fn folded_lines_and_escapes_with_crlf() {
        let it = one(include_str!("../tests/ics/folded.ics"));
        assert_eq!(
            it.text,
            "Lunch with the Smiths, the Joneses; then coffee - a summary long enough that the producer folded it across two lines second line"
        );
    }

    #[test]
    fn three_events_skip_timezone_block_and_alarm() {
        let items = parse(include_str!("../tests/ics/three_events.ics"), &ny());
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].text, "One");
        assert_eq!(items[0].sequence, 2);
        assert_eq!(items[0].at_minute, Some(9 * 60));
        assert_eq!(items[1].text, "Two");
        assert_eq!(items[1].at_minute, None);
        assert_eq!(items[2].text, "Three");
        // 10:00Z on Oct 7 is 06:00 in New York.
        assert_eq!(items[2].at_minute, Some(6 * 60));
    }

    #[test]
    fn invite_update_and_cancel_round_trip_through_store() {
        let mut store = Store::default();
        let r = store.import(&parse(include_str!("../tests/ics/invite.ics"), &ny()));
        assert_eq!(r.added, vec!["Dentist"]);
        assert_eq!(store.day(d("2026-09-29")).len(), 1);

        let r = store.import(&parse(include_str!("../tests/ics/invite_update.ics"), &ny()));
        assert_eq!(r.updated, vec!["Dentist"]);
        assert_eq!(store.day(d("2026-09-29")).len(), 0);
        assert_eq!(store.day(d("2026-10-01")).len(), 1);

        // The original arriving again is stale.
        let r = store.import(&parse(include_str!("../tests/ics/invite.ics"), &ny()));
        assert_eq!(r.skipped, 1);
        assert_eq!(store.day(d("2026-10-01")).len(), 1);

        let r = store.import(&parse(include_str!("../tests/ics/invite_cancel.ics"), &ny()));
        assert_eq!(r.cancelled, vec!["Dentist"]);
        assert_eq!(store.day(d("2026-10-01")).len(), 0);
    }

    #[test]
    fn cancelling_one_occurrence_punches_a_hole() {
        let mut store = Store::default();
        store.import(&parse(include_str!("../tests/ics/moved_occurrence.ics"), &ny()));
        assert_eq!(store.day(d("2026-10-16")).len(), 1);
        let items = parse(include_str!("../tests/ics/cancel_occurrence.ics"), &ny());
        assert!(items[0].cancel);
        assert_eq!(
            items[0].excludes_from,
            Some(("standup@fixtures".to_owned(), d("2026-10-16")))
        );
        let r = store.import(&items);
        assert_eq!(r.cancelled, vec!["Standup"]);
        assert_eq!(store.day(d("2026-10-16")).len(), 0);
        assert_eq!(store.day(d("2026-10-23")).len(), 1); // series continues
    }

    #[test]
    fn no_uid_imports_once() {
        let mut store = Store::default();
        let text = include_str!("../tests/ics/no_uid.ics");
        let r = store.import(&parse(text, &ny()));
        assert_eq!(r.added, vec!["Bare event"]);
        let r = store.import(&parse(text, &ny()));
        assert_eq!(r.skipped, 1);
        assert_eq!(store.day(d("2026-10-20")).len(), 1);
    }

    #[test]
    fn notification_text() {
        let mut r = ImportReport {
            added: vec!["Dentist".into()],
            ..ImportReport::default()
        };
        assert_eq!(
            summary(&r, "invite.ics"),
            ("Added 1 from invite.ics".to_owned(), "Added: Dentist".to_owned())
        );
        r.cancelled = vec!["Standup".into()];
        assert_eq!(summary(&r, "x.ics").0, "Imported x.ics");
        let mut r = ImportReport {
            skipped: 1,
            ..ImportReport::default()
        };
        assert_eq!(summary(&r, "x.ics").0, "Nothing new in x.ics");
        r.skipped = 0;
        assert_eq!(summary(&r, "x.ics").0, "Nothing to import in x.ics");
    }

    #[test]
    fn line_parsing_edge_cases() {
        let p = parse_line(r#"DTSTART;TZID="America/New_York":20261005T090000"#).unwrap();
        assert_eq!(p.name, "DTSTART");
        assert_eq!(p.param("TZID"), Some("America/New_York"));
        assert_eq!(p.value, "20261005T090000");
        assert!(parse_line("no colon here").is_none());
        assert_eq!(parse_byday("-1FR"), Some((Some(-1), 5)));
        assert_eq!(parse_byday("2tu"), Some((Some(2), 2)));
        assert_eq!(parse_byday("SU"), Some((None, 7)));
        assert_eq!(parse_byday("XX"), None);
        assert_eq!(unescape(r"a\,b\;c\\d\ne"), r"a,b;c\d e");
    }
}
