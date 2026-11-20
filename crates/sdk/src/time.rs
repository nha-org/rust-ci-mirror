//! Dates as the plugins write them, from a day number or a stamp the host's clock gave: arithmetic only, no clock read.

/// The months' short names, January first: a date's month as mail, tasks and the Gantt write it.
pub const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// Days since 1970-01-01 of a date written 2026-11-15 (its first ten characters), the civil calendar's arithmetic. A day
/// that does not exist (2026-02-31, 2026-13-01) or a field that is not four or two digits is none: only a date that reads
/// back the same is one.
pub fn days(date: &str) -> Option<i64> {
    let date = date.get(..10)?;
    let mut p = date.split('-').map(|n| n.parse::<i64>().ok());
    let (y, m, d) = (p.next()??, p.next()??, p.next()??);
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * ((m + 9) % 12) + 2) / 5 + d - 1;
    let n = era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468;
    (self::date(n) == date).then_some(n)
}

/// A day written whole, « 2026-11-15 » and nothing after it, that exists.
pub fn is_day(s: &str) -> bool {
    s.len() == 10 && days(s).is_some()
}

/// The date of a day number since 1970-01-01, written 2026-11-15.
pub fn date(days: i64) -> String {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

/// A time in Unix seconds in UTC, as GitHub writes them: « 2026-11-11T16:44:10Z ».
pub fn iso(secs: i64) -> String {
    let (days, s) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    format!(
        "{}T{:02}:{:02}:{:02}Z",
        date(days),
        s / 3600,
        s / 60 % 60,
        s % 60
    )
}

/// A time in Unix milliseconds as the old app's `Status.stamp` writes it: « 2026-11-09 00:15 UTC ». /watchers' beats and a task's
/// Activity both.
pub fn stamp(ms: i64) -> String {
    let s = ms.div_euclid(1000);
    let (day, min) = (s.div_euclid(86_400), s.rem_euclid(86_400) / 60);
    format!("{} {:02}:{:02} UTC", date(day), min / 60, min % 60)
}

#[cfg(test)]
mod tests {
    #[dash_meter::dash_test]
    fn days_and_dates_agree_and_a_day_that_does_not_exist_is_none() {
        assert_eq!(super::days("1970-01-01"), Some(0));
        assert_eq!(super::date(20_772), "2026-11-15");
        assert_eq!(super::days("2026-11-15T04:59:59Z"), Some(20_772));
        assert_eq!(super::days("2024-02-29"), Some(19_782), "a leap day");
        for bad in [
            "2026-13-01",
            "2026-02-31",
            "2026-02-29",
            "2026x11x15",
            "+026-11-15",
            "2026-1-150",
        ] {
            assert_eq!(super::days(bad), None, "{bad}");
        }
    }
}
