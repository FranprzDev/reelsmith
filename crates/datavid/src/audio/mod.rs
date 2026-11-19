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
    let cap = r.duration() as usize;
    let mut l: Vec<f64> = Vec::with_capacity(cap);
    let mut rr: Vec<f64> = Vec::with_capacity(cap);
    match sp.sample_format {
        hound::SampleFormat::Int => {
            let mx = (1i64 << (sp.bits_per_sample - 1)) as f64;
            collect_frames(r.samples::<i32>(), ch, |v| v as f64 / mx, &mut l, &mut rr)?;
        }
        hound::SampleFormat::Float => {
            collect_frames(r.samples::<f32>(), ch, |v| v as f64, &mut l, &mut rr)?;
        }
    }
    Ok((sp.sample_rate as usize, [l, rr]))
}

fn collect_frames<S: Copy>(
    samples: impl Iterator<Item = std::result::Result<S, hound::Error>>,
    ch: usize,
    convert: impl Fn(S) -> f64,
    l: &mut Vec<f64>,
    rr: &mut Vec<f64>,
) -> anyhow::Result<()> {
    let mut frame = [0.0f64; 2];
    let mut filled = 0usize;
    for s in samples {
        let v = convert(s?);
        match filled {
            0 => frame[0] = v,
            1 => frame[1] = v,
            _ => {}
        }
        filled += 1;
        if filled == ch {
            l.push(frame[0]);
            rr.push(if ch > 1 { frame[1] } else { frame[0] });
            filled = 0;
        }
    }
    Ok(())
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

    #[test]
    fn mixdown_signedzero_dry_negativezero_wet_matches_reference_bits() {
        for dry_val in [-0.0f64, 0.0f64] {
            let n = 16usize;
            let sr = 48000usize;
            let dry = [vec![dry_val; n], vec![dry_val; n]];
            let wet = [vec![-0.0f64; n], vec![-0.0f64; n]];
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
    }

    #[test]
    #[ignore]
    fn bench_zero_wet_mixdown_vs_old_fft_reference() {
        use std::hint::black_box;
        use std::time::{Duration, Instant};

        let sr = 48000usize;
        let seconds = 6.0;
        let n = (seconds * sr as f64) as usize;
        let mut sc = Score::new(sr, seconds, 7);
        for ch in 0..2 {
            for i in 0..n {
                let t = i as f64 / sr as f64;
                sc.dry[ch][i] = (2.0 * std::f64::consts::PI * 220.0 * t).sin();
            }
            sc.wet[ch] = vec![0.0; n];
        }
        let ir = sc.impulse(2.2);

        let mut want = [vec![], vec![]];
        for ch in 0..2 {
            let w = dsp::fftconvolve(&sc.wet[ch], &ir[ch]);
            let mut m: Vec<f64> = sc.dry[ch].iter().zip(w.iter()).map(|(d, w)| d + 0.55 * w).collect();
            dsp::sosfilt(&dsp::butter(2, dsp::Kind::High(25.0), sc.sr), &mut m);
            want[ch] = m;
        }
        let got = sc.mixdown(&ir);
        for ch in 0..2 {
            bits_eq(&got[ch], &want[ch]);
        }

        let mut old_times = [Duration::ZERO; 5];
        for slot in &mut old_times {
            let t0 = Instant::now();
            let mut r = [vec![], vec![]];
            for ch in 0..2 {
                let w = dsp::fftconvolve(&sc.wet[ch], &ir[ch]);
                let mut m: Vec<f64> = sc.dry[ch].iter().zip(w.iter()).map(|(d, w)| d + 0.55 * w).collect();
                dsp::sosfilt(&dsp::butter(2, dsp::Kind::High(25.0), sc.sr), &mut m);
                r[ch] = m;
            }
            black_box(&r);
            *slot = t0.elapsed();
        }

        let mut new_times = [Duration::ZERO; 5];
        for slot in &mut new_times {
            let t0 = Instant::now();
            let r = black_box(sc.mixdown(&ir));
            *slot = t0.elapsed();
            black_box(&r);
        }

        old_times.sort();
        new_times.sort();
        eprintln!("mixdown zero wet 6s: old_fft_median {:?}, fast_median {:?}", old_times[2], new_times[2]);

        let mut imp_times = [Duration::ZERO; 5];
        for slot in &mut imp_times {
            let mut sc2 = Score::new(sr, seconds, 7);
            let t0 = Instant::now();
            let r = black_box(sc2.impulse(2.2));
            *slot = t0.elapsed();
            black_box(&r);
        }
        imp_times.sort();
        eprintln!("impulse 2.2 median {:?}", imp_times[2]);
    }
}

#[cfg(test)]
mod wav_stream_tests {
    use std::path::{Path, PathBuf};

    // Exact legacy reference. Copied verbatim from the original implementation
    // so we can compare new production output bit-for-bit against the old stream.
    pub fn read_wav(path: &Path) -> anyhow::Result<(usize, [Vec<f64>; 2])> {
        let mut r = hound::WavReader::open(path)?;
        let sp = r.spec();
        let ch = sp.channels as usize;
        let s: Vec<f64> = match sp.sample_format {
            hound::SampleFormat::Int => {
                let mx = (1i64 << (sp.bits_per_sample - 1)) as f64;
                r.samples::<i32>()
                    .map(|v| v.unwrap() as f64 / mx)
                    .collect()
            }
            hound::SampleFormat::Float => r.samples::<f32>().map(|v| v.unwrap() as f64).collect(),
        };
        let n = s.len() / ch;
        let l = (0..n).map(|i| s[i * ch]).collect();
        let rr = (0..n).map(|i| s[i * ch + (ch > 1) as usize]).collect();
        Ok((sp.sample_rate as usize, [l, rr]))
    }

    type WavSample<T> = std::result::Result<T, hound::Error>;

    fn temp_path(tag: &str) -> PathBuf {
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "wav_stream_tests_{}_{}_{}.wav",
            tag,
            std::process::id(),
            n
        ))
    }

    fn write_int_wav(path: &Path, channels: u16, rate: u32, bits: u16, samples: &[i32]) {
        let spec = hound::WavSpec {
            channels,
            sample_rate: rate,
            bits_per_sample: bits,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(path, spec).unwrap();
        for &s in samples {
            w.write_sample(s).unwrap();
        }
        w.finalize().unwrap();
    }

    fn write_float_wav(path: &Path, channels: u16, rate: u32, samples: &[f32]) {
        let spec = hound::WavSpec {
            channels,
            sample_rate: rate,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        };
        let mut w = hound::WavWriter::create(path, spec).unwrap();
        for &s in samples {
            w.write_sample(s).unwrap();
        }
        w.finalize().unwrap();
    }

    fn assert_streams_bit_eq(
        new: &(usize, [Vec<f64>; 2]),
        old: &(usize, [Vec<f64>; 2]),
        tag: &str,
    ) {
        assert_eq!(new.0, old.0, "[{tag}] sample rate mismatch");
        assert_eq!(
            new.1[0].len(),
            old.1[0].len(),
            "[{tag}] left channel length mismatch"
        );
        assert_eq!(
            new.1[1].len(),
            old.1[1].len(),
            "[{tag}] right channel length mismatch"
        );
        for (i, (a, b)) in new.1[0].iter().zip(old.1[0].iter()).enumerate() {
            assert_eq!(
                a.to_bits(),
                b.to_bits(),
                "[{tag}] left sample {i} bits differ: new={:?}({:016x}) old={:?}({:016x})",
                a,
                a.to_bits(),
                b,
                b.to_bits()
            );
        }
        for (i, (a, b)) in new.1[1].iter().zip(old.1[1].iter()).enumerate() {
            assert_eq!(
                a.to_bits(),
                b.to_bits(),
                "[{tag}] right sample {i} bits differ: new={:?}({:016x}) old={:?}({:016x})",
                a,
                a.to_bits(),
                b,
                b.to_bits()
            );
        }
    }

    fn run_and_compare(path: &Path, tag: &str) {
        let new = super::read_wav(path).expect("new read_wav failed");
        let old = read_wav(path).expect("legacy read_wav failed");
        assert_streams_bit_eq(&new, &old, tag);
    }

    // ---------- PCM16 ----------

    #[test]
    fn pcm16_mono_exact_bits() {
        let p = temp_path("pcm16_mono");
        let samples: Vec<i32> = vec![
            i16::MIN as i32,
            -1,
            0,
            1,
            i16::MAX as i32,
            12345,
            -12345,
            0,
            7,
            -7,
        ];
        write_int_wav(&p, 1, 8000, 16, &samples);
        run_and_compare(&p, "pcm16_mono");
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn pcm16_stereo_exact_bits() {
        let p = temp_path("pcm16_stereo");
        // 10 samples => 5 full 2ch frames (whole-frame count required by Hound).
        let samples: Vec<i32> = vec![
            i16::MIN as i32,
            i16::MAX as i32,
            0,
            100,
            -100,
            1,
            -1,
            20000,
            -20000,
            i16::MIN as i32,
        ];
        write_int_wav(&p, 2, 44100, 16, &samples);
        run_and_compare(&p, "pcm16_stereo");
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn pcm16_three_channel_exact_bits() {
        let p = temp_path("pcm16_3ch");
        let samples: Vec<i32> = vec![
            1, 2, 3, -100, 100, 200, i16::MAX as i32, i16::MIN as i32, 0, 7, -7, 11,
        ];
        write_int_wav(&p, 3, 22050, 16, &samples);
        run_and_compare(&p, "pcm16_3ch");
        std::fs::remove_file(&p).ok();
    }

    // ---------- PCM24 ----------

    const I24_MIN: i32 = -8_388_608;
    const I24_MAX: i32 = 8_388_607;

    #[test]
    fn pcm24_mono_exact_bits() {
        let p = temp_path("pcm24_mono");
        let samples: Vec<i32> = vec![I24_MIN, -1, 0, 1, I24_MAX, 1_234_567, -1_234_567, 0];
        write_int_wav(&p, 1, 8000, 24, &samples);
        run_and_compare(&p, "pcm24_mono");
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn pcm24_stereo_exact_bits() {
        let p = temp_path("pcm24_stereo");
        let samples: Vec<i32> = vec![
            I24_MIN,
            I24_MAX,
            0,
            12_345,
            -12_345,
            1,
            -1,
            4_000_000,
            -4_000_000,
            I24_MAX,
        ];
        write_int_wav(&p, 2, 48000, 24, &samples);
        run_and_compare(&p, "pcm24_stereo");
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn pcm24_three_channel_exact_bits() {
        let p = temp_path("pcm24_3ch");
        let samples: Vec<i32> = vec![
            1, 2, 3, -1_000_000, 1_000_000, 2_000_000, I24_MAX, I24_MIN, 0, 42, -42, 7,
        ];
        write_int_wav(&p, 3, 96000, 24, &samples);
        run_and_compare(&p, "pcm24_3ch");
        std::fs::remove_file(&p).ok();
    }

    // ---------- PCM32 ----------

    #[test]
    fn pcm32_mono_exact_bits() {
        let p = temp_path("pcm32_mono");
        let samples: Vec<i32> = vec![
            i32::MIN,
            -1,
            0,
            1,
            i32::MAX,
            123_456_789,
            -123_456_789,
        ];
        write_int_wav(&p, 1, 8000, 32, &samples);
        run_and_compare(&p, "pcm32_mono");
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn pcm32_stereo_exact_bits() {
        let p = temp_path("pcm32_stereo");
        let samples: Vec<i32> = vec![
            i32::MIN,
            i32::MAX,
            0,
            i32::MIN / 2,
            i32::MAX / 2,
            1,
            -1,
            2_000_000_000,
            -2_000_000_000,
            17,
        ];
        write_int_wav(&p, 2, 44100, 32, &samples);
        run_and_compare(&p, "pcm32_stereo");
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn pcm32_three_channel_exact_bits() {
        let p = temp_path("pcm32_3ch");
        let samples: Vec<i32> = vec![
            1, 2, 3, -1_000_000_000, 1_000_000_000, 2_000_000_000, i32::MAX, i32::MIN, 0, 42,
            -42, 7,
        ];
        write_int_wav(&p, 3, 88200, 32, &samples);
        run_and_compare(&p, "pcm32_3ch");
        std::fs::remove_file(&p).ok();
    }

    // ---------- Float32 (NaN / Inf / signed zero) ----------

    #[test]
    fn float32_mono_exact_bits() {
        let p = temp_path("f32_mono");
        let samples: Vec<f32> = vec![
            f32::NAN,
            f32::INFINITY,
            f32::NEG_INFINITY,
            0.0,
            -0.0,
            1.0,
            -1.0,
            0.5,
            -0.5,
            f32::MIN_POSITIVE,
            -f32::MIN_POSITIVE,
            0.0,
        ];
        write_float_wav(&p, 1, 44100, &samples);
        run_and_compare(&p, "f32_mono");
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn float32_stereo_exact_bits() {
        let p = temp_path("f32_stereo");
        // 16 samples => 8 full 2ch frames (whole-frame count required by Hound).
        let samples: Vec<f32> = vec![
            f32::NAN,
            0.0,
            f32::INFINITY,
            -0.0,
            f32::NEG_INFINITY,
            1.0,
            0.0,
            -1.0,
            -0.0,
            0.5,
            0.25,
            -0.25,
            7.5,
            8.5,
            9.5,
            f32::NEG_INFINITY,
        ];
        write_float_wav(&p, 2, 48000, &samples);
        run_and_compare(&p, "f32_stereo");
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn float32_three_channel_exact_bits() {
        let p = temp_path("f32_3ch");
        let samples: Vec<f32> = vec![
            f32::NAN,
            0.0,
            -0.0,
            f32::INFINITY,
            f32::NEG_INFINITY,
            0.0,
            f32::MIN_POSITIVE,
            -f32::MIN_POSITIVE,
            1e-30,
            -1e-30,
            0.0,
            -0.0,
        ];
        write_float_wav(&p, 3, 96000, &samples);
        run_and_compare(&p, "f32_3ch");
        std::fs::remove_file(&p).ok();
    }

    // ---------- collect_frames: trailing partial frames via mock iterators ----------

    #[test]
    fn collect_frames_empty_iterator() {
        let samples: Vec<WavSample<i32>> = vec![];
        let mut l: Vec<f64> = Vec::new();
        let mut rr: Vec<f64> = Vec::new();
        super::collect_frames(samples.into_iter(), 2, |v| v as f64, &mut l, &mut rr).unwrap();
        assert!(l.is_empty());
        assert!(rr.is_empty());
    }

    #[test]
    fn collect_frames_partial_frame_mono() {
        // ch == 1: every sample is a complete frame, nothing is trailing.
        let samples: Vec<WavSample<i32>> = vec![Ok(100), Ok(200), Ok(300)];
        let mut l: Vec<f64> = Vec::new();
        let mut rr: Vec<f64> = Vec::new();
        super::collect_frames(samples.into_iter(), 1, |v| v as f64, &mut l, &mut rr).unwrap();
        assert_eq!(l, vec![100.0, 200.0, 300.0]);
        assert_eq!(rr, vec![100.0, 200.0, 300.0]);
    }

    #[test]
    fn collect_frames_trailing_partial_frame_stereo_is_dropped() {
        // 5 samples, ch = 2 => 2 full frames + 1 trailing sample dropped.
        let samples: Vec<WavSample<i32>> = vec![Ok(1), Ok(2), Ok(3), Ok(4), Ok(5)];
        let mut l: Vec<f64> = Vec::new();
        let mut rr: Vec<f64> = Vec::new();
        super::collect_frames(samples.into_iter(), 2, |v| v as f64, &mut l, &mut rr).unwrap();
        assert_eq!(l, vec![1.0, 3.0]);
        assert_eq!(rr, vec![2.0, 4.0]);
    }

    #[test]
    fn collect_frames_trailing_partial_frame_three_channel_is_dropped() {
        // 4 samples, ch = 3 => 1 full frame + 1 trailing sample dropped.
        let samples: Vec<WavSample<i32>> = vec![Ok(1), Ok(2), Ok(3), Ok(4)];
        let mut l: Vec<f64> = Vec::new();
        let mut rr: Vec<f64> = Vec::new();
        super::collect_frames(samples.into_iter(), 3, |v| v as f64, &mut l, &mut rr).unwrap();
        assert_eq!(l, vec![1.0]);
        assert_eq!(rr, vec![2.0]);
    }

    #[test]
    fn collect_frames_exact_multiple_of_channels() {
        let samples: Vec<WavSample<i32>> = vec![Ok(1), Ok(2), Ok(3), Ok(4), Ok(5), Ok(6)];
        let mut l: Vec<f64> = Vec::new();
        let mut rr: Vec<f64> = Vec::new();
        super::collect_frames(samples.into_iter(), 3, |v| v as f64, &mut l, &mut rr).unwrap();
        assert_eq!(l, vec![1.0, 4.0]);
        assert_eq!(rr, vec![2.0, 5.0]);
    }

    #[test]
    fn collect_frames_convert_applied() {
        let samples: Vec<WavSample<i32>> = vec![Ok(8), Ok(16), Ok(24), Ok(32)];
        let mut l: Vec<f64> = Vec::new();
        let mut rr: Vec<f64> = Vec::new();
        super::collect_frames(samples.into_iter(), 2, |v| v as f64 / 8.0, &mut l, &mut rr)
            .unwrap();
        assert_eq!(l, vec![1.0, 3.0]);
        assert_eq!(rr, vec![2.0, 4.0]);
    }

    // ---------- Benchmark (ignored by default) ----------

    #[test]
    #[ignore]
    fn bench_read_wav() {
        let path = std::env::var("RS_WAV_PATH").expect("RS_WAV_PATH env var must point to a .wav file");
        let p = PathBuf::from(path);
        let use_old = std::env::var("RS_WAV_OLD").is_ok();

        let start = std::time::Instant::now();
        let (rate, data) = if use_old {
            std::hint::black_box(read_wav(&p).expect("legacy read_wav failed"))
        } else {
            std::hint::black_box(super::read_wav(&p).expect("new read_wav failed"))
        };
        let elapsed = start.elapsed();

        println!(
            "bench_read_wav: old={} path={:?} sample_rate={} frames_l={} frames_r={} elapsed={:?}",
            use_old,
            p,
            rate,
            data[0].len(),
            data[1].len(),
            elapsed
        );
    }
}
