//! DSP: Butterworth filters designed like `scipy.signal.butter(..., output="sos")` (analog prototype, pre-warped bilinear
//! transform) and run like `sosfilt` (direct form II transposed, zero initial state); FFT convolution (overlap-add);
//! 4x polyphase oversampling (Kaiser-windowed sinc, scipy `resample_poly` style) for true-peak work.
use num_complex::Complex64 as C;
use rustfft::FftPlanner;
use std::f64::consts::PI;
use std::sync::OnceLock;

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
fn interp_fir_uncached(up: usize) -> Vec<f64> {
    let half = 10 * up;
    let n = 2 * half + 1;
    let fc = 1.0 / up as f64;
    let beta = 5.0;
    let i0_beta = bessel_i0(beta);
    (0..n)
        .map(|i| {
            let m = i as f64 - half as f64;
            let sinc = if m == 0.0 { 1.0 } else { (PI * fc * m).sin() / (PI * fc * m) };
            let w = bessel_i0(beta * (1.0 - (2.0 * i as f64 / (n - 1) as f64 - 1.0).powi(2)).sqrt()) / i0_beta;
            fc * sinc * w * up as f64
        })
        .collect()
}

static FIR_UP_4: OnceLock<Vec<f64>> = OnceLock::new();
static FIR_DOWN_4: OnceLock<Vec<f64>> = OnceLock::new();

fn interp_fir(up: usize) -> Vec<f64> {
    if up == 4 {
        FIR_UP_4.get_or_init(|| interp_fir_uncached(4)).clone()
    } else {
        interp_fir_uncached(up)
    }
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
    let h: Vec<f64> = if down == 4 {
        FIR_DOWN_4.get_or_init(|| interp_fir_uncached(4).iter().map(|v| v / 4.0).collect()).clone()
    } else {
        interp_fir_uncached(down).iter().map(|v| v / down as f64).collect()
    };
    let half = h.len() / 2;
    let n = x.len() / down;
    (0..n)
        .map(|i| {
            let c = i * down;
            let mut s = 0.0;
            // precompute valid k range, then zip contiguous slices in ascending k order
            let mut lo = if half > c { half - c } else { 0 };
            if lo > h.len() {
                lo = h.len();
            }
            let hi_raw = x.len() + half - c;
            let mut hi = if hi_raw < h.len() { hi_raw } else { h.len() };
            if hi < lo {
                hi = lo;
            }
            let len = hi - lo;
            if len > 0 {
                let start = c + lo - half;
                let xs = &x[start..start + len];
                let hs = &h[lo..hi];
                for (xv, hv) in xs.iter().zip(hs.iter()) {
                    s += *xv * *hv;
                }
            }
            s
        })
        .collect()
}



/// prior main (origin/main) production downsample: cached FIR for down==4, uncached otherwise; simple per-tap accumulation
#[cfg(test)]
fn prior_main_downsample(x: &[f64], down: usize) -> Vec<f64> {
    let h: Vec<f64> = if down == 4 {
        FIR_DOWN_4.get_or_init(|| interp_fir_uncached(4).iter().map(|v| v / 4.0).collect()).clone()
    } else {
        interp_fir_uncached(down).iter().map(|v| v / down as f64).collect()
    };
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

    fn reference_interp_fir(up: usize) -> Vec<f64> {
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

    fn reference_upsample(x: &[f64], up: usize) -> Vec<f64> {
        let h = reference_interp_fir(up);
        let half = h.len() / 2;
        let n = x.len() * up;
        let mut out = vec![0.0; n];
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

    fn reference_downsample(x: &[f64], down: usize) -> Vec<f64> {
        let h: Vec<f64> = reference_interp_fir(down).iter().map(|v| v / down as f64).collect();
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

    fn reference_fftconvolve(x: &[f64], h: &[f64]) -> Vec<f64> {
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

    fn signal(kind: &str, n: usize, seed: u64) -> Vec<f64> {
        match kind {
            "zeros" => vec![0.0; n],
            "impulse" => {
                let mut v = vec![0.0; n];
                if n > 0 {
                    v[0] = 1.0;
                }
                v
            }
            _ => {
                let mut s = seed;
                (0..n)
                    .map(|_| {
                        s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                        ((s >> 11) as f64 / (1u64 << 53) as f64) * 2.0 - 1.0
                    })
                    .collect()
            }
        }
    }

    fn assert_bits_eq(a: &[f64], b: &[f64], what: &str) {
        assert_eq!(a.len(), b.len(), "{what} length");
        for (i, (x, y)) in a.iter().zip(b.iter()).enumerate() {
            assert_eq!(x.to_bits(), y.to_bits(), "{what}[{i}]: {x:?} vs {y:?}");
        }
    }

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

    #[test]
    fn interp_fir_matches_reference_bits() {
        for up in [1, 2, 3, 4, 5, 8] {
            let a = interp_fir(up);
            let b = reference_interp_fir(up);
            assert_bits_eq(&a, &b, &format!("interp_fir({up})"));
        }
        assert_bits_eq(&interp_fir(4), &interp_fir(4), "cached interp_fir(4)");
    }

    #[test]
    fn upsample_matches_reference_bits() {
        for up in [1, 2, 3, 4, 5] {
            for n in [0, 1, 2, 3, 4, 5, 7, 64] {
                for kind in ["zeros", "impulse", "random"] {
                    let x = signal(kind, n, 7 + n as u64 + up as u64);
                    let a = upsample(&x, up);
                    let b = reference_upsample(&x, up);
                    assert_bits_eq(&a, &b, &format!("upsample {kind} n={n} up={up}"));
                }
            }
        }
    }

    #[test]
    fn downsample_matches_reference_bits() {
        for down in [1, 2, 3, 4, 5] {
            for n in [0, 1, 2, 3, 4, 5, 8, 64, 65] {
                for kind in ["zeros", "impulse", "random"] {
                    let x = signal(kind, n, 13 + n as u64 + down as u64);
                    let a = downsample(&x, down);
                    let b = reference_downsample(&x, down);
                    assert_bits_eq(&a, &b, &format!("downsample {kind} n={n} down={down}"));
                }
            }
        }
    }

    #[test]
    fn fftconvolve_matches_reference_bits() {
        for n in [0, 1, 2, 3, 4, 17, 1024] {
            for m in [1, 2, 3, 4, 17, 64] {
                for kind in ["zeros", "impulse", "random"] {
                    let x = signal(kind, n, 21 + n as u64 + m as u64);
                    let h = signal("random", m, 99 + m as u64);
                    let a = fftconvolve(&x, &h);
                    let b = reference_fftconvolve(&x, &h);
                    assert_bits_eq(&a, &b, &format!("fftconvolve x={kind} n={n} m={m}"));
                }
            }
        }
    }

    #[test]
    #[ignore]
    fn bench_interp_fir() {
        use std::hint::black_box;
        use std::time::Instant;
        let reps = 20_000;
        let t0 = Instant::now();
        let mut a = 0.0;
        for _ in 0..reps {
            a += black_box(reference_interp_fir(4))[40];
        }
        let tr = t0.elapsed();
        let t1 = Instant::now();
        let mut b = 0.0;
        for _ in 0..reps {
            b += black_box(interp_fir(4))[40];
        }
        let tc = t1.elapsed();
        assert_eq!(a.to_bits(), b.to_bits());
        eprintln!("interp_fir(4): reference {tr:?}, candidate {tc:?}, equal {a:?}");
    }

    #[test]
    #[ignore]
    fn bench_resample_kernels() {
        use std::hint::black_box;
        use std::time::Instant;
        let x = signal("random", 48_000, 3);
        let reps = 20;
        let t0 = Instant::now();
        let mut r_up = vec![];
        for _ in 0..reps {
            r_up = black_box(reference_upsample(&x, 4));
        }
        let tr_up = t0.elapsed();
        let t1 = Instant::now();
        let mut c_up = vec![];
        for _ in 0..reps {
            c_up = black_box(upsample(&x, 4));
        }
        let tc_up = t1.elapsed();
        assert_bits_eq(&r_up, &c_up, "bench upsample");

        let ud = upsample(&x, 4);
        let t2 = Instant::now();
        let mut r_dn = vec![];
        for _ in 0..reps {
            r_dn = black_box(reference_downsample(&ud, 4));
        }
        let tr_dn = t2.elapsed();
        let t3 = Instant::now();
        let mut c_dn = vec![];
        for _ in 0..reps {
            c_dn = black_box(downsample(&ud, 4));
        }
        let tc_dn = t3.elapsed();
        assert_bits_eq(&r_dn, &c_dn, "bench downsample");
        eprintln!("upsample(4): reference {tr_up:?}, candidate {tc_up:?}");
        eprintln!("downsample(4): reference {tr_dn:?}, candidate {tc_dn:?}");
    }

    #[test]
    #[ignore]
    fn bench_fftconvolve() {
        use std::hint::black_box;
        use std::time::Instant;
        let x = signal("random", 48_000, 5);
        let h = reference_interp_fir(4);
        let reps = 50;
        let t0 = Instant::now();
        let mut r = vec![];
        for _ in 0..reps {
            r = black_box(reference_fftconvolve(&x, &h));
        }
        let tr = t0.elapsed();
        let t1 = Instant::now();
        let mut c = vec![];
        for _ in 0..reps {
            c = black_box(fftconvolve(&x, &h));
        }
        let tc = t1.elapsed();
        assert_bits_eq(&r, &c, "bench fftconvolve");
        eprintln!("fftconvolve: reference {tr:?}, candidate {tc:?}");
    }

    #[test]
    fn upsample_raw_bits_odd_nan_inf_signedzero() {
        for up in [1usize, 2, 3, 4, 8] {
            for n in [0usize, 1, 3, 5, 7] {
                let mut x = vec![0.0; n];
                if n > 0 { x[0] = f64::NAN; }
                if n > 1 { x[1] = f64::INFINITY; }
                if n > 2 { x[2] = f64::NEG_INFINITY; }
                if n > 3 { x[3] = -0.0; }
                if n > 4 { x[4] = 1.0; }
                if n > 5 { x[5] = -1.0; }
                if n > 6 { x[6] = 0.5; }
                let a = upsample(&x, up);
                let b = reference_upsample(&x, up);
                assert_bits_eq(&a, &b, &format!("upsample raw bits n={n} up={up}"));
            }
        }
    }

    #[test]
    fn downsample_raw_bits_odd_nan_inf_signedzero() {
        for down in [1usize, 2, 3, 4, 8] {
            for n in [0usize, 1, 3, 5, 7, 9, 65] {
                let mut x = vec![0.0; n];
                if n > 0 { x[0] = f64::NAN; }
                if n > 1 { x[1] = f64::INFINITY; }
                if n > 2 { x[2] = f64::NEG_INFINITY; }
                if n > 3 { x[3] = -0.0; }
                if n > 4 { x[4] = 1.0; }
                if n > 5 { x[5] = -1.0; }
                if n > 6 { x[6] = 0.5; }
                let a = downsample(&x, down);
                let b = reference_downsample(&x, down);
                assert_bits_eq(&a, &b, &format!("downsample raw bits n={n} down={down}"));
            }
        }
    }

    #[test]
    #[ignore]
    fn bench_upsample_median5_6s() {
        use std::hint::black_box;
        use std::time::Instant;
        let x = signal("random", 6 * 48_000, 3);
        let up = 4usize;
        let c = upsample(&x, up);
        let r = reference_upsample(&x, up);
        assert_bits_eq(&c, &r, "bench upsample 6s");
        black_box(&c);
        black_box(&x);
        let mut new_medians = Vec::new();
        let mut old_medians = Vec::new();
        for _ in 0..5 {
            let mut ts: Vec<std::time::Duration> = (0..30)
                .map(|_| {
                    let s = Instant::now();
                    let out = upsample(black_box(&x), up);
                    black_box(&out);
                    s.elapsed()
                })
                .collect();
            ts.sort();
            new_medians.push(ts[ts.len() / 2]);

            let mut ts: Vec<std::time::Duration> = (0..30)
                .map(|_| {
                    let s = Instant::now();
                    let out = reference_upsample(black_box(&x), up);
                    black_box(&out);
                    s.elapsed()
                })
                .collect();
            ts.sort();
            old_medians.push(ts[ts.len() / 2]);
        }
        eprintln!("upsample 6s medians new: {:?}", new_medians);
        eprintln!("upsample 6s medians old: {:?}", old_medians);
    }

    #[test]
    #[ignore]
    fn bench_upsample_median5_tiny() {
        use std::hint::black_box;
        use std::time::Instant;
        let x = signal("random", 37, 11);
        let up = 4usize;
        let c = upsample(&x, up);
        let r = reference_upsample(&x, up);
        assert_bits_eq(&c, &r, "bench upsample tiny");
        black_box(&c);
        black_box(&x);
        let mut new_medians = Vec::new();
        let mut old_medians = Vec::new();
        for _ in 0..5 {
            let mut ts: Vec<std::time::Duration> = (0..30)
                .map(|_| {
                    let s = Instant::now();
                    let out = upsample(black_box(&x), up);
                    black_box(&out);
                    s.elapsed()
                })
                .collect();
            ts.sort();
            new_medians.push(ts[ts.len() / 2]);

            let mut ts: Vec<std::time::Duration> = (0..30)
                .map(|_| {
                    let s = Instant::now();
                    let out = reference_upsample(black_box(&x), up);
                    black_box(&out);
                    s.elapsed()
                })
                .collect();
            ts.sort();
            old_medians.push(ts[ts.len() / 2]);
        }
        eprintln!("upsample tiny medians new: {:?}", new_medians);
        eprintln!("upsample tiny medians old: {:?}", old_medians);
    }

    #[test]
    #[ignore]
    fn bench_downsample_median5_6s() {
        use std::hint::black_box;
        use std::time::Instant;
        let x = signal("random", 6 * 48_000, 7);
        let down = 4usize;
        let c = downsample(&x, down);
        let r = prior_main_downsample(&x, down);
        assert_bits_eq(&c, &r, "bench downsample 6s");
        black_box(&c);
        black_box(&r);
        black_box(&x);
        let mut new_medians = Vec::new();
        let mut old_medians = Vec::new();
        for _ in 0..5 {
            let mut ts: Vec<std::time::Duration> = (0..30)
                .map(|_| {
                    let s = Instant::now();
                    let out = downsample(black_box(&x), down);
                    black_box(&out);
                    s.elapsed()
                })
                .collect();
            ts.sort();
            new_medians.push(ts[ts.len() / 2]);

            let mut ts: Vec<std::time::Duration> = (0..30)
                .map(|_| {
                    let s = Instant::now();
                    let out = prior_main_downsample(black_box(&x), down);
                    black_box(&out);
                    s.elapsed()
                })
                .collect();
            ts.sort();
            old_medians.push(ts[ts.len() / 2]);
        }
        eprintln!("downsample 6s medians new: {:?}", new_medians);
        eprintln!("downsample 6s medians old: {:?}", old_medians);
    }

    #[test]
    #[ignore]
    fn bench_downsample_median5_24s() {
        use std::hint::black_box;
        use std::time::Instant;
        let x = signal("random", 24 * 48_000, 7);
        let down = 4usize;
        let c = downsample(&x, down);
        let r = prior_main_downsample(&x, down);
        assert_bits_eq(&c, &r, "bench downsample 24s");
        black_box(&c);
        black_box(&r);
        black_box(&x);
        let mut new_medians = Vec::new();
        let mut old_medians = Vec::new();
        for _ in 0..5 {
            let mut ts: Vec<std::time::Duration> = (0..30)
                .map(|_| {
                    let s = Instant::now();
                    let out = downsample(black_box(&x), down);
                    black_box(&out);
                    s.elapsed()
                })
                .collect();
            ts.sort();
            new_medians.push(ts[ts.len() / 2]);

            let mut ts: Vec<std::time::Duration> = (0..30)
                .map(|_| {
                    let s = Instant::now();
                    let out = prior_main_downsample(black_box(&x), down);
                    black_box(&out);
                    s.elapsed()
                })
                .collect();
            ts.sort();
            old_medians.push(ts[ts.len() / 2]);
        }
        eprintln!("downsample 24s medians new: {:?}", new_medians);
        eprintln!("downsample 24s medians old: {:?}", old_medians);
    }

    #[test]
    #[ignore]
    fn bench_downsample_median5_tiny() {
        use std::hint::black_box;
        use std::time::Instant;
        let x = signal("random", 37, 13);
        let down = 4usize;
        let c = downsample(&x, down);
        let r = reference_downsample(&x, down);
        assert_bits_eq(&c, &r, "bench downsample tiny");
        black_box(&c);
        black_box(&x);
        let mut new_medians = Vec::new();
        let mut old_medians = Vec::new();
        for _ in 0..5 {
            let mut ts: Vec<std::time::Duration> = (0..30)
                .map(|_| {
                    let s = Instant::now();
                    let out = downsample(black_box(&x), down);
                    black_box(&out);
                    s.elapsed()
                })
                .collect();
            ts.sort();
            new_medians.push(ts[ts.len() / 2]);

            let mut ts: Vec<std::time::Duration> = (0..30)
                .map(|_| {
                    let s = Instant::now();
                    let out = reference_downsample(black_box(&x), down);
                    black_box(&out);
                    s.elapsed()
                })
                .collect();
            ts.sort();
            old_medians.push(ts[ts.len() / 2]);
        }
        eprintln!("downsample tiny medians new: {:?}", new_medians);
        eprintln!("downsample tiny medians old: {:?}", old_medians);
    }

    #[test]
    #[ignore]
    fn bench_downsample_median5_oversampled6s() {
        use std::hint::black_box;
        use std::time::Instant;
        let x = signal("random", 1_152_000, 7);
        let down = 4usize;
        let c = downsample(&x, down);
        let r = prior_main_downsample(&x, down);
        assert_bits_eq(&c, &r, "bench downsample oversampled6s");
        black_box(&c);
        black_box(&r);
        black_box(&x);
        let mut new_medians = Vec::new();
        let mut old_medians = Vec::new();
        for round in 0..5usize {
            if round % 2 == 0 {
                let mut ts: Vec<std::time::Duration> = (0..30)
                    .map(|_| {
                        let s = Instant::now();
                        let out = prior_main_downsample(black_box(&x), down);
                        black_box(&out);
                        s.elapsed()
                    })
                    .collect();
                ts.sort();
                old_medians.push(ts[ts.len() / 2]);

                let mut ts: Vec<std::time::Duration> = (0..30)
                    .map(|_| {
                        let s = Instant::now();
                        let out = downsample(black_box(&x), down);
                        black_box(&out);
                        s.elapsed()
                    })
                    .collect();
                ts.sort();
                new_medians.push(ts[ts.len() / 2]);
            } else {
                let mut ts: Vec<std::time::Duration> = (0..30)
                    .map(|_| {
                        let s = Instant::now();
                        let out = downsample(black_box(&x), down);
                        black_box(&out);
                        s.elapsed()
                    })
                    .collect();
                ts.sort();
                new_medians.push(ts[ts.len() / 2]);

                let mut ts: Vec<std::time::Duration> = (0..30)
                    .map(|_| {
                        let s = Instant::now();
                        let out = prior_main_downsample(black_box(&x), down);
                        black_box(&out);
                        s.elapsed()
                    })
                    .collect();
                ts.sort();
                old_medians.push(ts[ts.len() / 2]);
            }
        }
        eprintln!("downsample oversampled6s medians new: {:?}", new_medians);
        eprintln!("downsample oversampled6s medians old: {:?}", old_medians);
    }
}
