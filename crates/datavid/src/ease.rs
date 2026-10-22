//! Easing and interpolation helpers shared by beats, cameras and the score.

pub fn clamp(x: f64) -> f64 {
    x.clamp(0.0, 1.0)
}
/// cubic ease-out
pub fn eout(x: f64) -> f64 {
    let x = clamp(x);
    1.0 - (1.0 - x).powi(3)
}
/// quintic ease-out
pub fn eout5(x: f64) -> f64 {
    let x = clamp(x);
    1.0 - (1.0 - x).powi(5)
}
/// cubic ease-in-out
pub fn einout(x: f64) -> f64 {
    let x = clamp(x);
    if x < 0.5 { 4.0 * x.powi(3) } else { 1.0 - (-2.0 * x + 2.0).powi(3) / 2.0 }
}
pub fn lerp(a: f64, b: f64, k: f64) -> f64 {
    a + (b - a) * k
}
/// exponential decay that starts at `t0` (0 before it)
pub fn decay(t: f64, t0: f64, tau: f64) -> f64 {
    if t >= t0 { (-(t - t0) / tau).exp() } else { 0.0 }
}
/// piecewise-linear interpolation, `xs` ascending, clamped at both ends (numpy.interp semantics)
pub fn interp(x: f64, xs: &[f64], ys: &[f64]) -> f64 {
    if x <= xs[0] {
        return ys[0];
    }
    let n = xs.len();
    if x >= xs[n - 1] {
        return ys[n - 1];
    }
    let i = xs.partition_point(|v| *v <= x) - 1;
    let k = (x - xs[i]) / (xs[i + 1] - xs[i]);
    ys[i] + (ys[i + 1] - ys[i]) * k
}
/// fade in over `fin` from `t0`, fade out over `fout` from `t1`
pub fn window(t: f64, t0: f64, fin: f64, t1: f64, fout: f64) -> f64 {
    clamp((t - t0) / fin) * (1.0 - clamp((t - t1) / fout))
}
