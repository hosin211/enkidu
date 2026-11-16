//! A moment said in the Mac's own time: a workflow's start as the clock shows it, "02:00", "tomorrow at 02:00".

/// A moment's date and time of day, in local time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Local {
    pub year: i32,
    /// 1 to 12.
    pub month: u32,
    pub day: u32,
    /// 0 is Sunday.
    pub weekday: u32,
    pub hour: u32,
    pub minute: u32,
}

/// The local date and time of a moment (milliseconds since 1970).
pub fn local(at: i64) -> Local {
    let seconds = at.div_euclid(1000) as libc::time_t;
    // SAFETY: localtime_r only writes into the tm we own
    let tm = unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&seconds, &mut tm);
        tm
    };
    Local {
        year: tm.tm_year + 1900,
        month: (tm.tm_mon + 1).max(1) as u32,
        day: tm.tm_mday.max(1) as u32,
        weekday: tm.tm_wday.clamp(0, 6) as u32,
        hour: tm.tm_hour.clamp(0, 23) as u32,
        minute: tm.tm_min.clamp(0, 59) as u32,
    }
}

/// The moment a local date and time of day names (milliseconds since 1970).
pub fn moment(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> i64 {
    // SAFETY: mktime only reads and normalises the tm we own
    let seconds = unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        tm.tm_year = year - 1900;
        tm.tm_mon = month as i32 - 1;
        tm.tm_mday = day as i32;
        tm.tm_hour = hour as i32;
        tm.tm_min = minute as i32;
        // let the system say whether summer time holds then
        tm.tm_isdst = -1;
        libc::mktime(&mut tm)
    };
    seconds as i64 * 1000
}

/// The day's number, counted from 1970 (Howard Hinnant's days from civil), to tell how many days lie between two
/// local dates.
fn day_number(date: &Local) -> i64 {
    let (y, m, d) = (date.year as i64, date.month as i64, date.day as i64);
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

const WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// The clock time of a moment, "02:00".
pub fn clock_of(at: i64) -> String {
    let at = local(at);
    format!("{:02}:{:02}", at.hour, at.minute)
}

/// "today at 14:00", "tomorrow at 02:00", "yesterday at 03:40", else "Wed 12 Nov at 02:00".
pub fn when_words(at: i64, now: i64) -> String {
    let (then, today) = (local(at), local(now));
    let date = match day_number(&then) - day_number(&today) {
        0 => "today".to_string(),
        1 => "tomorrow".to_string(),
        -1 => "yesterday".to_string(),
        _ => {
            let (weekday, month) = (WEEKDAYS[then.weekday as usize], MONTHS[(then.month as usize - 1).min(11)]);
            format!("{weekday} {} {month}", then.day)
        }
    };
    format!("{date} at {:02}:{:02}", then.hour, then.minute)
}

/// Since which day something has been as it is: "since today", "since yesterday", else "since Mon 2 Nov" (a
/// workspace that was put away, in the menu of the hidden ones).
pub fn since_words(at: i64, now: i64) -> String {
    let (then, today) = (local(at), local(now));
    match day_number(&today) - day_number(&then) {
        0 => "since today".to_string(),
        1 => "since yesterday".to_string(),
        _ => {
            let (weekday, month) = (WEEKDAYS[then.weekday as usize], MONTHS[(then.month as usize - 1).min(11)]);
            format!("since {weekday} {} {month}", then.day)
        }
    }
}

/// A day and its clock time, as a run is dated: "Today, 02:00", "Yesterday, 02:00", else "Mon 9 Nov, 02:00".
pub fn day_words(at: i64, now: i64) -> String {
    let (then, today) = (local(at), local(now));
    let date = match day_number(&today) - day_number(&then) {
        0 => "Today".to_string(),
        1 => "Yesterday".to_string(),
        _ => {
            let (weekday, month) = (WEEKDAYS[then.weekday as usize], MONTHS[(then.month as usize - 1).min(11)]);
            format!("{weekday} {} {month}", then.day)
        }
    };
    format!("{date}, {:02}:{:02}", then.hour, then.minute)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// The now of these tests: Wed 11 Nov 2026, 15:00 local time.
    pub(crate) fn now() -> i64 {
        moment(2026, 11, 11, 15, 0)
    }

    /// A time of day on a day of November 2026, local time.
    pub(crate) fn at(day: u32, hour: u32, minute: u32) -> i64 {
        moment(2026, 11, day, hour, minute)
    }

    /// Reads a clock time as the next time the clock shows it.
    #[test]
    fn says_a_moment_as_the_clock_and_the_calendar_show_it() {
        assert_eq!(when_words(at(12, 2, 0), now()), "tomorrow at 02:00");
        assert_eq!(when_words(at(11, 16, 30), now()), "today at 16:30");
        assert_eq!(when_words(at(14, 2, 0), now()), "Sat 14 Nov at 02:00");
        assert_eq!(when_words(at(10, 3, 40), now()), "yesterday at 03:40");
        assert_eq!(when_words(at(1, 9, 5), now()), "Sun 1 Nov at 09:05");
        assert_eq!(clock_of(at(12, 2, 0)), "02:00");
        // since which day: a workspace that was put away
        assert_eq!(since_words(at(11, 9, 0), now()), "since today");
        assert_eq!(since_words(at(10, 23, 59), now()), "since yesterday");
        assert_eq!(since_words(at(2, 8, 0), now()), "since Mon 2 Nov");
        // a moment and its local parts are each other's
        let then = local(at(14, 23, 59));
        assert_eq!((then.year, then.month, then.day, then.hour, then.minute), (2026, 11, 14, 23, 59));
        assert_eq!(then.weekday, 6, "a Saturday");
        // across a year's end
        assert_eq!(when_words(moment(2027, 1, 1, 0, 30), moment(2026, 12, 31, 23, 0)), "tomorrow at 00:30");
        // a run's day
        assert_eq!(day_words(at(11, 2, 0), now()), "Today, 02:00");
        assert_eq!(day_words(at(10, 23, 59), now()), "Yesterday, 23:59");
        assert_eq!(day_words(at(9, 2, 0), now()), "Mon 9 Nov, 02:00");
        assert_eq!(day_words(at(12, 2, 0), now()), "Thu 12 Nov, 02:00", "a day ahead is named too");
    }
}
