//! Small numerical tools for fitting model parameters to datasheet values.

/// Boltzmann's constant over the electron charge, in volts per kelvin.
pub(crate) const K_OVER_Q: f64 = 8.617_333_262e-5;
pub(crate) const ZERO_CELSIUS: f64 = 273.15;

/// The thermal voltage `kT/q` at a temperature in degrees Celsius.
pub(crate) fn thermal_voltage(celsius: f64) -> f64 {
    K_OVER_Q * (celsius + ZERO_CELSIUS)
}

/// The least-squares line `y = slope * x + intercept`, or none when the points
/// cannot fix one.
pub(crate) fn line(xs: &[f64], ys: &[f64]) -> Option<(f64, f64)> {
    let count = xs.len();
    if count < 2 || ys.len() != count {
        return None;
    }
    let n = count as f64;
    let mean_x = xs.iter().sum::<f64>() / n;
    let mean_y = ys.iter().sum::<f64>() / n;
    let spread: f64 = xs.iter().map(|x| (x - mean_x).powi(2)).sum();
    if spread <= 0.0 || !spread.is_finite() {
        return None;
    }
    let covariance: f64 = xs
        .iter()
        .zip(ys)
        .map(|(x, y)| (x - mean_x) * (y - mean_y))
        .sum();
    let slope = covariance / spread;
    Some((slope, mean_y - slope * mean_x))
}

/// Where `f` is least on `[low, high]`, for an `f` with one minimum there.
pub(crate) fn golden_minimum(f: impl Fn(f64) -> f64, mut low: f64, mut high: f64) -> f64 {
    const RATIO: f64 = 0.618_033_988_749_894_9;
    const STEPS: usize = 200;
    let mut left = high - RATIO * (high - low);
    let mut right = low + RATIO * (high - low);
    let mut f_left = f(left);
    let mut f_right = f(right);
    for _ in 0..STEPS {
        if (high - low).abs() <= f64::EPSILON * (low.abs() + high.abs()).max(f64::MIN_POSITIVE) {
            break;
        }
        if f_left <= f_right {
            high = right;
            right = left;
            f_right = f_left;
            left = high - RATIO * (high - low);
            f_left = f(left);
        } else {
            low = left;
            left = right;
            f_left = f_right;
            right = low + RATIO * (high - low);
            f_right = f(right);
        }
    }
    0.5 * (low + high)
}

/// The `x` in `[low, high]` where an increasing or decreasing `f` crosses zero.
/// None when it does not change sign there.
pub(crate) fn root(f: impl Fn(f64) -> f64, mut low: f64, mut high: f64) -> Option<f64> {
    let mut f_low = f(low);
    let f_high = f(high);
    if !(f_low.is_finite() && f_high.is_finite()) || f_low.signum() == f_high.signum() {
        return (f_low == 0.0)
            .then_some(low)
            .or((f_high == 0.0).then_some(high));
    }
    for _ in 0..300 {
        let middle = 0.5 * (low + high);
        let f_middle = f(middle);
        if f_middle == 0.0 || (high - low).abs() <= 1e-15 * middle.abs().max(1e-300) {
            return Some(middle);
        }
        if f_middle.signum() == f_low.signum() {
            low = middle;
            f_low = f_middle;
        } else {
            high = middle;
        }
    }
    Some(0.5 * (low + high))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64, tolerance: f64) -> bool {
        (a - b).abs() <= tolerance * b.abs().max(1e-300)
    }

    #[test]
    fn the_thermal_voltage_at_25_c() {
        assert!(close(thermal_voltage(25.0), 0.025_693, 1e-4));
    }

    #[test]
    fn a_line_through_exact_points_is_recovered() {
        let xs = [1.0, 2.0, 3.0, 4.0];
        let ys: Vec<f64> = xs.iter().map(|x| 2.5 * x - 1.0).collect();
        let (slope, intercept) = line(&xs, &ys).unwrap();
        assert!(close(slope, 2.5, 1e-12) && close(intercept, -1.0, 1e-12));
        assert!(line(&[1.0, 1.0], &[2.0, 3.0]).is_none());
        assert!(line(&[1.0], &[2.0]).is_none());
    }

    #[test]
    fn the_golden_section_finds_a_minimum() {
        let x = golden_minimum(|x| (x - 0.37).powi(2) + 1.0, 0.0, 2.0);
        // A minimum is flat, so only about half the digits can be resolved.
        assert!(close(x, 0.37, 1e-6));
    }

    #[test]
    fn a_root_is_found_and_a_missing_one_is_none() {
        let x = root(|x| x * x - 2.0, 0.0, 2.0).unwrap();
        assert!(close(x, std::f64::consts::SQRT_2, 1e-12));
        assert!(root(|x| x * x + 1.0, 0.0, 2.0).is_none());
    }
}
