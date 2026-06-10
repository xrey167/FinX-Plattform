//! Shared smoothing primitive used by Wilder-style indicators.

#[allow(clippy::cast_precision_loss)]
const fn len_f64(n: usize) -> f64 {
    n as f64
}

/// Wilder's running smoothing of `values` over `length`, returning a column of
/// the same length with `None` until the first full window is seen.
///
/// Wilder seeds the smoothed series with the *sum* of the first `length` values
/// (not the mean — this is the form used for the directional-movement and TR
/// accumulators inside ADX), then recurses
/// `s_t = s_{t−1} − s_{t−1}/length + value_t`. When `length == 0` (which the
/// callers reject before reaching here) the result is all-`None`.
pub fn wilder_smooth(values: &[f64], length: usize) -> Vec<Option<f64>> {
    let mut out = vec![None; values.len()];
    if length == 0 || values.len() < length {
        return out;
    }
    let mut acc: f64 = values[..length].iter().sum();
    out[length - 1] = Some(acc);
    for (i, &value) in values.iter().enumerate().skip(length) {
        acc = acc - acc / len_f64(length) + value;
        out[i] = Some(acc);
    }
    out
}

/// Wilder's running *average* of `values` over `length` (the ADX form): seed
/// with the simple mean of the first `length` values, then recurse
/// `avg_t = (avg_{t−1}·(length−1) + value_t)/length`. Returns a same-length
/// column with `None` until the first full window is seen.
pub fn wilder_average(values: &[f64], length: usize) -> Vec<Option<f64>> {
    let mut out = vec![None; values.len()];
    if length == 0 || values.len() < length {
        return out;
    }
    let mut avg: f64 = values[..length].iter().sum::<f64>() / len_f64(length);
    out[length - 1] = Some(avg);
    for (i, &value) in values.iter().enumerate().skip(length) {
        avg = (avg.mul_add(len_f64(length - 1), value)) / len_f64(length);
        out[i] = Some(avg);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{wilder_average, wilder_smooth};

    #[test]
    fn average_seeds_with_mean() {
        // length 2 over [2,4,6]: seed = mean(2,4) = 3 at idx1;
        // idx2 = (3*1 + 6)/2 = 4.5.
        let out = wilder_average(&[2.0, 4.0, 6.0], 2);
        assert_eq!(out[0], None);
        assert!((out[1].unwrap_or(0.0) - 3.0).abs() < 1e-12);
        assert!((out[2].unwrap_or(0.0) - 4.5).abs() < 1e-12);
    }

    #[test]
    fn seeds_with_sum_then_recurses() {
        // length 2 over [1,2,3]: seed = 1+2 = 3 at idx1; idx2 = 3 - 3/2 + 3 = 4.5.
        let out = wilder_smooth(&[1.0, 2.0, 3.0], 2);
        assert_eq!(out[0], None);
        assert!((out[1].unwrap_or(0.0) - 3.0).abs() < 1e-12);
        assert!((out[2].unwrap_or(0.0) - 4.5).abs() < 1e-12);
    }

    #[test]
    fn short_input_is_all_none() {
        assert_eq!(wilder_smooth(&[1.0], 3), vec![None]);
    }
}
