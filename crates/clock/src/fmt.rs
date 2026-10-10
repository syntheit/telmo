//! Durations and moments as the screens write them.

const SECOND: i64 = 1000;
const MINUTE: i64 = 60 * SECOND;
const HOUR: i64 = 60 * MINUTE;
const DAY: i64 = 24 * HOUR;

/// A countdown: `4:32` or `1:04:32`. Rounds up, so it only reads `0:00` at zero.
pub fn countdown(ms: i64) -> String {
    let secs = (ms.max(0) + SECOND - 1) / SECOND;
    let (h, m, s) = (secs / 3600, secs / 60 % 60, secs % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// The stopwatch's big readout: `12:34.5` or `1:02:03.4`.
pub fn tenths(ms: i64) -> String {
    let ms = ms.max(0);
    let (h, m, s, t) = (ms / HOUR, ms / MINUTE % 60, ms / SECOND % 60, ms / 100 % 10);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}.{t}")
    } else {
        format!("{}:{s:02}.{t}", ms / MINUTE)
    }
}

/// A lap time: `3:07.26` or `1:02:03.45`.
pub fn hundredths(ms: i64) -> String {
    let ms = ms.max(0);
    let (h, m, s, c) = (ms / HOUR, ms / MINUTE % 60, ms / SECOND % 60, ms / 10 % 100);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}.{c:02}")
    } else {
        format!("{}:{s:02}.{c:02}", ms / MINUTE)
    }
}

/// How a lap compares with the one before: `−0:46.25` or `+0:03.10`.
pub fn delta(ms: i64) -> String {
    let sign = if ms < 0 { '−' } else { '+' };
    format!("{sign}{}", hundredths(ms.abs()))
}

/// A length for labels: `5m`, `1h30m`, `1m30s`, `45s`.
pub fn short(ms: i64) -> String {
    let secs = (ms.max(0) + 500) / SECOND;
    let (h, m, s) = (secs / 3600, secs / 60 % 60, secs % 60);
    let mut out = String::new();
    if h > 0 {
        out += &format!("{h}h");
    }
    if m > 0 {
        out += &format!("{m}m");
    }
    if s > 0 || out.is_empty() {
        out += &format!("{s}s");
    }
    out
}

/// A length in words, for the new-timer preview: `25 minutes`, `1 hour 30 minutes`.
pub fn words(ms: i64) -> String {
    let secs = ms.max(0) / SECOND;
    let (h, m, s) = (secs / 3600, secs / 60 % 60, secs % 60);
    let unit = |n: i64, one: &str| format!("{n} {one}{}", if n == 1 { "" } else { "s" });
    let mut parts = Vec::new();
    if h > 0 {
        parts.push(unit(h, "hour"));
    }
    if m > 0 {
        parts.push(unit(m, "minute"));
    }
    if s > 0 {
        parts.push(unit(s, "second"));
    }
    parts.join(" ")
}

/// Time until something: `in 45s`, `in 12m`, `in 6h 53m`, `in 1d 17h`.
pub fn until(ms: i64) -> String {
    let ms = ms.max(0);
    if ms < MINUTE {
        format!("in {}s", ms / SECOND)
    } else if ms < HOUR {
        format!("in {}m", ms / MINUTE)
    } else if ms < DAY {
        format!("in {}h {}m", ms / HOUR, ms / MINUTE % 60)
    } else {
        format!("in {}d {}h", ms / DAY, ms / HOUR % 24)
    }
}

/// `UTC−3`, `UTC+5:30`, `UTC`.
pub fn utc_offset(seconds: i32) -> String {
    if seconds == 0 {
        return "UTC".into();
    }
    let sign = if seconds < 0 { '−' } else { '+' };
    let (h, m) = (seconds.abs() / 3600, seconds.abs() / 60 % 60);
    if m == 0 {
        format!("UTC{sign}{h}")
    } else {
        format!("UTC{sign}{h}:{m:02}")
    }
}

/// The gap between two zones: `same`, `+4h`, `−1h`, `+5h 30m`.
pub fn offset_difference(seconds: i32) -> String {
    if seconds == 0 {
        return "same".into();
    }
    let sign = if seconds < 0 { '−' } else { '+' };
    let (h, m) = (seconds.abs() / 3600, seconds.abs() / 60 % 60);
    match (h, m) {
        (h, 0) => format!("{sign}{h}h"),
        (0, m) => format!("{sign}{m}m"),
        (h, m) => format!("{sign}{h}h {m}m"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn countdowns_round_up() {
        assert_eq!(countdown(272_000), "4:32");
        assert_eq!(countdown(271_001), "4:32");
        assert_eq!(countdown(0), "0:00");
        assert_eq!(countdown(1), "0:01");
        assert_eq!(countdown(3_872_000), "1:04:32");
        assert_eq!(countdown(-5), "0:00");
    }

    #[test]
    fn stopwatch_readouts() {
        assert_eq!(tenths(754_560), "12:34.5");
        assert_eq!(tenths(3_723_400), "1:02:03.4");
        assert_eq!(hundredths(187_260), "3:07.26");
        assert_eq!(hundredths(3_723_450), "1:02:03.45");
        assert_eq!(delta(-46_250), "−0:46.25");
        assert_eq!(delta(3_100), "+0:03.10");
    }

    #[test]
    fn labels() {
        assert_eq!(short(300_000), "5m");
        assert_eq!(short(5_400_000), "1h30m");
        assert_eq!(short(90_000), "1m30s");
        assert_eq!(short(45_000), "45s");
        assert_eq!(words(1_500_000), "25 minutes");
        assert_eq!(words(5_400_000), "1 hour 30 minutes");
        assert_eq!(words(61_000), "1 minute 1 second");
    }

    #[test]
    fn distances() {
        assert_eq!(until(45_000), "in 45s");
        assert_eq!(until(12 * MINUTE), "in 12m");
        assert_eq!(until(6 * HOUR + 53 * MINUTE), "in 6h 53m");
        assert_eq!(until(DAY + 17 * HOUR + 23 * MINUTE), "in 1d 17h");
        assert_eq!(utc_offset(-3 * 3600), "UTC−3");
        assert_eq!(utc_offset(19_800), "UTC+5:30");
        assert_eq!(utc_offset(0), "UTC");
        assert_eq!(offset_difference(0), "same");
        assert_eq!(offset_difference(12 * 3600), "+12h");
        assert_eq!(offset_difference(-3600), "−1h");
        assert_eq!(offset_difference(19_800), "+5h 30m");
    }
}
