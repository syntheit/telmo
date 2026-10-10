//! What the user types: a timer ("5m tea") or an alarm ("7:30 weekdays wake up").

use std::fmt;

const MAX_MS: i64 = 100 * 3_600_000 - 1000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    Empty,
    Invalid(String),
    Zero,
    TooLong,
    BadTime(String),
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "Type a length, like 5m or 25:00."),
            Self::Invalid(word) => write!(f, "Can't read '{word}' as a length."),
            Self::Zero => write!(f, "A timer needs more than zero time."),
            Self::TooLong => write!(f, "That is longer than 99 hours."),
            Self::BadTime(word) => write!(f, "Can't read '{word}' as a time."),
        }
    }
}

impl std::error::Error for ParseError {}

/// `5m`, `1h30`, `1h30m`, `90s`, `25:00` (minutes:seconds), `1:02:03`, or a
/// bare `10` (minutes). Returns milliseconds.
pub fn duration(word: &str) -> Result<i64, ParseError> {
    let invalid = || ParseError::Invalid(word.to_string());
    let word = word.trim().to_lowercase();
    if word.is_empty() {
        return Err(ParseError::Empty);
    }
    let secs = if word.contains(':') {
        let parts: Vec<u64> = word
            .split(':')
            .map(|p| p.parse::<u64>().map_err(|_| invalid()))
            .collect::<Result<_, _>>()?;
        // Checked: a huge number must be "too long", not a panic or a wrap.
        let total = match parts[..] {
            [m, s] if s < 60 => m.checked_mul(60).and_then(|m| m.checked_add(s)),
            [h, m, s] if m < 60 && s < 60 => {
                h.checked_mul(3600).and_then(|h| h.checked_add(m * 60 + s))
            }
            _ => return Err(invalid()),
        };
        total.ok_or(ParseError::TooLong)?
    } else {
        units(&word).ok_or_else(invalid)?
    };
    let ms = i64::try_from(secs)
        .ok()
        .and_then(|s| s.checked_mul(1000))
        .ok_or(ParseError::TooLong)?;
    match ms {
        0 => Err(ParseError::Zero),
        ms if ms > MAX_MS => Err(ParseError::TooLong),
        ms => Ok(ms),
    }
}

/// Numbers with unit letters, largest first. A last number with no unit
/// counts in the next smaller unit (`1h30` is 1h30m, `5m30` is 5m30s), and a
/// lone number is minutes.
fn units(word: &str) -> Option<u64> {
    let mut total: u64 = 0;
    // Seconds per unit still allowed: hours, minutes, seconds.
    let mut allowed = [3600u64, 60, 1].as_slice();
    let mut rest = word;
    let mut first = true;
    while !rest.is_empty() {
        let digits = rest
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(rest.len());
        if digits == 0 {
            return None;
        }
        let n: u64 = rest[..digits].parse().ok()?;
        rest = &rest[digits..];
        let letters = rest
            .find(|c: char| c.is_ascii_digit())
            .unwrap_or(rest.len());
        let unit = match &rest[..letters] {
            "h" | "hr" | "hrs" | "hour" | "hours" => Some(3600),
            "m" | "min" | "mins" | "minute" | "minutes" => Some(60),
            "s" | "sec" | "secs" | "second" | "seconds" => Some(1),
            "" => None,
            _ => return None,
        };
        rest = &rest[letters..];
        let seconds = match unit {
            Some(seconds) => seconds,
            // A bare number: minutes on its own, otherwise the next unit down.
            None if first => 60,
            None => *allowed.first()?,
        };
        let position = allowed.iter().position(|&a| a == seconds)?;
        allowed = &allowed[position + 1..];
        total = total.checked_add(n.checked_mul(seconds)?)?;
        if unit.is_none() && !rest.is_empty() {
            return None;
        }
        first = false;
    }
    Some(total)
}

/// A timer request: the length first, then any words are its name.
pub fn timer(input: &str) -> Result<(i64, String), ParseError> {
    let input = input.trim();
    let (first, name) = input.split_once(char::is_whitespace).unwrap_or((input, ""));
    Ok((duration(first)?, name.trim().to_string()))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlarmSpec {
    pub hour: u8,
    pub minute: u8,
    /// 1 = Monday … 7 = Sunday; empty means once.
    pub days: Vec<u8>,
    pub name: String,
}

/// `7:30`, `19:00`, `7`, `6pm`, `6:15am`.
pub fn time_of_day(word: &str) -> Result<(u8, u8), ParseError> {
    let bad = || ParseError::BadTime(word.to_string());
    let lower = word.trim().to_lowercase();
    let (body, meridiem) = match lower
        .strip_suffix("am")
        .map(|b| (b, 'a'))
        .or_else(|| lower.strip_suffix("pm").map(|b| (b, 'p')))
    {
        Some((body, m)) => (body, Some(m)),
        None => (lower.as_str(), None),
    };
    let (h, m) = match body.split_once(':') {
        Some((h, m)) => (h, m),
        None => (body, "0"),
    };
    let hour: u8 = h.parse().map_err(|_| bad())?;
    let minute: u8 = m.parse().map_err(|_| bad())?;
    if minute > 59 || (m.len() > 2) {
        return Err(bad());
    }
    let hour = match meridiem {
        None if hour < 24 => hour,
        Some(_) if (1..=12).contains(&hour) => {
            hour % 12 + if meridiem == Some('p') { 12 } else { 0 }
        }
        _ => return Err(bad()),
    };
    Ok((hour, minute))
}

const WEEKDAYS: [&str; 7] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];

/// One day word (`mon`, `friday`), or a range (`mon-fri`), as day numbers.
fn day_word(word: &str) -> Option<Vec<u8>> {
    let one = |w: &str| -> Option<u8> {
        if w.len() < 3 {
            return None;
        }
        let position = WEEKDAYS.iter().position(|d| w.starts_with(d))?;
        let full = [
            "monday",
            "tuesday",
            "wednesday",
            "thursday",
            "friday",
            "saturday",
            "sunday",
        ][position];
        (full.starts_with(w) || w == &full[..3]).then_some(position as u8 + 1)
    };
    if word.contains(',') {
        let parts: Vec<Vec<u8>> = word
            .split(',')
            .filter(|p| !p.is_empty())
            .map(day_word)
            .collect::<Option<_>>()?;
        return Some(parts.concat());
    }
    if let Some((a, b)) = word.split_once('-') {
        let (a, b) = (one(a)?, one(b)?);
        // A range may wrap past Sunday (sat-mon).
        let mut days = vec![a];
        let mut d = a;
        while d != b {
            d = d % 7 + 1;
            days.push(d);
        }
        return Some(days);
    }
    one(word).map(|d| vec![d])
}

/// A time, then optional days (`daily`, `weekdays`, `weekends`, `mon wed fri`,
/// `mon-fri`), then a name.
pub fn alarm(input: &str) -> Result<AlarmSpec, ParseError> {
    let mut words = input.split_whitespace().peekable();
    let first = words.next().ok_or(ParseError::Empty)?;
    let (hour, minute) = time_of_day(first)?;
    let mut days: Vec<u8> = Vec::new();
    while let Some(word) = words.peek() {
        let lower = word.to_lowercase();
        let found = match lower.as_str() {
            "daily" | "everyday" | "every" => Some((1..=7).collect()),
            "weekdays" => Some((1..=5).collect()),
            "weekends" => Some(vec![6, 7]),
            "once" => Some(Vec::new()),
            other => day_word(other),
        };
        let Some(found) = found else { break };
        days.extend(found);
        words.next();
    }
    days.sort_unstable();
    days.dedup();
    let name = words.collect::<Vec<_>>().join(" ");
    Ok(AlarmSpec {
        hour,
        minute,
        days,
        name,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: i64 = 60_000;

    #[test]
    fn durations() {
        for (text, ms) in [
            ("5m", 5 * MIN),
            ("5", 5 * MIN),
            ("1h30", 90 * MIN),
            ("1h30m", 90 * MIN),
            ("1H", 60 * MIN),
            ("90s", 90_000),
            ("2m30", 150_000),
            ("25:00", 25 * MIN),
            ("1:02:03", 3_723_000),
            ("0:45", 45_000),
            ("10min", 10 * MIN),
            ("2hours", 120 * MIN),
        ] {
            assert_eq!(duration(text), Ok(ms), "{text}");
        }
    }

    #[test]
    fn bad_durations() {
        assert_eq!(duration(""), Err(ParseError::Empty));
        assert_eq!(duration("0"), Err(ParseError::Zero));
        assert_eq!(duration("0:00"), Err(ParseError::Zero));
        assert_eq!(duration("100h"), Err(ParseError::TooLong));
        // Would overflow (and panic or wrap to 44 seconds) without checked math.
        assert_eq!(duration("307445734561825861:00"), Err(ParseError::TooLong));
        assert_eq!(duration("5124095576030431:00:00"), Err(ParseError::TooLong));
        for text in [
            "abc", "5x", "m5", "1h30s5", "30s1m", "1:2:3:4", "1:75", "1:61:00", "-5m", "5m3m",
            "1.5h",
        ] {
            assert!(
                matches!(duration(text), Err(ParseError::Invalid(_))),
                "{text}"
            );
        }
    }

    #[test]
    fn timers_with_names() {
        assert_eq!(timer("5m"), Ok((5 * MIN, String::new())));
        assert_eq!(
            timer("  25m  pasta water "),
            Ok((25 * MIN, "pasta water".into()))
        );
        assert_eq!(timer("tea 5m"), Err(ParseError::Invalid("tea".into())));
        assert_eq!(timer("   "), Err(ParseError::Empty));
    }

    #[test]
    fn times_of_day() {
        assert_eq!(time_of_day("7:30"), Ok((7, 30)));
        assert_eq!(time_of_day("07:05"), Ok((7, 5)));
        assert_eq!(time_of_day("19:00"), Ok((19, 0)));
        assert_eq!(time_of_day("7"), Ok((7, 0)));
        assert_eq!(time_of_day("6pm"), Ok((18, 0)));
        assert_eq!(time_of_day("6:15AM"), Ok((6, 15)));
        assert_eq!(time_of_day("12am"), Ok((0, 0)));
        assert_eq!(time_of_day("12pm"), Ok((12, 0)));
        for text in ["24:00", "7:60", "13pm", "0am", "x", "7:3x", "7:300"] {
            assert!(
                matches!(time_of_day(text), Err(ParseError::BadTime(_))),
                "{text}"
            );
        }
    }

    #[test]
    fn alarms() {
        let spec = alarm("7:30 weekdays wake up").expect("alarm");
        assert_eq!((spec.hour, spec.minute), (7, 30));
        assert_eq!(spec.days, vec![1, 2, 3, 4, 5]);
        assert_eq!(spec.name, "wake up");
        assert_eq!(
            alarm("6pm mon wed fri gym").expect("gym").days,
            vec![1, 3, 5]
        );
        assert_eq!(alarm("6pm mon,wed gym").expect("commas").days, vec![1, 3]);
        assert_eq!(alarm("8:00 sat-mon").expect("range").days, vec![1, 6, 7]);
        assert_eq!(
            alarm("21:00 daily pills").expect("daily").days,
            (1..=7).collect::<Vec<_>>()
        );
        assert_eq!(alarm("21:00 weekends").expect("weekends").days, vec![6, 7]);
        let once = alarm("4:15 flight to madrid").expect("once");
        assert!(once.days.is_empty());
        assert_eq!(once.name, "flight to madrid");
        // A name that starts like a weekday but is not one.
        assert_eq!(alarm("9:00 monitor").expect("name").name, "monitor");
        assert_eq!(alarm(""), Err(ParseError::Empty));
        assert!(matches!(alarm("soon"), Err(ParseError::BadTime(_))));
    }
}
