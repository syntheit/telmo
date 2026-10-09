//! "2m" in the list, "2 min ago" in the details.

const MINUTE: u64 = 60;
const HOUR: u64 = 3600;
const DAY: u64 = 86400;
const TWO_DAYS: u64 = 2 * DAY;

fn age(now: u64, then: u64) -> u64 {
    now.saturating_sub(then)
}

pub fn short(now: u64, then: u64) -> String {
    let secs = age(now, then);
    match secs {
        0..MINUTE => "now".into(),
        MINUTE..HOUR => format!("{}m", secs / MINUTE),
        HOUR..DAY => format!("{}h", secs / HOUR),
        DAY..TWO_DAYS => "yesterday".into(),
        _ => format!("{}d", secs / DAY),
    }
}

pub fn long(now: u64, then: u64) -> String {
    let secs = age(now, then);
    let count = |n: u64, unit: &str| format!("{n} {unit}{} ago", if n == 1 { "" } else { "s" });
    match secs {
        0..MINUTE => "just now".into(),
        MINUTE..HOUR => format!("{} min ago", secs / MINUTE),
        HOUR..DAY => count(secs / HOUR, "hour"),
        DAY..TWO_DAYS => "yesterday".into(),
        _ => count(secs / DAY, "day"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_forms() {
        let now = 10_000_000;
        let cases = [
            (5, "now"),
            (120, "2m"),
            (59 * 60, "59m"),
            (3600, "1h"),
            (5 * 3600 + 5, "5h"),
            (24 * 3600, "yesterday"),
            (47 * 3600, "yesterday"),
            (2 * DAY + 5, "2d"),
            (30 * DAY, "30d"),
        ];
        for (secs, want) in cases {
            assert_eq!(short(now, now - secs), want, "{secs}");
        }
    }

    #[test]
    fn long_forms() {
        let now = 1_000_000;
        assert_eq!(long(now, now - 2), "just now");
        assert_eq!(long(now, now - 120), "2 min ago");
        assert_eq!(long(now, now - 3600), "1 hour ago");
        assert_eq!(long(now, now - 3 * 3600), "3 hours ago");
        assert_eq!(long(now, now - 30 * 3600), "yesterday");
        assert_eq!(long(now, now - 3 * DAY), "3 days ago");
        assert_eq!(long(now, now + 5), "just now");
    }
}
