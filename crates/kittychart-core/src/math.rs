//! Ports of Chart.js `helpers.math`. JavaScript numeric semantics are kept where they change
//! results (notably `Math.round`, which rounds .5 toward +∞).

/// JavaScript `Math.round`: half rounds toward +∞ (`-2.5 → -2`).
pub fn js_round(x: f64) -> f64 {
    let f = x.floor();
    if x - f >= 0.5 { f + 1.0 } else { f }
}

/// JavaScript `Math.sign`, which keeps ±0 and maps NaN to NaN.
pub fn sign(x: f64) -> f64 {
    if x > 0.0 {
        1.0
    } else if x < 0.0 {
        -1.0
    } else {
        x
    }
}

pub fn almost_equals(x: f64, y: f64, epsilon: f64) -> bool {
    (x - y).abs() < epsilon
}

pub fn almost_whole(x: f64, epsilon: f64) -> bool {
    let r = js_round(x);
    r - epsilon <= x && r + epsilon >= x
}

pub fn nice_num(range: f64) -> f64 {
    let rounded = js_round(range);
    let range = if almost_equals(range, rounded, range / 1000.0) { rounded } else { range };
    let nice_range = 10f64.powf(range.log10().floor());
    let fraction = range / nice_range;
    let nice_fraction = if fraction <= 1.0 {
        1.0
    } else if fraction <= 2.0 {
        2.0
    } else if fraction <= 5.0 {
        5.0
    } else {
        10.0
    };
    nice_fraction * nice_range
}

/// `_decimalPlaces`: the number of decimal digits needed to represent `x` exactly.
pub fn decimal_places(x: f64) -> Option<u32> {
    if !x.is_finite() {
        return None;
    }
    let mut e = 1.0;
    let mut p = 0;
    while js_round(x * e) / e != x {
        e *= 10.0;
        p += 1;
        if p > 30 {
            break; // JS would loop on, but no double needs more than this
        }
    }
    Some(p)
}

/// `_factorize`: proper divisors of `value` in ascending order, excluding `value` itself.
pub fn factorize(value: u64) -> Vec<u64> {
    let mut result = Vec::new();
    let sqrt = (value as f64).sqrt();
    let mut i = 1;
    while (i as f64) < sqrt {
        if value.is_multiple_of(i) {
            result.push(i);
            result.push(value / i);
        }
        i += 1;
    }
    if sqrt == sqrt.trunc() {
        result.push(sqrt as u64);
    }
    result.sort_unstable();
    result.pop();
    result
}

pub fn limit_value(v: f64, min: f64, max: f64) -> f64 {
    min.max(max.min(v))
}

pub fn int16_range(v: f64) -> f64 {
    limit_value(v, -32768.0, 32767.0)
}

pub fn is_between(v: f64, start: f64, end: f64) -> bool {
    let eps = 1e-6;
    v >= start.min(end) - eps && v <= start.max(end) + eps
}

pub fn to_radians(deg: f64) -> f64 {
    deg * (std::f64::consts::PI / 180.0)
}

pub fn to_degrees(rad: f64) -> f64 {
    rad * (180.0 / std::f64::consts::PI)
}

/// `_alignPixel`: snap a line coordinate so `width`-wide lines land on device pixels.
pub fn align_pixel(dpr: f64, pixel: f64, width: f64) -> f64 {
    let half = if width != 0.0 { (width / 2.0).max(0.5) } else { 0.0 };
    js_round((pixel - half) * dpr) / dpr + half
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn js_round_semantics() {
        assert_eq!(js_round(2.5), 3.0);
        assert_eq!(js_round(-2.5), -2.0);
        assert_eq!(js_round(-2.6), -3.0);
    }

    #[test]
    fn nice_numbers() {
        assert_eq!(nice_num(0.7), 1.0);
        assert_eq!(nice_num(1.3), 2.0);
        assert_eq!(nice_num(3.0), 5.0);
        assert_eq!(nice_num(7.0), 10.0);
        assert_eq!(nice_num(12.0), 20.0);
        assert!((nice_num(0.0023) - 0.005).abs() < 1e-15);
    }

    #[test]
    fn decimals_and_factors() {
        assert_eq!(decimal_places(0.25), Some(2));
        assert_eq!(decimal_places(10.0), Some(0));
        assert_eq!(decimal_places(0.1), Some(1));
        assert_eq!(factorize(12), vec![1, 2, 3, 4, 6]);
        assert_eq!(factorize(16), vec![1, 2, 4, 8]);
    }
}
