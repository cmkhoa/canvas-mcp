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

/// Line width for html2text; wide enough that ordinary paragraphs don't wrap.
const TEXT_WIDTH: usize = 400;

/// Canvas HTML (pages, descriptions, posts) as markdown-ish text, capped at `max_chars`.
/// Links become numbered footnotes; scripts and styles are dropped.
pub fn html_to_text(html: &str, max_chars: usize) -> String {
    let text = html2text::config::plain()
        .string_from_read(html.as_bytes(), TEXT_WIDTH)
        .unwrap_or_else(|_| html.to_owned());
    let mut out = String::with_capacity(text.len());
    let mut blank_run = 0;
    for line in text.lines().map(str::trim_end) {
        blank_run = if line.is_empty() { blank_run + 1 } else { 0 };
        if blank_run < 2 {
            out.push_str(line);
            out.push('\n');
        }
    }
    truncate(out.trim(), max_chars)
}

/// Cuts `text` to at most `max_chars` characters, saying how much was left out.
pub fn truncate(text: &str, max_chars: usize) -> String {
    match text.char_indices().nth(max_chars) {
        None => text.to_owned(),
        Some((cut, _)) => {
            let total = text.chars().count();
            format!(
                "{}\n…[truncated: showing {max_chars} of {total} characters]",
                text[..cut].trim_end()
            )
        }
    }
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
    fn html_becomes_readable_text() {
        let html = r#"<h2>Week 3 &amp; Lab</h2><p>Read <a href="https://x/files/9">chapter 3</a> by <b>Friday</b>.</p>



<ul><li>One</li></ul><script>alert(1)</script><p>Bye</p>"#;
        let text = html_to_text(html, 1000);
        assert!(text.starts_with("## Week 3 & Lab"), "{text}");
        assert!(text.contains("[chapter 3][1]") && text.contains("[1]: https://x/files/9"));
        assert!(text.contains("* One"));
        assert!(!text.contains("alert") && !text.contains("<"));
        assert!(!text.contains("\n\n\n"));
    }

    #[test]
    fn truncate_counts_characters_not_bytes() {
        assert_eq!(truncate("héllo", 10), "héllo");
        assert_eq!(
            truncate("héllo wörld", 5),
            "héllo\n…[truncated: showing 5 of 11 characters]"
        );
    }

    #[test]
    fn shows_year_and_daylight_time_when_relevant() {
        let out = format_date(utc("2027-06-01T17:00:00Z"), Chicago, utc(NOW));
        assert_eq!(out, "Tue Jun 1 2027, 12:00 PM CDT (in 203 days)");
    }
}
