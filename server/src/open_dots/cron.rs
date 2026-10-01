//! Five-field cron (`minute hour day-of-month month day-of-week`) in the computer's local time,
//! the schedule format Open Dot's routines use (croner). Lists, ranges, steps, `*`, month and day
//! names, `7` for Sunday and the `@daily`-style shorthands; when both day fields are restricted a
//! day matches either, as classic cron (and croner's default) does.

use chrono::{
    DateTime, Datelike, Duration, Local, LocalResult, NaiveDate, NaiveDateTime, TimeZone, Timelike,
};

#[derive(Clone, Debug)]
pub(crate) struct CronSchedule {
    minutes: [bool; 60],
    hours: [bool; 24],
    days: [bool; 32],
    months: [bool; 13],
    weekdays: [bool; 7],
    days_restricted: bool,
    weekdays_restricted: bool,
}

const MONTH_NAMES: [&str; 12] = [
    "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
];
const DAY_NAMES: [&str; 7] = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"];

fn shorthand(expression: &str) -> Option<&'static str> {
    match expression.trim().to_ascii_lowercase().as_str() {
        "@yearly" | "@annually" => Some("0 0 1 1 *"),
        "@monthly" => Some("0 0 1 * *"),
        "@weekly" => Some("0 0 * * 0"),
        "@daily" | "@midnight" => Some("0 0 * * *"),
        "@hourly" => Some("0 * * * *"),
        _ => None,
    }
}

fn parse_value(text: &str, names: Option<(&[&str], u32)>) -> Result<u32, String> {
    let lower = text.trim().to_ascii_lowercase();
    if let Some((names, offset)) = names {
        if let Some(index) = names.iter().position(|name| *name == lower) {
            return Ok(index as u32 + offset);
        }
    }
    lower
        .parse::<u32>()
        .map_err(|_| format!("\"{text}\" is not a number"))
}

/// Fills `slots[min..=max]` from one field.
fn parse_field(
    field: &str,
    min: u32,
    max: u32,
    names: Option<(&[&str], u32)>,
    slots: &mut [bool],
) -> Result<bool, String> {
    let restricted = field.trim() != "*";
    for part in field.split(',') {
        let part = part.trim();
        if part.is_empty() {
            return Err("an empty list item".to_string());
        }
        let (range, step) = match part.split_once('/') {
            Some((range, step)) => {
                let step = step
                    .parse::<u32>()
                    .map_err(|_| format!("\"{step}\" is not a step"))?;
                if step == 0 {
                    return Err("a step of 0".to_string());
                }
                (range, step)
            }
            None => (part, 1),
        };
        let (start, end) = if range == "*" {
            (min, max)
        } else if let Some((from, to)) = range.split_once('-') {
            (parse_value(from, names)?, parse_value(to, names)?)
        } else {
            let value = parse_value(range, names)?;
            // `5/15` runs from 5 to the end in steps of 15.
            (value, if part.contains('/') { max } else { value })
        };
        if start < min || end > max || start > end {
            return Err(format!("\"{part}\" is outside {min}-{max}"));
        }
        let mut value = start;
        while value <= end {
            slots[value as usize] = true;
            // A step larger than the range (even one near u32::MAX) just ends the field.
            let Some(next) = value.checked_add(step) else {
                break;
            };
            value = next;
        }
    }
    Ok(restricted)
}

pub(crate) fn parse(expression: &str) -> Result<CronSchedule, String> {
    let expression = shorthand(expression).unwrap_or(expression);
    let fields: Vec<&str> = expression.split_whitespace().collect();
    if fields.len() != 5 {
        return Err("a schedule has five fields: minute hour day month weekday".to_string());
    }
    let mut schedule = CronSchedule {
        minutes: [false; 60],
        hours: [false; 24],
        days: [false; 32],
        months: [false; 13],
        weekdays: [false; 7],
        days_restricted: false,
        weekdays_restricted: false,
    };
    parse_field(fields[0], 0, 59, None, &mut schedule.minutes)?;
    parse_field(fields[1], 0, 23, None, &mut schedule.hours)?;
    schedule.days_restricted = parse_field(fields[2], 1, 31, None, &mut schedule.days)?;
    parse_field(
        fields[3],
        1,
        12,
        Some((&MONTH_NAMES, 1)),
        &mut schedule.months,
    )?;
    let mut weekdays = [false; 8];
    schedule.weekdays_restricted =
        parse_field(fields[4], 0, 7, Some((&DAY_NAMES, 0)), &mut weekdays)?;
    for (day, slot) in schedule.weekdays.iter_mut().enumerate() {
        *slot = weekdays[day] || (day == 0 && weekdays[7]);
    }
    if schedule.weekdays_restricted && fields[4].trim().starts_with('*') {
        // `*/2` restricts the weekdays even though it starts with `*`.
        schedule.weekdays_restricted = schedule.weekdays.iter().any(|slot| !slot);
    }
    if schedule.days_restricted && fields[2].trim().starts_with('*') {
        schedule.days_restricted = schedule.days[1..].iter().any(|slot| !slot);
    }
    Ok(schedule)
}

impl CronSchedule {
    fn day_matches(&self, date: NaiveDate) -> bool {
        if !self.months[date.month() as usize] {
            return false;
        }
        let day = self.days[date.day() as usize];
        let weekday = self.weekdays[date.weekday().num_days_from_sunday() as usize];
        match (self.days_restricted, self.weekdays_restricted) {
            (true, true) => day || weekday,
            (true, false) => day,
            (false, true) => weekday,
            (false, false) => true,
        }
    }

    /// The first matching minute strictly after `after`, walked in wall-clock (local) time.
    ///
    /// CDXC:OpenDots 2026-10-01 WHY:
    /// A run whose minute falls in a daylight-saving gap (02:30 on the spring-forward night) runs at the first minute after the gap, as cron does, instead of being skipped; the repeated hour in autumn runs once. The walk starts from the wall-clock minute after `after`, because adding a minute to an instant just before the gap lands past the whole gap.
    pub(crate) fn next_after(&self, after: DateTime<Local>) -> Option<DateTime<Local>> {
        let start = after.naive_local() + Duration::minutes(1);
        let start_date = start.date();
        // Five years covers every schedule that can ever run (29 February included).
        for offset in 0..(366 * 5) {
            let date = start_date + Duration::days(offset);
            if !self.day_matches(date) {
                continue;
            }
            let (from_hour, from_minute) = if offset == 0 {
                (start.hour(), start.minute())
            } else {
                (0, 0)
            };
            for hour in from_hour..24 {
                if !self.hours[hour as usize] {
                    continue;
                }
                let first_minute = if hour == from_hour { from_minute } else { 0 };
                for minute in first_minute..60 {
                    if !self.minutes[minute as usize] {
                        continue;
                    }
                    let naive = date.and_hms_opt(hour, minute, 0)?;
                    if let Some(local) = resolve_local(naive, after) {
                        return Some(local);
                    }
                }
            }
        }
        None
    }
}

/// A wall-clock minute as an instant after `after`: the first instant after a daylight-saving gap
/// when the minute does not exist, and the first of the two instants of a repeated minute that is
/// still after `after`.
fn resolve_local(naive: NaiveDateTime, after: DateTime<Local>) -> Option<DateTime<Local>> {
    match Local.from_local_datetime(&naive) {
        LocalResult::Single(local) => (local > after).then_some(local),
        LocalResult::Ambiguous(first, second) => {
            [first, second].into_iter().find(|local| *local > after)
        }
        LocalResult::None => (1..=180)
            .find_map(|minutes| {
                Local
                    .from_local_datetime(&(naive + Duration::minutes(minutes)))
                    .earliest()
            })
            .filter(|local| *local > after),
    }
}

/// Checks a schedule, with a customer-readable reason when it is not valid.
pub(crate) fn validate(expression: &str) -> Result<(), String> {
    let schedule = parse(expression)?;
    schedule
        .next_after(Local::now())
        .map(drop)
        .ok_or_else(|| "it never runs".to_string())
}

/// The next run after `after_ms`, in ms since the epoch.
pub(crate) fn next_run_ms(expression: &str, after_ms: i64) -> Option<i64> {
    let schedule = parse(expression).ok()?;
    let after = Local.timestamp_millis_opt(after_ms).single()?;
    schedule
        .next_after(after)
        .map(|time| time.timestamp_millis())
}

/// "Mon 6 Oct, 08:00" in the computer's time zone.
pub(crate) fn describe_time(ms: i64) -> String {
    Local
        .timestamp_millis_opt(ms)
        .single()
        .map(|time| time.format("%a %-d %b %Y, %H:%M").to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

/// The computer's time zone, as the dots are told to write cron in it.
pub(crate) fn local_zone_label() -> String {
    let offset = Local::now().offset().local_minus_utc();
    let sign = if offset < 0 { '-' } else { '+' };
    let offset = offset.abs();
    let zone = std::env::var("TZ")
        .ok()
        .filter(|zone| !zone.trim().is_empty())
        .or_else(|| {
            std::fs::read_link("/etc/localtime").ok().and_then(|link| {
                let link = link.to_string_lossy().to_string();
                link.split("zoneinfo/").nth(1).map(str::to_string)
            })
        });
    let utc = format!("UTC{sign}{:02}:{:02}", offset / 3600, (offset % 3600) / 60);
    match zone {
        Some(zone) => format!("{zone}, {utc}"),
        None => utc,
    }
}
