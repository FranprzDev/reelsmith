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
    let inter: Vec<f64> = (0..x[0].len()).flat_map(|i| [x[0][i], x[1][i]]).collect();
    m.add_frames_f64(&inter)?;
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

fn limit(x: &[Vec<f64>; 2], sr: usize, t: &Target) -> [Vec<f64>; 2] {
    let up = [dsp::upsample(&x[0], 4), dsp::upsample(&x[1], 4)];
    let n = up[0].len();
    let xs: Vec<f64> = t.ceiling_env.iter().map(|p| p.0).collect();
    let ys: Vec<f64> = t.ceiling_env.iter().map(|p| p.1).collect();
    let need: Vec<f64> = (0..n)
        .map(|i| {
            let extra = if xs.is_empty() { 0.0 } else { crate::ease::interp(i as f64 / (4.0 * sr as f64), &xs, &ys) };
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

/// normalise, saturate, limit, and iterate the gain until the integrated loudness is within 0.08 LU of the target
pub fn master(mix: &[Vec<f64>; 2], sr: usize, t: &Target) -> Result<([Vec<f64>; 2], Loudness)> {
    let pk = mix.iter().flat_map(|c| c.iter()).fold(0.0f64, |a, v| a.max(v.abs())).max(1e-9);
    let mut x = [vec![], vec![]];
    for ch in 0..2 {
        x[ch] = mix[ch].iter().map(|v| ((v / pk * 0.5) * 1.3).tanh() / 1.3).collect();
    }
    let goal = t.lufs + 0.05;
    for _ in 0..6 {
        let y = limit(&x, sr, t);
        let l = measure(&y, sr)?;
        if (l.integrated_lufs - goal).abs() < 0.08 {
            return Ok((y, l));
        }
        let g = 10f64.powf((goal - l.integrated_lufs) / 20.0);
        for ch in x.iter_mut() {
            ch.iter_mut().for_each(|v| *v *= g);
        }
    }
    let y = limit(&x, sr, t);
    let l = measure(&y, sr)?;
    Ok((y, l))
}
