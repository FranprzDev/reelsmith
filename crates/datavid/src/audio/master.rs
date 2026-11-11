//! Loudness mastering: glue saturation, a 4x-oversampled look-ahead true-peak limiter and gain iterations until the
//! integrated loudness (EBU R128 / ITU-R BS.1770, `ebur128` crate) hits the target. Also a plain measurement function.
use super::dsp;
use anyhow::Result;
use ebur128::{EbuR128, Mode};

#[derive(Debug, Clone)]
pub struct Target {
    pub lufs: f64,
    /// limiter ceiling in dBTP (below the delivery limit: AAC encoding overshoots ~0.5–1 dB)
    pub ceiling_db: f64,
    /// optional extra ceiling (dB, negative = lower) as (time s, dB) points over the timeline
    pub ceiling_env: Vec<(f64, f64)>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Loudness {
    pub integrated_lufs: f64,
    pub true_peak_dbtp: f64,
    pub lra: f64,
    pub seconds: f64,
}

pub fn measure(x: &[Vec<f64>; 2], sr: usize) -> Result<Loudness> {
    let mut m = EbuR128::new(2, sr as u32, Mode::I | Mode::LRA | Mode::TRUE_PEAK)?;
    let n = x[0].len();
    let mut inter: Vec<f64> = Vec::with_capacity(4096 * 2);
    if n == 0 {
        m.add_frames_f64(&inter)?;
    } else {
        let mut i = 0usize;
        while i < n {
            let end = (i + 4096).min(n);
            inter.clear();
            for j in i..end {
                inter.push(x[0][j]);
                inter.push(x[1][j]);
            }
            m.add_frames_f64(&inter)?;
            i = end;
        }
    }
    let tp = m.true_peak(0)?.max(m.true_peak(1)?);
    Ok(Loudness { integrated_lufs: m.loudness_global()?, true_peak_dbtp: 20.0 * tp.max(1e-12).log10(), lra: m.loudness_range()?, seconds: x[0].len() as f64 / sr as f64 })
}

/// hold-min over ±la samples then a box average of width la: the gain never exceeds what each sample needs
fn smooth_gain(need: &[f64], la: usize) -> Vec<f64> {
    let n = need.len();
    let mut g = vec![0.0; n];
    let mut dq: std::collections::VecDeque<usize> = std::collections::VecDeque::new();
    let mut j = 0;
    for c in 0..n {
        let hi = (c + la).min(n - 1);
        while j <= hi {
            while let Some(&b) = dq.back() {
                if need[b] >= need[j] { dq.pop_back(); } else { break; }
            }
            dq.push_back(j);
            j += 1;
        }
        let lo = c.saturating_sub(la);
        while let Some(&f) = dq.front() {
            if f < lo { dq.pop_front(); } else { break; }
        }
        g[c] = need[*dq.front().unwrap()];
    }
    let half = la / 2;
    let mut pre = vec![0.0; n + 1];
    for i in 0..n {
        pre[i + 1] = pre[i] + g[i];
    }
    (0..n)
        .map(|i| {
            let a = i.saturating_sub(half);
            let b = (i + la - half).min(n);
            (pre[b] - pre[a]) / (b - a) as f64
        })
        .collect()
}

enum Ceiling {
    Constant(f64),
    Curve(Vec<f64>),
}

impl Ceiling {
    fn at(&self, i: usize) -> f64 {
        match self {
            Ceiling::Constant(c) => *c,
            Ceiling::Curve(v) => v[i],
        }
    }
}

/// A materialised curve costs 8 bytes per oversampled sample (~276 MB for 180 s at 48 kHz). Above this bound the
/// original per-iteration env path is used instead, trading the per-sample cache for bounded memory.
const MAX_CEILING_CURVE_SAMPLES: usize = 1 << 22;

fn ceiling_curve(n: usize, sr: usize, t: &Target) -> Vec<f64> {
    let xs: Vec<f64> = t.ceiling_env.iter().map(|p| p.0).collect();
    let ys: Vec<f64> = t.ceiling_env.iter().map(|p| p.1).collect();
    (0..n)
        .map(|i| {
            let extra = crate::ease::interp(i as f64 / (4.0 * sr as f64), &xs, &ys);
            10f64.powf((t.ceiling_db + extra) / 20.0)
        })
        .collect()
}

fn ceiling_for(t: &Target, n: usize, sr: usize) -> Option<Ceiling> {
    if t.ceiling_env.is_empty() {
        Some(Ceiling::Constant(10f64.powf(t.ceiling_db / 20.0)))
    } else if n <= MAX_CEILING_CURVE_SAMPLES {
        Some(Ceiling::Curve(ceiling_curve(n, sr, t)))
    } else {
        None
    }
}

fn finish_limit(up: [Vec<f64>; 2], need: &[f64], sr: usize) -> [Vec<f64>; 2] {
    let la = 4 * (0.025 * sr as f64) as usize;
    let g = smooth_gain(need, la);
    let mut out = [vec![], vec![]];
    for ch in 0..2 {
        let y: Vec<f64> = up[ch].iter().zip(g.iter()).map(|(a, b)| a * b).collect();
        out[ch] = dsp::downsample(&y, 4);
    }
    out
}

fn limit_with_ceiling(x: &[Vec<f64>; 2], sr: usize, ceiling: &Ceiling) -> [Vec<f64>; 2] {
    let up = [dsp::upsample(&x[0], 4), dsp::upsample(&x[1], 4)];
    let n = up[0].len();
    let need: Vec<f64> = (0..n)
        .map(|i| {
            let c = ceiling.at(i);
            let pk = up[0][i].abs().max(up[1][i].abs()).max(1e-9);
            (c / pk).min(1.0)
        })
        .collect();
    finish_limit(up, &need, sr)
}

fn limit_with_env(x: &[Vec<f64>; 2], sr: usize, t: &Target) -> [Vec<f64>; 2] {
    let up = [dsp::upsample(&x[0], 4), dsp::upsample(&x[1], 4)];
    let n = up[0].len();
    let xs: Vec<f64> = t.ceiling_env.iter().map(|p| p.0).collect();
    let ys: Vec<f64> = t.ceiling_env.iter().map(|p| p.1).collect();
    let need: Vec<f64> = (0..n)
        .map(|i| {
            let extra = crate::ease::interp(i as f64 / (4.0 * sr as f64), &xs, &ys);
            let c = 10f64.powf((t.ceiling_db + extra) / 20.0);
            let pk = up[0][i].abs().max(up[1][i].abs()).max(1e-9);
            (c / pk).min(1.0)
        })
        .collect();
    finish_limit(up, &need, sr)
}

#[cfg(test)]
fn limit(x: &[Vec<f64>; 2], sr: usize, t: &Target) -> [Vec<f64>; 2] {
    match ceiling_for(t, x[0].len() * 4, sr) {
        Some(ceiling) => limit_with_ceiling(x, sr, &ceiling),
        None => limit_with_env(x, sr, t),
    }
}

/// normalise, saturate, limit, and iterate the gain until the integrated loudness is within 0.08 LU of the target
pub fn master(mix: &[Vec<f64>; 2], sr: usize, t: &Target) -> Result<([Vec<f64>; 2], Loudness)> {
    let pk = mix.iter().flat_map(|c| c.iter()).fold(0.0f64, |a, v| a.max(v.abs())).max(1e-9);
    let mut x = [vec![], vec![]];
    for ch in 0..2 {
        x[ch] = mix[ch].iter().map(|v| ((v / pk * 0.5) * 1.3).tanh() / 1.3).collect();
    }
    let goal = t.lufs + 0.05;
    let ceiling = ceiling_for(t, x[0].len() * 4, sr);
    for _ in 0..6 {
        let y = match &ceiling {
            Some(c) => limit_with_ceiling(&x, sr, c),
            None => limit_with_env(&x, sr, t),
        };
        let l = measure(&y, sr)?;
        if (l.integrated_lufs - goal).abs() < 0.08 {
            return Ok((y, l));
        }
        let g = 10f64.powf((goal - l.integrated_lufs) / 20.0);
        for ch in x.iter_mut() {
            ch.iter_mut().for_each(|v| *v *= g);
        }
    }
    let y = match &ceiling {
        Some(c) => limit_with_ceiling(&x, sr, c),
        None => limit_with_env(&x, sr, t),
    };
    let l = measure(&y, sr)?;
    Ok((y, l))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ease;

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

    fn reference_ceiling(n: usize, sr: usize, t: &Target) -> Vec<f64> {
        let xs: Vec<f64> = t.ceiling_env.iter().map(|p| p.0).collect();
        let ys: Vec<f64> = t.ceiling_env.iter().map(|p| p.1).collect();
        (0..n)
            .map(|i| {
                let extra = if xs.is_empty() { 0.0 } else { ease::interp(i as f64 / (4.0 * sr as f64), &xs, &ys) };
                10f64.powf((t.ceiling_db + extra) / 20.0)
            })
            .collect()
    }

    fn reference_limit(x: &[Vec<f64>; 2], sr: usize, t: &Target) -> [Vec<f64>; 2] {
        let up = [dsp::upsample(&x[0], 4), dsp::upsample(&x[1], 4)];
        let n = up[0].len();
        let xs: Vec<f64> = t.ceiling_env.iter().map(|p| p.0).collect();
        let ys: Vec<f64> = t.ceiling_env.iter().map(|p| p.1).collect();
        let need: Vec<f64> = (0..n)
            .map(|i| {
                let extra = if xs.is_empty() { 0.0 } else { ease::interp(i as f64 / (4.0 * sr as f64), &xs, &ys) };
                let c = 10f64.powf((t.ceiling_db + extra) / 20.0);
                let pk = up[0][i].abs().max(up[1][i].abs()).max(1e-9);
                (c / pk).min(1.0)
            })
            .collect();
        let la = 4 * (0.025 * sr as f64) as usize;
        let g = smooth_gain(&need, la);
        let mut out = [vec![], vec![]];
        for ch in 0..2 {
            let y: Vec<f64> = up[ch].iter().zip(g.iter()).map(|(a, b)| a * b).collect();
            out[ch] = dsp::downsample(&y, 4);
        }
        out
    }

    fn reference_master(mix: &[Vec<f64>; 2], sr: usize, t: &Target) -> anyhow::Result<([Vec<f64>; 2], Loudness)> {
        let pk = mix.iter().flat_map(|c| c.iter()).fold(0.0f64, |a, v| a.max(v.abs())).max(1e-9);
        let mut x = [vec![], vec![]];
        for ch in 0..2 {
            x[ch] = mix[ch].iter().map(|v| ((v / pk * 0.5) * 1.3).tanh() / 1.3).collect();
        }
        let goal = t.lufs + 0.05;
        for _ in 0..6 {
            let y = reference_limit(&x, sr, t);
            let l = measure(&y, sr)?;
            if (l.integrated_lufs - goal).abs() < 0.08 {
                return Ok((y, l));
            }
            let g = 10f64.powf((goal - l.integrated_lufs) / 20.0);
            for ch in x.iter_mut() {
                ch.iter_mut().for_each(|v| *v *= g);
            }
        }
        let y = reference_limit(&x, sr, t);
        let l = measure(&y, sr)?;
        Ok((y, l))
    }

    fn reference_measure(x: &[Vec<f64>; 2], sr: usize) -> Result<Loudness> {
        let mut m = EbuR128::new(2, sr as u32, Mode::I | Mode::LRA | Mode::TRUE_PEAK)?;
        let inter: Vec<f64> = (0..x[0].len()).flat_map(|i| [x[0][i], x[1][i]]).collect();
        m.add_frames_f64(&inter)?;
        let tp = m.true_peak(0)?.max(m.true_peak(1)?);
        Ok(Loudness { integrated_lufs: m.loudness_global()?, true_peak_dbtp: 20.0 * tp.max(1e-12).log10(), lra: m.loudness_range()?, seconds: x[0].len() as f64 / sr as f64 })
    }

    fn assert_loudness_bits_eq(a: &Loudness, b: &Loudness, what: &str) {
        assert_eq!(a.integrated_lufs.to_bits(), b.integrated_lufs.to_bits(), "{what} integrated_lufs");
        assert_eq!(a.true_peak_dbtp.to_bits(), b.true_peak_dbtp.to_bits(), "{what} true_peak_dbtp");
        assert_eq!(a.lra.to_bits(), b.lra.to_bits(), "{what} lra");
        assert_eq!(a.seconds.to_bits(), b.seconds.to_bits(), "{what} seconds");
    }

    fn assert_bits_eq(a: &[f64], b: &[f64], what: &str) {
        assert_eq!(a.len(), b.len(), "{what} length");
        for (i, (x, y)) in a.iter().zip(b.iter()).enumerate() {
            assert_eq!(x.to_bits(), y.to_bits(), "{what}[{i}]: {x:?} vs {y:?}");
        }
    }

    fn targets() -> Vec<Target> {
        vec![
            Target { lufs: -14.0, ceiling_db: -1.0, ceiling_env: vec![] },
            Target { lufs: -14.0, ceiling_db: -1.5, ceiling_env: vec![(0.0, 0.0)] },
            Target { lufs: -14.0, ceiling_db: -1.0, ceiling_env: vec![(0.0, 0.0), (1.0, -6.0)] },
            Target { lufs: -14.0, ceiling_db: -1.0, ceiling_env: vec![(0.0, -3.0), (0.5, 0.0), (1.0, -6.0)] },
        ]
    }

    #[test]
    fn ceiling_curve_matches_reference_bits() {
        let sr = 8000;
        for n in [0, 1, 2, 3, 4, 7, 64, 1000] {
            for (k, t) in targets().iter().enumerate() {
                let c = ceiling_for(t, n, sr).expect("test size is below the curve bound");
                let a: Vec<f64> = (0..n).map(|i| c.at(i)).collect();
                let b = reference_ceiling(n, sr, t);
                assert_bits_eq(&a, &b, &format!("ceiling n={n} case={k}"));
            }
        }
    }

    #[test]
    fn ceiling_representation_is_bounded() {
        let sr = 8000;
        let empty = &targets()[0];
        assert!(matches!(ceiling_for(empty, MAX_CEILING_CURVE_SAMPLES + 1, sr), Some(Ceiling::Constant(_))));
        let env = &targets()[2];
        assert!(matches!(ceiling_for(env, 4, sr), Some(Ceiling::Curve(_))));
        assert!(ceiling_for(env, MAX_CEILING_CURVE_SAMPLES + 1, sr).is_none());
    }

    #[test]
    fn limit_matches_reference_bits() {
        let sr = 8000;
        for n in [0, 1, 2, 3, 4, 5, 8, 65, 500] {
            for kind in ["zeros", "impulse", "random"] {
                let x = [signal(kind, n, 31 + n as u64), signal("random", n, 71 + n as u64)];
                for (k, t) in targets().iter().enumerate() {
                    let a = reference_limit(&x, sr, t);
                    let b = limit(&x, sr, t);
                    assert_bits_eq(&a[0], &b[0], &format!("limit L {kind} n={n} case={k}"));
                    assert_bits_eq(&a[1], &b[1], &format!("limit R {kind} n={n} case={k}"));
                }
            }
        }
    }

    #[test]
    #[ignore]
    fn bench_limit_constant_ceiling() {
        use std::hint::black_box;
        use std::time::Instant;
        let sr = 48_000;
        let x = [signal("random", 48_000, 1), signal("random", 48_000, 2)];
        let t = Target { lufs: -14.0, ceiling_db: -1.0, ceiling_env: vec![] };
        let reps = 20;
        let ceiling = ceiling_for(&t, x[0].len() * 4, sr).unwrap();
        let t0 = Instant::now();
        let mut r = [vec![], vec![]];
        for _ in 0..reps {
            r = black_box(reference_limit(&x, sr, &t));
        }
        let tr = t0.elapsed();
        let t1 = Instant::now();
        let mut c = [vec![], vec![]];
        for _ in 0..reps {
            c = black_box(limit_with_ceiling(&x, sr, &ceiling));
        }
        let tc = t1.elapsed();
        assert_bits_eq(&r[0], &c[0], "bench limit L");
        assert_bits_eq(&r[1], &c[1], "bench limit R");
        eprintln!("limit constant ceiling: reference {tr:?}, precomputed {tc:?}");
    }

    #[test]
    #[ignore]
    fn bench_limit_ceiling_env() {
        use std::hint::black_box;
        use std::time::Instant;
        let sr = 48_000;
        let x = [signal("random", 48_000, 1), signal("random", 48_000, 2)];
        let t = Target { lufs: -14.0, ceiling_db: -1.0, ceiling_env: vec![(0.0, 0.0), (1.0, -6.0)] };
        let reps = 20;
        let ceiling = ceiling_for(&t, x[0].len() * 4, sr).unwrap();
        let t0 = Instant::now();
        let mut r = [vec![], vec![]];
        for _ in 0..reps {
            r = black_box(reference_limit(&x, sr, &t));
        }
        let tr = t0.elapsed();
        let t1 = Instant::now();
        let mut c = [vec![], vec![]];
        for _ in 0..reps {
            c = black_box(limit_with_ceiling(&x, sr, &ceiling));
        }
        let tc = t1.elapsed();
        assert_bits_eq(&r[0], &c[0], "bench env limit L");
        assert_bits_eq(&r[1], &c[1], "bench env limit R");
        eprintln!("limit ceiling env: reference {tr:?}, precomputed {tc:?}");
    }

    #[test]
    #[ignore]
    fn bench_master_candidate_vs_reference() {
        use std::time::Instant;
        let sr = 48_000;
        let mix = [signal("random", 6 * sr, 101), signal("random", 6 * sr, 202)];
        for (k, t) in targets().iter().enumerate() {
            let t0 = Instant::now();
            let (r_out, r_l) = reference_master(&mix, sr, t).unwrap();
            let tr = t0.elapsed();
            let t1 = Instant::now();
            let (c_out, c_l) = master(&mix, sr, t).unwrap();
            let tc = t1.elapsed();
            assert_bits_eq(&r_out[0], &c_out[0], &format!("bench master L case={k}"));
            assert_bits_eq(&r_out[1], &c_out[1], &format!("bench master R case={k}"));
            assert_eq!(r_l.integrated_lufs.to_bits(), c_l.integrated_lufs.to_bits(), "master loudness case={k}");
            assert_eq!(r_l.true_peak_dbtp.to_bits(), c_l.true_peak_dbtp.to_bits(), "master true peak case={k}");
            assert_eq!(r_l.lra.to_bits(), c_l.lra.to_bits(), "master lra case={k}");
            assert_eq!(r_l.seconds.to_bits(), c_l.seconds.to_bits(), "master seconds case={k}");
            eprintln!("master 6s case={k}: reference {tr:?}, candidate {tc:?}");
        }
    }

    fn sinusoidal_stereo(n: usize, sr: usize) -> [Vec<f64>; 2] {
        let l: Vec<f64> = (0..n).map(|i| (2.0 * std::f64::consts::PI * 440.0 * i as f64 / sr as f64).sin()).collect();
        let r: Vec<f64> = (0..n).map(|i| (2.0 * std::f64::consts::PI * 660.0 * i as f64 / sr as f64).sin()).collect();
        [l, r]
    }

    #[test]
    fn measure_matches_reference_bits_288000() {
        let sr = 48_000;
        let n = 288_000;
        let x = sinusoidal_stereo(n, sr);
        let a = measure(&x, sr).unwrap();
        let b = reference_measure(&x, sr).unwrap();
        assert_loudness_bits_eq(&a, &b, "measure 288000");
    }

    #[test]
    fn measure_errors_match_reference_for_shorts() {
        let sr = 48_000;
        for n in 0..64usize {
            let x = sinusoidal_stereo(n, sr);
            let a = measure(&x, sr);
            let b = reference_measure(&x, sr);
            match (a, b) {
                (Ok(la), Ok(lb)) => assert_loudness_bits_eq(&la, &lb, &format!("measure short n={n}")),
                (Err(ea), Err(eb)) => assert_eq!(format!("{ea:?}"), format!("{eb:?}"), "measure short n={n} error mismatch"),
                _ => panic!("measure short n={n}: Ok/Err mismatch"),
            }
        }
    }

    #[test]
    #[ignore]
    fn bench_measure_median_5_6s() {
        use std::time::Instant;
        let sr = 48_000;
        let n = 6 * sr;
    let x = sinusoidal_stereo(n, sr);
    let mut dr: Vec<u128> = Vec::with_capacity(5);
    let mut dm: Vec<u128> = Vec::with_capacity(5);
    for _ in 0..5 {
        let t0 = Instant::now();
        let _ = reference_measure(&x, sr).unwrap();
        dr.push(t0.elapsed().as_nanos());
        let t0 = Instant::now();
        let _ = measure(&x, sr).unwrap();
        dm.push(t0.elapsed().as_nanos());
    }
    dr.sort();
    dm.sort();
    eprintln!("reference_measure 6s median of 5: {} us", dr[2] / 1000);
    eprintln!("measure 6s median of 5: {} us", dm[2] / 1000);
}

    fn noise_stereo(n: usize, sr: usize) -> [Vec<f64>; 2] {
        [
            signal("random", n, 0xA5A5_5A5A ^ sr as u64),
            signal("random", n, 0x5A5A_A5A5 ^ (sr as u64).wrapping_mul(31)),
        ]
    }

    #[test]
    fn measure_matches_reference_bits_noise() {
        for &sr in &[44100usize, 48000usize] {
            for &n in &[4095usize, 4096usize, 4097usize, 6 * sr, 30 * sr] {
                let x = noise_stereo(n, sr);
                let a = measure(&x, sr).unwrap();
                let b = reference_measure(&x, sr).unwrap();
                assert_loudness_bits_eq(&a, &b, &format!("noise sr={sr} n={n}"));
            }
        }
    }
}
