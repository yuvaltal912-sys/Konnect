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

/// Solve `a x = b` by Gaussian elimination with partial pivoting. None for a
/// singular system.
fn solve(mut a: Vec<Vec<f64>>, mut b: Vec<f64>) -> Option<Vec<f64>> {
    let n = b.len();
    for column in 0..n {
        let pivot =
            (column..n).max_by(|&i, &j| a[i][column].abs().total_cmp(&a[j][column].abs()))?;
        if !a[pivot][column].is_finite() || a[pivot][column].abs() < 1e-300 {
            return None;
        }
        a.swap(column, pivot);
        b.swap(column, pivot);
        for row in column + 1..n {
            let factor = a[row][column] / a[column][column];
            let pivot_row = a[column].clone();
            for (cell, above) in a[row][column..].iter_mut().zip(&pivot_row[column..]) {
                *cell -= factor * above;
            }
            b[row] -= factor * b[column];
        }
    }
    let mut x = vec![0.0; n];
    for row in (0..n).rev() {
        let tail: f64 = (row + 1..n).map(|k| a[row][k] * x[k]).sum();
        x[row] = (b[row] - tail) / a[row][row];
    }
    x.iter().all(|value| value.is_finite()).then_some(x)
}

/// The parameters that make the squares of `residuals` least, by
/// Levenberg-Marquardt with a numeric Jacobian. `clamp` puts a trial point back
/// inside the parameters' limits. A start that gives no finite residuals is
/// returned as it is.
pub(crate) fn least_squares(
    residuals: impl Fn(&[f64]) -> Vec<f64>,
    start: &[f64],
    clamp: impl Fn(&mut [f64]),
) -> Vec<f64> {
    let n = start.len();
    let mut x = start.to_vec();
    clamp(&mut x);
    let cost = |r: &[f64]| -> f64 { r.iter().map(|value| value * value).sum() };
    let mut r = residuals(&x);
    let mut c = cost(&r);
    if !c.is_finite() {
        return x;
    }
    let mut lambda = 1e-3;
    for _ in 0..300 {
        let m = r.len();
        let mut jacobian = vec![vec![0.0; n]; m];
        for k in 0..n {
            let step = 1e-6 * x[k].abs().max(1e-3);
            let (mut up, mut down) = (x.clone(), x.clone());
            up[k] += step;
            down[k] -= step;
            clamp(&mut up);
            clamp(&mut down);
            let width = up[k] - down[k];
            if width == 0.0 {
                continue;
            }
            let (high, low) = (residuals(&up), residuals(&down));
            for i in 0..m {
                jacobian[i][k] = (high[i] - low[i]) / width;
            }
        }
        let mut normal = vec![vec![0.0; n]; n];
        let mut gradient = vec![0.0; n];
        for i in 0..m {
            for a in 0..n {
                gradient[a] += jacobian[i][a] * r[i];
                for b in 0..n {
                    normal[a][b] += jacobian[i][a] * jacobian[i][b];
                }
            }
        }
        let mut accepted = false;
        for _ in 0..30 {
            let mut damped = normal.clone();
            for a in 0..n {
                damped[a][a] += lambda * normal[a][a].max(1e-12);
            }
            let rhs: Vec<f64> = gradient.iter().map(|value| -value).collect();
            if let Some(delta) = solve(damped, rhs) {
                let mut trial: Vec<f64> = x.iter().zip(&delta).map(|(a, d)| a + d).collect();
                clamp(&mut trial);
                let trial_r = residuals(&trial);
                let trial_c = cost(&trial_r);
                if trial_c.is_finite() && trial_c < c {
                    let gain = c - trial_c;
                    x = trial;
                    r = trial_r;
                    c = trial_c;
                    lambda = (lambda / 3.0).max(1e-12);
                    accepted = true;
                    if gain <= 1e-15 * c.max(1e-300) {
                        return x;
                    }
                    break;
                }
            }
            lambda *= 4.0;
        }
        if !accepted {
            break;
        }
    }
    x
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
    fn least_squares_recovers_an_exponential_and_respects_a_clamp() {
        // y = a exp(b x) through four exact points.
        let xs = [0.0_f64, 1.0, 2.0, 3.0];
        let ys: Vec<f64> = xs.iter().map(|x| 2.0 * (0.5 * x).exp()).collect();
        let found = least_squares(
            |p| {
                xs.iter()
                    .zip(&ys)
                    .map(|(x, y)| p[0] * (p[1] * x).exp() - y)
                    .collect()
            },
            &[1.0, 0.1],
            |_| {},
        );
        assert!(
            close(found[0], 2.0, 1e-6) && close(found[1], 0.5, 1e-6),
            "{found:?}"
        );
        // The unconstrained best b is 0.5; held at 0.3 the fit stops there.
        let held = least_squares(
            |p| {
                xs.iter()
                    .zip(&ys)
                    .map(|(x, y)| p[0] * (p[1] * x).exp() - y)
                    .collect()
            },
            &[1.0, 0.1],
            |p| p[1] = p[1].min(0.3),
        );
        assert!((held[1] - 0.3).abs() < 1e-9, "{held:?}");
    }

    #[test]
    fn a_root_is_found_and_a_missing_one_is_none() {
        let x = root(|x| x * x - 2.0, 0.0, 2.0).unwrap();
        assert!(close(x, std::f64::consts::SQRT_2, 1e-12));
        assert!(root(|x| x * x + 1.0, 0.0, 2.0).is_none());
    }
}
