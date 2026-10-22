//! DSP: Butterworth filters designed like `scipy.signal.butter(..., output="sos")` (analog prototype, pre-warped bilinear
//! transform) and run like `sosfilt` (direct form II transposed, zero initial state); FFT convolution (overlap-add);
//! 4x polyphase oversampling (Kaiser-windowed sinc, scipy `resample_poly` style) for true-peak work.
use num_complex::Complex64 as C;
use rustfft::FftPlanner;
use std::f64::consts::PI;

pub type Sos = Vec<[f64; 6]>;

#[derive(Clone, Copy, Debug)]
pub enum Kind {
    Low(f64),
    High(f64),
    Band(f64, f64),
}

fn proto(n: usize) -> Vec<C> {
    (0..n).map(|k| C::from_polar(1.0, PI * (2 * k + n + 1) as f64 / (2 * n) as f64)).collect()
}

/// Butterworth design as second-order sections.
pub fn butter(order: usize, kind: Kind, fs: f64) -> Sos {
    let warp = |f: f64| 2.0 * fs * (PI * f / fs).tan();
    let p0 = proto(order);
    let (z, p, k): (Vec<C>, Vec<C>, f64) = match kind {
        Kind::Low(f) => {
            let wc = warp(f);
            (vec![], p0.iter().map(|p| p * wc).collect(), wc.powi(order as i32))
        }
        Kind::High(f) => {
            let wc = warp(f);
            let prod: C = p0.iter().map(|p| -p).product();
            (vec![C::new(0.0, 0.0); order], p0.iter().map(|p| wc / p).collect(), (C::new(1.0, 0.0) / prod).re)
        }
        Kind::Band(a, b) => {
            let (w1, w2) = (warp(a), warp(b));
            let bw = w2 - w1;
            let w0 = (w1 * w2).sqrt();
            let mut ps = vec![];
            for p in &p0 {
                let h = p * bw / 2.0;
                let d = (h * h - w0 * w0).sqrt();
                ps.push(h + d);
                ps.push(h - d);
            }
            (vec![C::new(0.0, 0.0); order], ps, bw.powi(order as i32))
        }
    };
    // bilinear transform
    let fs2 = 2.0 * fs;
    let zd: Vec<C> = z.iter().map(|z| (fs2 + z) / (fs2 - z)).chain(std::iter::repeat(C::new(-1.0, 0.0)).take(p.len() - z.len())).collect();
    let pd: Vec<C> = p.iter().map(|p| (fs2 + p) / (fs2 - p)).collect();
    let num: C = z.iter().map(|z| fs2 - z).product::<C>();
    let den: C = p.iter().map(|p| fs2 - p).product::<C>();
    let kd = k * (num / den).re;
    // pair poles into sections: conjugate pairs (by imaginary part), real poles paired together
    let mut cpx: Vec<C> = pd.iter().filter(|p| p.im > 1e-12).cloned().collect();
    cpx.sort_by(|a, b| (a.norm()).partial_cmp(&b.norm()).unwrap());
    let mut reals: Vec<f64> = pd.iter().filter(|p| p.im.abs() <= 1e-12).map(|p| p.re).collect();
    reals.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mut zs: Vec<f64> = zd.iter().map(|z| z.re).collect();
    let mut sos: Sos = vec![];
    let take_z = |zs: &mut Vec<f64>, n: usize| -> (f64, f64) {
        let a = if n > 0 { zs.pop() } else { None };
        let b = if n > 1 { zs.pop() } else { None };
        match (a, b) {
            (Some(a), Some(b)) => (-(a + b), a * b),
            (Some(a), None) => (-a, 0.0),
            _ => (0.0, 0.0),
        }
    };
    for p in &cpx {
        let (b1, b2) = take_z(&mut zs, 2);
        sos.push([1.0, b1, b2, 1.0, -2.0 * p.re, p.norm_sqr()]);
    }
    while !reals.is_empty() {
        let a = reals.pop().unwrap();
        if let Some(b) = reals.pop() {
            let (b1, b2) = take_z(&mut zs, 2);
            sos.push([1.0, b1, b2, 1.0, -(a + b), a * b]);
        } else {
            let (b1, b2) = take_z(&mut zs, 1);
            sos.push([1.0, b1, b2, 1.0, -a, 0.0]);
        }
    }
    sos[0][0] *= kd;
    sos[0][1] *= kd;
    sos[0][2] *= kd;
    sos
}

/// filter in place (direct form II transposed, zero initial conditions)
pub fn sosfilt(sos: &Sos, x: &mut [f64]) {
    for s in sos {
        let (b0, b1, b2, a1, a2) = (s[0], s[1], s[2], s[4], s[5]);
        let (mut z1, mut z2) = (0.0, 0.0);
        for v in x.iter_mut() {
            let xi = *v;
            let y = b0 * xi + z1;
            z1 = b1 * xi - a1 * y + z2;
            z2 = b2 * xi - a2 * y;
            *v = y;
        }
    }
}

pub fn filt(x: &[f64], order: usize, kind: Kind, fs: f64) -> Vec<f64> {
    let mut y = x.to_vec();
    sosfilt(&butter(order, kind, fs), &mut y);
    y
}

/// linear convolution truncated to `x.len()` (overlap-add FFT)
pub fn fftconvolve(x: &[f64], h: &[f64]) -> Vec<f64> {
    let n = x.len();
    let m = h.len();
    let mut size = 1;
    while size < 2 * m {
        size <<= 1;
    }
    size = size.max(1 << 16);
    let block = size - m + 1;
    let mut planner = FftPlanner::<f64>::new();
    let fwd = planner.plan_fft_forward(size);
    let inv = planner.plan_fft_inverse(size);
    let mut hf: Vec<rustfft::num_complex::Complex<f64>> = (0..size).map(|i| rustfft::num_complex::Complex::new(if i < m { h[i] } else { 0.0 }, 0.0)).collect();
    fwd.process(&mut hf);
    let mut out = vec![0.0; n + size];
    let mut buf = vec![rustfft::num_complex::Complex::new(0.0, 0.0); size];
    let mut a = 0;
    while a < n {
        let b = (a + block).min(n);
        for (i, v) in buf.iter_mut().enumerate() {
            *v = rustfft::num_complex::Complex::new(if i < b - a { x[a + i] } else { 0.0 }, 0.0);
        }
        fwd.process(&mut buf);
        for (v, hv) in buf.iter_mut().zip(hf.iter()) {
            *v *= hv;
        }
        inv.process(&mut buf);
        for i in 0..size {
            out[a + i] += buf[i].re / size as f64;
        }
        a = b;
    }
    out.truncate(n);
    out
}

fn bessel_i0(x: f64) -> f64 {
    let mut s = 1.0;
    let mut t = 1.0;
    for k in 1..50 {
        t *= (x / (2.0 * k as f64)).powi(2);
        s += t;
    }
    s
}

/// low-pass FIR for factor-`up` interpolation (scipy resample_poly: half_len = 10*up, Kaiser beta 5)
fn interp_fir(up: usize) -> Vec<f64> {
    let half = 10 * up;
    let n = 2 * half + 1;
    let fc = 1.0 / up as f64;
    let beta = 5.0;
    (0..n)
        .map(|i| {
            let m = i as f64 - half as f64;
            let sinc = if m == 0.0 { 1.0 } else { (PI * fc * m).sin() / (PI * fc * m) };
            let w = bessel_i0(beta * (1.0 - (2.0 * i as f64 / (n - 1) as f64 - 1.0).powi(2)).sqrt()) / bessel_i0(beta);
            fc * sinc * w * up as f64
        })
        .collect()
}

/// upsample by `up` (zero-stuff + FIR), output length x.len() * up, delay-compensated
pub fn upsample(x: &[f64], up: usize) -> Vec<f64> {
    let h = interp_fir(up);
    let half = h.len() / 2;
    let n = x.len() * up;
    let mut out = vec![0.0; n];
    // polyphase: out[j] = sum_k x[k] h[j - k*up + half]
    for (j, o) in out.iter_mut().enumerate() {
        let jj = j + half;
        let kmax = jj / up;
        let mut s = 0.0;
        let mut k = kmax as isize;
        while k >= 0 {
            let idx = jj as isize - k * up as isize;
            if idx as usize >= h.len() {
                break;
            }
            if (k as usize) < x.len() {
                s += x[k as usize] * h[idx as usize];
            }
            k -= 1;
        }
        *o = s;
    }
    out
}

/// decimate by `down` after the same anti-alias FIR
pub fn downsample(x: &[f64], down: usize) -> Vec<f64> {
    let h: Vec<f64> = interp_fir(down).iter().map(|v| v / down as f64).collect();
    let half = h.len() / 2;
    let n = x.len() / down;
    (0..n)
        .map(|i| {
            let c = i * down;
            let mut s = 0.0;
            for (k, hv) in h.iter().enumerate() {
                let idx = c as isize + k as isize - half as isize;
                if idx >= 0 && (idx as usize) < x.len() {
                    s += x[idx as usize] * hv;
                }
            }
            s
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lowpass_matches_scipy() {
        // scipy.signal.butter(2, 1000, fs=48000, output="sos")
        let s = butter(2, Kind::Low(1000.0), 48000.0);
        let r = [0.003916126660547, 0.007832253321095, 0.003916126660547, 1.0, -1.815341082704568, 0.831005589346757];
        for i in 0..6 {
            assert!((s[0][i] - r[i]).abs() < 1e-9, "{i}: {} vs {}", s[0][i], r[i]);
        }
    }
    #[test]
    fn upsample_dc() {
        let x = vec![1.0; 400];
        let y = upsample(&x, 4);
        assert!((y[800] - 1.0).abs() < 1e-3);
    }
}
