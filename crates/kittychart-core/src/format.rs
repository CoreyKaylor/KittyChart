//! Number formatting compatible with `Intl.NumberFormat('en-US', …)` for the options Chart.js
//! uses: fixed fraction digits, grouping separators, and scientific notation.

/// Format `value` with exactly `digits` fraction digits, `,` grouping, and half-expand rounding.
pub fn format_fixed(value: f64, digits: usize) -> String {
    if value.is_nan() {
        return "NaN".into();
    }
    if value.is_infinite() {
        return if value > 0.0 { "∞".into() } else { "-∞".into() };
    }
    let (int_part, frac_part) = round_half_expand(value.abs(), digits);
    let mut out = String::new();
    // Intl keeps the sign even when the rounded result is zero ("-0.0").
    if value < 0.0 {
        out.push('-');
    }
    out.push_str(&group(&int_part));
    if digits > 0 {
        out.push('.');
        out.push_str(&frac_part);
    }
    out
}

/// `notation: 'scientific'`: mantissa with `digits` fraction digits, then `E` and the exponent.
pub fn format_scientific(value: f64, digits: usize) -> String {
    if value == 0.0 {
        return format!("{}E0", format_fixed(0.0, digits));
    }
    let mut exp = value.abs().log10().floor() as i32;
    let mut mantissa = value / 10f64.powi(exp);
    // Rounding can carry the mantissa to 10.
    let rounded: f64 =
        format_fixed(mantissa.abs(), digits).replace(',', "").parse().unwrap_or(mantissa.abs());
    if rounded >= 10.0 {
        exp += 1;
        mantissa = value / 10f64.powi(exp);
    }
    format!("{}E{}", format_fixed(mantissa, digits), exp)
}

/// Round half away from zero on the *shortest round-trip* decimal form of the double, as V8's
/// Intl does (`1.005` → `"1.01"`, although the binary value is 1.00499…).
fn round_half_expand(abs: f64, digits: usize) -> (String, String) {
    // Rust's `Display` for f64 prints the shortest round-trip digits, never in exponent form.
    let s = format!("{abs}");
    let (int_s, frac_s) = s.split_once('.').unwrap_or((&s, ""));
    // Fraction digits, zero-padded to `digits` ("1" → "1.0" for one digit).
    let frac_digits = frac_s.bytes().chain(std::iter::repeat(b'0')).take(digits);
    let mut digits_vec: Vec<u8> = int_s.bytes().chain(frac_digits).map(|b| b - b'0').collect();
    let next = frac_s.as_bytes().get(digits).map_or(0, |b| b - b'0');
    if next >= 5 {
        // Carry.
        let mut i = digits_vec.len();
        loop {
            if i == 0 {
                digits_vec.insert(0, 1);
                break;
            }
            i -= 1;
            if digits_vec[i] == 9 {
                digits_vec[i] = 0;
            } else {
                digits_vec[i] += 1;
                break;
            }
        }
    }
    let int_len = digits_vec.len() - digits;
    let to_s = |d: &[u8]| d.iter().map(|d| char::from(b'0' + d)).collect::<String>();
    let mut int_part = to_s(&digits_vec[..int_len]);
    let trimmed = int_part.trim_start_matches('0');
    int_part = if trimmed.is_empty() { "0".into() } else { trimmed.into() };
    (int_part, to_s(&digits_vec[int_len..]))
}

fn group(int_part: &str) -> String {
    let bytes = int_part.as_bytes();
    let mut out = String::with_capacity(bytes.len() + bytes.len() / 3);
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 && (bytes.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(char::from(*b));
    }
    out
}

/// Chart.js `Ticks.formatters.numeric` for a tick at `index`, given every tick value.
pub fn numeric_tick(value: f64, index: usize, ticks: &[f64]) -> String {
    let _ = index;
    if value == 0.0 {
        return "0".into();
    }
    let mut scientific = false;
    let mut delta = value;
    if ticks.len() > 1 {
        let max_tick = ticks[0].abs().max(ticks[ticks.len() - 1].abs());
        if max_tick < 1e-4 || max_tick > 1e15 {
            scientific = true;
        }
        delta = if ticks.len() > 3 { ticks[2] - ticks[1] } else { ticks[1] - ticks[0] };
        if delta.abs() >= 1.0 && value != value.floor() {
            delta = value - value.floor();
        }
    }
    let log_delta = delta.abs().log10();
    let num_decimal = if log_delta.is_nan() { 1 } else { (-log_delta.floor()).clamp(0.0, 20.0) as usize };
    if scientific { format_scientific(value, num_decimal) } else { format_fixed(value, num_decimal) }
}

/// `scale.getLabelForValue` for linear scales: `Intl.NumberFormat('en-US')` defaults
/// (up to 3 fraction digits, trailing zeros dropped).
pub fn format_default(value: f64) -> String {
    let s = format_fixed(value, 3);
    if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_matches_intl() {
        assert_eq!(format_fixed(1234567.0, 0), "1,234,567");
        assert_eq!(format_fixed(0.125, 2), "0.13"); // Intl rounds half-expand; Rust's {:.2} gives 0.12
        assert_eq!(format_fixed(-0.5, 1), "-0.5");
        assert_eq!(format_fixed(-0.04, 1), "-0.0");
        assert_eq!(format_fixed(999.96, 1), "1,000.0");
        assert_eq!(format_fixed(1.005, 2), "1.01"); // V8 rounds the shortest decimal form, "1.005"
        assert_eq!(format_fixed(1e-7, 2), "0.00");
        assert_eq!(format_fixed(123456789.987654, 3), "123,456,789.988");
    }

    #[test]
    fn numeric_ticks() {
        let ticks = [0.0, 0.2, 0.4, 0.6, 0.8, 1.0];
        let labels: Vec<_> = ticks.iter().enumerate().map(|(i, v)| numeric_tick(*v, i, &ticks)).collect();
        assert_eq!(labels, ["0", "0.2", "0.4", "0.6", "0.8", "1.0"]);
        let ticks = [0.0, 5000.0, 10000.0];
        assert_eq!(numeric_tick(10000.0, 2, &ticks), "10,000");
        let ticks = [0.0, 5e15, 1e16, 1.5e16, 2e16];
        assert_eq!(numeric_tick(1.5e16, 3, &ticks), "2E16");
        assert_eq!(numeric_tick(5e15, 1, &ticks), "5E15");
    }

    #[test]
    fn default_label() {
        assert_eq!(format_default(1234.5), "1,234.5");
        assert_eq!(format_default(3.0), "3");
        assert_eq!(format_default(0.12345), "0.123");
    }
}
