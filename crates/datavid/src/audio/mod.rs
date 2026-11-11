//! Procedural audio: a two-bus score (dry + reverb send), synthesised instruments, and loudness mastering.
//!
//! ```ignore
//! let mut sc = Score::new(48000, total_s, seed);
//! sc.add(1.2, &sc_instr::boom(...), 0.8, 0.0, 0.6);   // time, signal, gain, pan (-1..1), reverb send
//! let mix = sc.mixdown(2.6);                           // reverb, high-pass
//! let m = master::master(&mix, sr, &master::Target { lufs: -14.0, ceiling_db: -2.8, .. })?;
//! ```
pub mod dsp;
pub mod master;
pub mod synth;

use rand::{Rng, SeedableRng};
use rand_distr::{Distribution, StandardNormal};
use rand_pcg::Pcg64;

pub struct Score {
    pub sr: f64,
    pub n: usize,
    pub dry: [Vec<f64>; 2],
    pub wet: [Vec<f64>; 2],
    pub rng: Pcg64,
}

pub fn hz(m: f64) -> f64 {
    440.0 * 2f64.powf((m - 69.0) / 12.0)
}

impl Score {
    pub fn new(sr: usize, seconds: f64, seed: u64) -> Self {
        let n = (seconds * sr as f64) as usize;
        Self { sr: sr as f64, n, dry: [vec![0.0; n], vec![0.0; n]], wet: [vec![0.0; n], vec![0.0; n]], rng: Pcg64::seed_from_u64(seed) }
    }
    pub fn t(&self, i: usize) -> f64 {
        i as f64 / self.sr
    }
    pub fn noise(&mut self, n: usize) -> Vec<f64> {
        (0..n).map(|_| StandardNormal.sample(&mut self.rng)).collect()
    }
    pub fn uniform(&mut self, a: f64, b: f64) -> f64 {
        self.rng.gen_range(a..b)
    }
    pub fn random(&mut self) -> f64 {
        self.rng.r#gen::<f64>()
    }
    /// piecewise-linear envelope over the whole timeline
    pub fn env(&self, pts: &[(f64, f64)]) -> Vec<f64> {
        let xs: Vec<f64> = pts.iter().map(|p| p.0).collect();
        let ys: Vec<f64> = pts.iter().map(|p| p.1).collect();
        (0..self.n).map(|i| crate::ease::interp(i as f64 / self.sr, &xs, &ys)).collect()
    }
    /// place a mono signal at `x` seconds with gain, equal-power pan and a reverb send
    pub fn add(&mut self, x: f64, sig: &[f64], g: f64, pan: f64, wet: f64) {
        let l = ((pan + 1.0) * std::f64::consts::PI / 4.0).cos();
        let r = ((pan + 1.0) * std::f64::consts::PI / 4.0).sin();
        let a0 = (x * self.sr).round() as isize;
        for (ch, k) in [(0usize, l), (1, r)] {
            let gk = g * k * 1.414;
            for (i, s) in sig.iter().enumerate() {
                let j = a0 + i as isize;
                if j < 0 {
                    continue;
                }
                let j = j as usize;
                if j >= self.n {
                    break;
                }
                self.dry[ch][j] += gk * s;
                self.wet[ch][j] += gk * s * wet;
            }
        }
    }
    /// add a full-length bed (same signal on both channels, `roll` samples apart for width)
    pub fn add_bed(&mut self, sig: &[f64], dry: f64, wet: f64, roll: usize) {
        for ch in 0..2 {
            let off = ch * roll;
            for i in 0..self.n {
                let s = sig[(i + self.n - off) % self.n];
                self.dry[ch][i] += dry * s;
                self.wet[ch][i] += wet * sig[i];
            }
        }
    }
    /// a stereo reverb impulse (decaying low-passed noise, energy-normalised)
    pub fn impulse(&mut self, seconds: f64) -> [Vec<f64>; 2] {
        let n = (seconds * self.sr) as usize;
        let mut irs = [vec![], vec![]];
        for ir in irs.iter_mut() {
            let nz = self.noise(n);
            let mut v = dsp::filt(&nz, 2, dsp::Kind::Low(5000.0), self.sr);
            for (i, s) in v.iter_mut().enumerate() {
                let t = i as f64 / self.sr;
                *s *= (-t * 2.4).exp() * (t / 0.01).min(1.0);
            }
            let e = v.iter().map(|x| x * x).sum::<f64>().sqrt();
            v.iter_mut().for_each(|x| *x /= e);
            *ir = v;
        }
        irs
    }
    /// dry + 0.55 x reverb(wet), high-passed at 25 Hz
    pub fn mixdown(&self, ir: &[Vec<f64>; 2]) -> [Vec<f64>; 2] {
        let mut out = [vec![], vec![]];
        for ch in 0..2 {
        let mut m: Vec<f64> = if self.wet[ch].iter().all(|v| *v == 0.0)
            && ir[ch].iter().all(|v| v.abs() <= 1.0)
        {
            self.dry[ch].iter().take(self.wet[ch].len()).map(|d| d + 0.0).collect()
        } else {
                let w = dsp::fftconvolve(&self.wet[ch], &ir[ch]);
                self.dry[ch].iter().zip(w.iter()).map(|(d, w)| d + 0.55 * w).collect()
            };
            dsp::sosfilt(&dsp::butter(2, dsp::Kind::High(25.0), self.sr), &mut m);
            out[ch] = m;
        }
        out
    }
}

/// write a 16-bit stereo WAV
pub fn write_wav(path: &std::path::Path, sr: usize, x: &[Vec<f64>; 2]) -> anyhow::Result<()> {
    let spec = hound::WavSpec { channels: 2, sample_rate: sr as u32, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p)?;
    }
    let mut w = hound::WavWriter::create(path, spec)?;
    for i in 0..x[0].len() {
        for ch in x.iter() {
            w.write_sample((ch[i].clamp(-1.0, 1.0) * 32767.0) as i16)?;
        }
    }
    w.finalize()?;
    Ok(())
}

pub fn read_wav(path: &std::path::Path) -> anyhow::Result<(usize, [Vec<f64>; 2])> {
    let mut r = hound::WavReader::open(path)?;
    let sp = r.spec();
    let ch = sp.channels as usize;
    let s: Vec<f64> = match sp.sample_format {
        hound::SampleFormat::Int => {
            let mx = (1i64 << (sp.bits_per_sample - 1)) as f64;
            r.samples::<i32>().map(|v| v.unwrap() as f64 / mx).collect()
        }
        hound::SampleFormat::Float => r.samples::<f32>().map(|v| v.unwrap() as f64).collect(),
    };
    let n = s.len() / ch;
    let l: Vec<f64> = (0..n).map(|i| s[i * ch]).collect();
    let rr: Vec<f64> = (0..n).map(|i| s[i * ch + (ch > 1) as usize]).collect();
    Ok((sp.sample_rate as usize, [l, rr]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bits_eq(a: &[f64], b: &[f64]) {
        assert_eq!(a.len(), b.len());
        for (i, (x, y)) in a.iter().zip(b.iter()).enumerate() {
            assert_eq!(x.to_bits(), y.to_bits(), "index {i}: {x:?} vs {y:?}");
        }
    }

    #[test]
    fn mixdown_zero_wet_matches_reference_bits() {
        for wet_val in [0.0f64, -0.0f64] {
            for ir in [
                vec![0.1f64, -0.2, 0.05],
                vec![],
                vec![f64::NAN, 1.0],
            ] {
                let n = 32usize;
                let sr = 48000usize;
                let dry = [vec![0.123456789f64; n], vec![-0.5f64; n]];
                let wet = [vec![wet_val; n], vec![wet_val; n]];
                let sc = Score { sr: sr as f64, n, dry: dry.clone(), wet, rng: Pcg64::seed_from_u64(1) };
                let irs = [ir.clone(), ir.clone()];
                let got = sc.mixdown(&irs);
                // reference (pre-fast-path) implementation
                let mut want = [vec![], vec![]];
                for ch in 0..2 {
                    let w = dsp::fftconvolve(&sc.wet[ch], &irs[ch]);
                    let mut m: Vec<f64> = sc.dry[ch].iter().zip(w.iter()).map(|(d, w)| d + 0.55 * w).collect();
                    dsp::sosfilt(&dsp::butter(2, dsp::Kind::High(25.0), sc.sr), &mut m);
                    want[ch] = m;
                }
                for ch in 0..2 {
                    bits_eq(&got[ch], &want[ch]);
                }
            }
        }
    }

    #[test]
    fn mixdown_nonzero_wet_matches_reference_bits() {
        let n = 32usize;
        let sr = 48000usize;
        let dry = [vec![0.123456789f64; n], vec![-0.5f64; n]];
        let wet = [vec![0.25f64; n], vec![-0.1f64; n]];
        let sc = Score { sr: sr as f64, n, dry: dry.clone(), wet, rng: Pcg64::seed_from_u64(1) };
        let ir = [vec![0.1f64, -0.2, 0.05], vec![0.2, 0.0, -0.1]];
        let got = sc.mixdown(&ir);
        let mut want = [vec![], vec![]];
        for ch in 0..2 {
            let w = dsp::fftconvolve(&sc.wet[ch], &ir[ch]);
            let mut m: Vec<f64> = sc.dry[ch].iter().zip(w.iter()).map(|(d, w)| d + 0.55 * w).collect();
            dsp::sosfilt(&dsp::butter(2, dsp::Kind::High(25.0), sc.sr), &mut m);
            want[ch] = m;
        }
        for ch in 0..2 {
            bits_eq(&got[ch], &want[ch]);
        }
    }

    #[test]
    fn mixdown_zero_wet_edge_cases() {
        // dry with negative zero and positive zero
        for dry_val in [-0.0f64, 0.0f64] {
            let n = 16usize;
            let sr = 48000usize;
            let dry = [vec![dry_val; n], vec![dry_val; n]];
            let wet = [vec![0.0f64; n], vec![0.0f64; n]];
            let sc = Score { sr: sr as f64, n, dry: dry.clone(), wet, rng: Pcg64::seed_from_u64(1) };
            let ir = [vec![0.25f64, -0.1, 0.0], vec![0.0, 0.5, -0.25]];
            let got = sc.mixdown(&ir);
            let mut want = [vec![], vec![]];
            for ch in 0..2 {
                let w = dsp::fftconvolve(&sc.wet[ch], &ir[ch]);
                let mut m: Vec<f64> = sc.dry[ch].iter().zip(w.iter()).map(|(d, w)| d + 0.55 * w).collect();
                dsp::sosfilt(&dsp::butter(2, dsp::Kind::High(25.0), sc.sr), &mut m);
                want[ch] = m;
            }
            for ch in 0..2 {
                bits_eq(&got[ch], &want[ch]);
            }
        }
        // wet length different and empty
        for (dry_n, wet_n) in [(16usize, 8usize), (8usize, 16usize), (16usize, 0usize)] {
            let sr = 48000usize;
            let dry = [vec![0.123456789f64; dry_n], vec![-0.5f64; dry_n]];
            let wet = [vec![0.0f64; wet_n], vec![0.0f64; wet_n]];
            let sc = Score { sr: sr as f64, n: dry_n, dry: dry.clone(), wet, rng: Pcg64::seed_from_u64(1) };
            let ir = [vec![0.1f64, -0.2, 0.05], vec![0.2, 0.0, -0.1]];
            let got = sc.mixdown(&ir);
            let mut want = [vec![], vec![]];
            for ch in 0..2 {
                let w = dsp::fftconvolve(&sc.wet[ch], &ir[ch]);
                let mut m: Vec<f64> = sc.dry[ch].iter().zip(w.iter()).map(|(d, w)| d + 0.55 * w).collect();
                dsp::sosfilt(&dsp::butter(2, dsp::Kind::High(25.0), sc.sr), &mut m);
                want[ch] = m;
            }
            for ch in 0..2 {
                bits_eq(&got[ch], &want[ch]);
            }
        }
        // huge finite IR falls back to convolution path
        {
            let n = 16usize;
            let sr = 48000usize;
            let dry = [vec![0.1f64; n], vec![-0.2f64; n]];
            let wet = [vec![0.0f64; n], vec![0.0f64; n]];
            let sc = Score { sr: sr as f64, n, dry: dry.clone(), wet, rng: Pcg64::seed_from_u64(1) };
            let ir = [vec![f64::MAX, 1e308], vec![f64::MAX]];
            let got = sc.mixdown(&ir);
            let mut want = [vec![], vec![]];
            for ch in 0..2 {
                let w = dsp::fftconvolve(&sc.wet[ch], &ir[ch]);
                let mut m: Vec<f64> = sc.dry[ch].iter().zip(w.iter()).map(|(d, w)| d + 0.55 * w).collect();
                dsp::sosfilt(&dsp::butter(2, dsp::Kind::High(25.0), sc.sr), &mut m);
                want[ch] = m;
            }
            for ch in 0..2 {
                bits_eq(&got[ch], &want[ch]);
            }
        }
    }
}
