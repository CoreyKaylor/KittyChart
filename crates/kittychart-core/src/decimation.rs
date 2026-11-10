//! The `decimation` plugin: a port of Chart.js `plugin.decimation.js` (LTTB and min-max).

use crate::chart::Parsed;

/// Largest-Triangle-Three-Buckets down to `samples` points.
pub fn lttb(data: &[Parsed], start: usize, count: usize, samples: usize) -> Vec<Parsed> {
    if samples >= count || samples < 3 {
        return data[start..start + count].to_vec();
    }
    let xy = |p: &Parsed| (p.x.unwrap_or(f64::NAN), p.y.unwrap_or(f64::NAN));
    let mut out = Vec::with_capacity(samples);
    let bucket = (count - 2) as f64 / (samples - 2) as f64;
    let end_index = start + count - 1;
    let mut a = start;
    out.push(data[a]);
    for i in 0..samples - 2 {
        let fi = i as f64;
        let avg_start = ((fi + 1.0) * bucket).floor() as usize + 1 + start;
        let avg_end = ((((fi + 2.0) * bucket).floor() as usize) + 1).min(count) + start;
        let len = (avg_end - avg_start) as f64;
        let (mut ax, mut ay) = (0.0, 0.0);
        for p in &data[avg_start..avg_end] {
            let (x, y) = xy(p);
            ax += x;
            ay += y;
        }
        ax /= len;
        ay /= len;
        let range_offs = (fi * bucket).floor() as usize + 1 + start;
        let range_to = ((((fi + 1.0) * bucket).floor() as usize) + 1).min(count) + start;
        let (pax, pay) = xy(&data[a]);
        let mut max_area = -1.0;
        let mut next_a = a;
        for (j, p) in data.iter().enumerate().take(range_to).skip(range_offs) {
            let (x, y) = xy(p);
            let area = 0.5 * ((pax - ax) * (y - pay) - (pax - x) * (ay - pay)).abs();
            if area > max_area {
                max_area = area;
                next_a = j;
            }
        }
        out.push(data[next_a]);
        a = next_a;
    }
    out.push(data[end_index]);
    out
}

/// Min-max: per horizontal pixel, keep the first, last, min and max points.
pub fn min_max(data: &[Parsed], start: usize, count: usize, available_width: f64) -> Vec<Parsed> {
    let x = |i: usize| data[i].x.unwrap_or(f64::NAN);
    let y = |i: usize| data[i].y.unwrap_or(f64::NAN);
    let end_index = start + count - 1;
    let (x_min, x_max) = (x(start), x(end_index));
    let dx = x_max - x_min;
    let mut out: Vec<Parsed> = Vec::new();
    let (mut avg_x, mut count_x) = (0.0, 0.0);
    let mut prev_x: Option<i64> = None;
    let (mut min_y, mut max_y) = (0.0, 0.0);
    let (mut min_i, mut max_i, mut start_i): (Option<usize>, Option<usize>, Option<usize>) =
        (None, None, None);
    for i in start..start + count {
        let px = (x(i) - x_min) / dx * available_width;
        let yy = y(i);
        let trunc = px as i64; // `x | 0`
        if Some(trunc) == prev_x {
            if yy < min_y {
                min_y = yy;
                min_i = Some(i);
            } else if yy > max_y {
                max_y = yy;
                max_i = Some(i);
            }
            count_x += 1.0;
            avg_x = ((count_x - 1.0) * avg_x + x(i)) / count_x;
        } else {
            let last = i.checked_sub(1);
            if let (Some(mi), Some(ma)) = (min_i, max_i) {
                let (i1, i2) = (mi.min(ma), mi.max(ma));
                if Some(i1) != start_i && Some(i1) != last {
                    out.push(Parsed { x: Some(avg_x), ..data[i1] });
                }
                if Some(i2) != start_i && Some(i2) != last {
                    out.push(Parsed { x: Some(avg_x), ..data[i2] });
                }
            }
            if i > 0
                && last != start_i
                && let Some(l) = last
            {
                out.push(data[l]);
            }
            out.push(data[i]);
            prev_x = Some(trunc);
            count_x = 0.0;
            min_y = yy;
            max_y = yy;
            min_i = Some(i);
            max_i = Some(i);
            start_i = Some(i);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn series(n: usize) -> Vec<Parsed> {
        (0..n)
            .map(|i| Parsed {
                x: Some(i as f64),
                y: Some(((i as f64) * 0.37).sin() * 100.0),
                ..Parsed::default()
            })
            .collect()
    }

    #[test]
    fn lttb_keeps_ends_and_count() {
        let d = series(10_000);
        let out = lttb(&d, 0, d.len(), 200);
        assert_eq!(out.len(), 200);
        assert_eq!(out[0].x, Some(0.0));
        assert_eq!(out[199].x, Some(9999.0));
    }

    #[test]
    fn min_max_preserves_extremes() {
        let d = series(10_000);
        let out = min_max(&d, 0, d.len(), 100.0);
        assert!(out.len() < 600, "{}", out.len());
        let ymax = |v: &[Parsed]| v.iter().filter_map(|p| p.y).fold(f64::MIN, f64::max);
        let ymin = |v: &[Parsed]| v.iter().filter_map(|p| p.y).fold(f64::MAX, f64::min);
        assert_eq!(ymax(&out), ymax(&d));
        assert_eq!(ymin(&out), ymin(&d));
    }
}
