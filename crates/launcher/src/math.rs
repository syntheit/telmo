//! Calculations and unit conversions, by `fend-core`.

/// A result: what the row shows, what ↵ copies, and related values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Math {
    /// `60.96 cm`, thousands grouped.
    pub display: String,
    /// `60.96`: the bare number, which ↵ copies.
    pub plain: String,
    /// `60.96 cm`: tab copies this, unit and all.
    pub with_unit: String,
    /// The dim line under the result.
    pub related: Vec<String>,
}

/// Keeps fend's context between queries, so only the first one pays for setup.
pub struct Calc {
    context: fend_core::Context,
}

impl Default for Calc {
    fn default() -> Self {
        Self::new()
    }
}

/// Units that convert into each other; the related line offers the others.
const FAMILIES: [&[&str]; 6] = [
    &["mm", "cm", "m", "km", "inch", "ft", "yd", "mi"],
    &["g", "kg", "oz", "lb"],
    &["ml", "l", "floz", "gallon"],
    &["B", "KB", "MB", "GB", "TB"],
    &["s", "min", "h", "day", "week"],
    &["m/s", "km/h", "mph", "knot"],
];

impl Calc {
    pub fn new() -> Self {
        Self {
            context: fend_core::Context::new(),
        }
    }

    fn run(&mut self, expr: &str) -> Option<String> {
        let result = fend_core::evaluate(expr, &mut self.context).ok()?;
        let text = result.get_main_result().trim().to_string();
        (!text.is_empty()).then_some(text)
    }

    /// The result of `expr`, or `None` when it isn't a calculation. A bare
    /// number is no calculation unless `forced` (typed after `=`).
    pub fn eval(&mut self, expr: &str, forced: bool) -> Option<Math> {
        let expr = expr.trim();
        if expr.is_empty() || (!forced && is_plain_number(expr)) {
            return None;
        }
        let text = self.run(expr)?;
        if !forced && text == expr {
            return None;
        }
        let (approx, rest) = match text.strip_prefix("approx. ") {
            Some(rest) => (true, rest),
            None => (false, text.as_str()),
        };
        let (number, unit) = split_number(rest);
        let plain = number.to_string();
        let shown = format!("{}{}", group(number), unit_part(unit));
        let with_unit = format!("{number}{}", unit_part(unit));
        let related = self.related(expr, number, unit);
        Some(Math {
            display: if approx {
                format!("≈ {shown}")
            } else {
                shown
            },
            plain,
            with_unit,
            related,
        })
    }

    fn related(&mut self, expr: &str, number: &str, unit: &str) -> Vec<String> {
        if let Some(percent) = percent_of(expr) {
            return percent;
        }
        if !unit.is_empty() {
            return self.other_units(number, unit);
        }
        integer_forms(number)
    }

    /// The same quantity in the other units of its family.
    fn other_units(&mut self, number: &str, unit: &str) -> Vec<String> {
        let Some(family) = FAMILIES
            .iter()
            .find(|f| f.iter().any(|u| same_unit(u, unit)))
        else {
            return Vec::new();
        };
        family
            .iter()
            .filter(|u| !same_unit(u, unit))
            .filter_map(|other| {
                let converted = self.run(&format!("{number} {unit} to {other}"))?;
                let converted = converted.strip_prefix("approx. ").unwrap_or(&converted);
                let (n, u) = split_number(converted);
                (!n.is_empty()).then(|| format!("{}{}", group(&trim_digits(n)), unit_part(u)))
            })
            .take(3)
            .collect()
    }
}

fn unit_part(unit: &str) -> String {
    if unit.is_empty() {
        String::new()
    } else {
        format!(" {unit}")
    }
}

/// fend prints some units in full (`feet`); the family table uses symbols.
fn same_unit(symbol: &str, printed: &str) -> bool {
    let printed = printed.trim();
    if symbol.eq_ignore_ascii_case(printed) {
        return true;
    }
    let long: &[&str] = match symbol {
        "inch" => &["inches", "in"],
        "ft" => &["foot", "feet"],
        "yd" => &["yard", "yards"],
        "mi" => &["mile", "miles"],
        "oz" => &["ounce", "ounces"],
        "lb" => &["pound", "pounds"],
        "h" => &["hour", "hours"],
        "min" => &["minute", "minutes"],
        "s" => &["second", "seconds"],
        "day" => &["days"],
        "week" => &["weeks"],
        "gallon" => &["gallons"],
        "knot" => &["knots"],
        _ => &[],
    };
    long.contains(&printed)
}

/// Rounds a long decimal to six places for the related line.
fn trim_digits(number: &str) -> String {
    match number.parse::<f64>() {
        Ok(v) if number.contains('.') && !number.contains('e') => {
            let text = format!("{v:.6}");
            text.trim_end_matches('0').trim_end_matches('.').to_string()
        }
        _ => number.to_string(),
    }
}

fn is_plain_number(expr: &str) -> bool {
    let cleaned: String = expr
        .chars()
        .filter(|c| !matches!(c, ',' | '_' | ' '))
        .collect();
    cleaned.parse::<f64>().is_ok()
}

/// Splits `-12.5e3 km` into the number and what follows.
pub fn split_number(text: &str) -> (&str, &str) {
    let bytes = text.as_bytes();
    let mut end = 0;
    if bytes.first() == Some(&b'-') {
        end = 1;
    }
    let digits = |from: usize| {
        bytes[from..]
            .iter()
            .take_while(|b| b.is_ascii_digit())
            .count()
    };
    let whole = digits(end);
    if whole == 0 {
        return ("", text);
    }
    end += whole;
    if bytes.get(end) == Some(&b'.') && digits(end + 1) > 0 {
        end += 1 + digits(end + 1);
    }
    if matches!(bytes.get(end), Some(b'e' | b'E')) {
        let sign = usize::from(matches!(bytes.get(end + 1), Some(b'-' | b'+')));
        let exp = digits(end + 1 + sign);
        if exp > 0 {
            end += 1 + sign + exp;
        }
    }
    (&text[..end], text[end..].trim())
}

/// Groups the integer digits by thousands: `1234567.5` -> `1,234,567.5`.
pub fn group(number: &str) -> String {
    if number.contains(['e', 'E']) {
        return number.to_string();
    }
    let (sign, body) = number.strip_prefix('-').map_or(("", number), |b| ("-", b));
    let (whole, fraction) = body
        .split_once('.')
        .map_or((body, None), |(w, f)| (w, Some(f)));
    let mut out = String::from(sign);
    for (i, digit) in whole.chars().enumerate() {
        if i > 0 && (whole.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(digit);
    }
    if let Some(fraction) = fraction {
        out.push('.');
        out.push_str(fraction);
    }
    out
}

/// `20% of 150`: what is left after the discount and what it is with the raise.
fn percent_of(expr: &str) -> Option<Vec<String>> {
    let lower = expr.to_ascii_lowercase();
    let (percent, of) = lower.split_once("% of ")?;
    let percent: f64 = percent.trim().parse().ok()?;
    let of: f64 = of.trim().replace(',', "").parse().ok()?;
    let part = percent / 100.0 * of;
    let money = |v: f64| group(&trim_digits(&format!("{v:.6}")));
    Some(vec![
        format!("{} after a {percent}% discount", money(of - part)),
        format!("{} plus {percent}%", money(of + part)),
    ])
}

/// Hex and binary forms of a whole number.
fn integer_forms(number: &str) -> Vec<String> {
    let Ok(value) = number.parse::<i64>() else {
        return Vec::new();
    };
    if !(0..=u32::MAX as i64).contains(&value) {
        return Vec::new();
    }
    let mut forms = vec![format!("0x{value:x}")];
    if value < 1 << 16 {
        forms.push(format!("0b{value:b}"));
    }
    forms
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eval(expr: &str) -> Option<Math> {
        Calc::new().eval(expr, false)
    }

    #[test]
    fn a_conversion_gives_value_unit_and_neighbours() {
        let m = eval("2 ft to cm").expect("result");
        assert_eq!(m.display, "60.96 cm");
        assert_eq!(m.plain, "60.96");
        assert_eq!(m.with_unit, "60.96 cm");
        assert_eq!(m.related.len(), 3);
        assert!(
            m.related.iter().all(|r| !r.contains("cm")),
            "{:?}",
            m.related
        );
    }

    #[test]
    fn arithmetic_groups_thousands_but_copies_the_bare_number() {
        let m = eval("1200 * 3").expect("result");
        assert_eq!(m.display, "3,600");
        assert_eq!(m.plain, "3600");
        assert_eq!(m.related, ["0xe10", "0b111000010000"]);
    }

    #[test]
    fn inexact_results_say_so() {
        let m = eval("1/3").expect("result");
        assert!(m.display.starts_with("≈ 0.3333"), "{}", m.display);
        assert!(m.plain.starts_with("0.3333"));
    }

    #[test]
    fn percentages_offer_the_discount() {
        let m = eval("20% of 150").expect("result");
        assert_eq!(m.plain, "30");
        assert_eq!(m.related, ["120 after a 20% discount", "180 plus 20%"]);
    }

    #[test]
    fn a_bare_number_is_not_a_calculation_unless_forced() {
        assert!(eval("42").is_none());
        assert!(eval("1,000").is_none());
        let forced = Calc::new().eval("42", true).expect("forced");
        assert_eq!(forced.display, "42");
    }

    #[test]
    fn half_typed_input_and_app_names_give_nothing() {
        assert!(eval("2 ft to").is_none());
        assert!(eval("1password").is_none());
        assert!(eval("2 +").is_none());
        assert!(eval("").is_none());
    }

    #[test]
    fn numbers_split_from_their_units() {
        assert_eq!(split_number("-12.5e3 km"), ("-12.5e3", "km"));
        assert_eq!(split_number("7"), ("7", ""));
        assert_eq!(split_number("3.x"), ("3", ".x"));
        assert_eq!(split_number("kg"), ("", "kg"));
        assert_eq!(group("-1234567.25"), "-1,234,567.25");
        assert_eq!(group("999"), "999");
        assert_eq!(group("1e21"), "1e21");
    }

    #[test]
    fn every_family_unit_converts_to_the_others() {
        // The related line silently drops what fend can't convert; make sure
        // the table only holds names fend knows.
        let mut calc = Calc::new();
        for family in FAMILIES {
            for from in family {
                for to in family.iter().filter(|u| *u != from) {
                    assert!(
                        calc.run(&format!("1 {from} to {to}")).is_some(),
                        "1 {from} to {to}"
                    );
                }
            }
        }
    }
}
