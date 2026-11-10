//! Turning Canvas data into compact text for the model.
use chrono::{DateTime, Datelike, Utc};
use chrono_tz::Tz;

/// A Canvas timestamp in the user's zone with a relative hint, e.g.
/// `Sat Nov 14, 11:59 PM CST (in 4 days)`. The year appears only when it isn't `now`'s year.
pub fn format_date(at: DateTime<Utc>, tz: Tz, now: DateTime<Utc>) -> String {
    let local = at.with_timezone(&tz);
    let pattern = if local.year() == now.with_timezone(&tz).year() {
        "%a %b %-d, %-I:%M %p %Z"
    } else {
        "%a %b %-d %Y, %-I:%M %p %Z"
    };
    format!("{} ({})", local.format(pattern), relative(at, tz, now))
}

fn relative(at: DateTime<Utc>, tz: Tz, now: DateTime<Utc>) -> String {
    let secs = (at - now).num_seconds();
    let future = secs >= 0;
    let abs = secs.unsigned_abs();
    let phrase = |n: u64, unit: &str| {
        let unit = if n == 1 {
            unit.to_owned()
        } else {
            format!("{unit}s")
        };
        if future {
            format!("in {n} {unit}")
        } else {
            format!("{n} {unit} ago")
        }
    };

    if abs < 60 {
        return "now".into();
    }
    if abs < 3600 {
        return phrase(abs / 60, "minute");
    }
    if abs < 24 * 3600 {
        return phrase(abs / 3600, "hour");
    }
    // Past a day, count calendar days in the user's zone so "tomorrow" means tomorrow there.
    let days =
        (at.with_timezone(&tz).date_naive() - now.with_timezone(&tz).date_naive()).num_days();
    match days {
        1 => "tomorrow".into(),
        -1 => "yesterday".into(),
        d => phrase(d.unsigned_abs(), "day"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono_tz::America::Chicago;

    fn utc(s: &str) -> DateTime<Utc> {
        s.parse().unwrap()
    }

    // Tue Nov 10 2026, 2:00 PM in Chicago (CST, UTC-6).
    const NOW: &str = "2026-11-10T20:00:00Z";

    #[test]
    fn shows_local_time_and_days_ahead() {
        // Sun Nov 15 05:59 UTC is Sat Nov 14 11:59 PM in Chicago.
        let out = format_date(utc("2026-11-15T05:59:00Z"), Chicago, utc(NOW));
        assert_eq!(out, "Sat Nov 14, 11:59 PM CST (in 4 days)");
    }

    #[test]
    fn uses_hours_minutes_and_calendar_words() {
        let now = utc(NOW);
        assert!(format_date(utc("2026-11-10T22:30:00Z"), Chicago, now).ends_with("(in 2 hours)"));
        assert!(
            format_date(utc("2026-11-10T19:15:00Z"), Chicago, now).ends_with("(45 minutes ago)")
        );
        assert!(format_date(utc("2026-11-10T20:00:30Z"), Chicago, now).ends_with("(now)"));
        // 2 PM Wed in Chicago: 24h later and the next calendar day.
        assert!(format_date(utc("2026-11-11T20:00:00Z"), Chicago, now).ends_with("(tomorrow)"));
        assert!(format_date(utc("2026-11-08T20:00:00Z"), Chicago, now).ends_with("(2 days ago)"));
    }

    #[test]
    fn days_follow_the_users_calendar_not_utc() {
        // Thu 4:30 AM UTC is still Wed 10:30 PM in Chicago: tomorrow, not 2 days.
        let out = format_date(utc("2026-11-12T04:30:00Z"), Chicago, utc(NOW));
        assert_eq!(out, "Wed Nov 11, 10:30 PM CST (tomorrow)");
    }

    #[test]
    fn shows_year_and_daylight_time_when_relevant() {
        let out = format_date(utc("2027-06-01T17:00:00Z"), Chicago, utc(NOW));
        assert_eq!(out, "Tue Jun 1 2027, 12:00 PM CDT (in 203 days)");
    }
}
