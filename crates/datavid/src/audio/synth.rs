//! Synthesised instruments (no samples): booms, taiko, a brass-like "braam", whooshes, swells, ticks, snare, hats, pads and
//! a vowel-ish choir. Each returns a mono signal; noise comes from the score's seeded generator, so a render is repeatable.
use super::{Score, dsp, dsp::Kind, hz};
use std::f64::consts::PI;

fn tt(sc: &Score, d: f64) -> Vec<f64> {
    (0..(d * sc.sr) as usize).map(|i| i as f64 / sc.sr).collect()
}
fn phase_of(sc: &Score, f: impl Iterator<Item = f64>) -> Vec<f64> {
    let mut acc = 0.0;
    f.map(|v| {
        acc += v;
        2.0 * PI * acc / sc.sr
    })
    .collect()
}

pub fn boom(sc: &mut Score, dur: f64, f0: f64, f1: f64, dec: f64, drive: f64) -> Vec<f64> {
    let t = tt(sc, dur);
    let ph = phase_of(sc, t.iter().map(|x| f1 + (f0 - f1) * (-x * 9.0).exp()));
    let n = sc.noise(t.len());
    let click = dsp::filt(&n, 2, Kind::Low(2500.0), sc.sr);
    t.iter().enumerate().map(|(i, x)| ((drive * (ph[i].sin() * (-x * dec).exp() + click[i] * (-x * 60.0).exp() * 0.5)).tanh()) / drive.tanh()).collect()
}

pub fn taiko(sc: &mut Score, g: f64, f0: f64, f1: f64) -> Vec<f64> {
    let t = tt(sc, 0.9);
    let ph = phase_of(sc, t.iter().map(|x| f1 + (f0 - f1) * (-x * 28.0).exp()));
    let n1 = sc.noise(t.len());
    let skin = dsp::filt(&n1, 2, Kind::Band(120.0, 900.0), sc.sr);
    let n2 = sc.noise(t.len());
    let slap = dsp::filt(&n2, 2, Kind::High(2000.0), sc.sr);
    t.iter().enumerate().map(|(i, x)| (1.5 * (ph[i].sin() * (-x * 6.5).exp() + skin[i] * (-x * 30.0).exp() * 0.9 + slap[i] * (-x * 90.0).exp() * 0.25)).tanh() * g).collect()
}
pub fn taiko_d(sc: &mut Score, g: f64) -> Vec<f64> {
    taiko(sc, g, 150.0, 58.0)
}

/// time-varying low-pass by blocks (each block filtered with 1200 samples of history), like the Python score
fn sweep_lp(sc: &Score, s: &[f64], cut: impl Fn(usize) -> f64, band: Option<f64>) -> Vec<f64> {
    let blk = if band.is_some() { 1200 } else { 2400 };
    let mut out = vec![0.0; s.len()];
    let mut a = 0;
    while a < s.len() {
        let b = (a + blk).min(s.len());
        let st = a.saturating_sub(1200);
        let f = cut(a);
        let kind = match band {
            Some(_) => Kind::Band(f * 0.6, (f * 1.6).min(20000.0)),
            None => Kind::Low(f),
        };
        let y = dsp::filt(&s[st..b], 2, kind, sc.sr);
        out[a..b].copy_from_slice(&y[y.len() - (b - a)..]);
        a = b;
    }
    out
}

pub fn braam(sc: &mut Score, root: f64, dur: f64) -> Vec<f64> {
    let t = tt(sc, dur);
    let mut s = vec![0.0; t.len()];
    for m in [root, root + 7.0, root + 12.0, root + 15.0, root + 19.0] {
        for det in [-0.12, 0.0, 0.11] {
            let f = hz(m + det);
            let g = if m == root { 0.9 } else { 0.6 };
            for (i, x) in t.iter().enumerate() {
                let ph = (f * x) % 1.0;
                s[i] += (2.0 * ph - 1.0) * g;
            }
        }
    }
    let cut = |i: usize| {
        let x = i as f64 / sc.sr;
        250.0 + 2600.0 * (-x * 1.4).exp() * (x / 0.15).min(1.0)
    };
    let f = sweep_lp(sc, &s, cut, None);
    let ph = phase_of(sc, t.iter().map(|x| 30.0 + 30.0 * (-x * 3.0).exp()));
    t.iter().enumerate().map(|(i, x)| ((f[i] * (x / 0.04).min(1.0) * (-x * 0.7).exp() * 0.35).tanh() * 1.2) + ph[i].sin() * (-x * 0.9).exp() * 0.9).collect()
}

pub fn whoosh(sc: &mut Score, dur: f64, f0: f64, f1: f64, rev: bool) -> Vec<f64> {
    let t = tt(sc, dur);
    let n = sc.noise(t.len());
    let sr = sc.sr;
    let out = sweep_lp(sc, &n, |i| f0 * (f1 / f0).powf((i as f64 / sr) / dur), Some(1.0));
    t.iter().enumerate().map(|(i, x)| out[i] * if !rev { (x / dur).powf(2.5) } else { (1.0 - x / dur).max(0.0).powf(1.5) * (x / 0.05).min(1.0) }).collect()
}

pub fn rev_swell(sc: &mut Score, dur: f64) -> Vec<f64> {
    let t = tt(sc, dur);
    let n = sc.noise(t.len());
    let h = dsp::filt(&n, 2, Kind::High(1500.0), sc.sr);
    t.iter().enumerate().map(|(i, x)| h[i] * (x / dur).powi(3)).collect()
}

pub fn tick(sc: &mut Score, hi: bool) -> Vec<f64> {
    let t = tt(sc, 0.05);
    let f = if hi { 3200.0 } else { 2400.0 };
    let n = sc.noise(t.len());
    let b = dsp::filt(&n, 2, Kind::Band(2500.0, 7000.0), sc.sr);
    t.iter().enumerate().map(|(i, x)| ((2.0 * PI * f * x).sin() * 0.6 + b[i] * 0.6) * (-x * 140.0).exp()).collect()
}

pub fn snare(sc: &mut Score, g: f64) -> Vec<f64> {
    let t = tt(sc, 0.35);
    let n = sc.noise(t.len());
    let b = dsp::filt(&n, 2, Kind::Band(900.0, 8000.0), sc.sr);
    t.iter().enumerate().map(|(i, x)| (b[i] * (-x * 16.0).exp() + (2.0 * PI * 190.0 * x).sin() * (-x * 25.0).exp() * 0.6) * g).collect()
}

pub fn hat(sc: &mut Score) -> Vec<f64> {
    let t = tt(sc, 0.06);
    let n = sc.noise(t.len());
    let h = dsp::filt(&n, 2, Kind::High(7000.0), sc.sr);
    t.iter().enumerate().map(|(i, x)| h[i] * (-x * 80.0).exp()).collect()
}

/// a sine "pluck" (decaying sine at midi note `m`)
pub fn ping(sc: &Score, m: f64, dur: f64, dec: f64, amp: f64) -> Vec<f64> {
    tt(sc, dur).iter().map(|x| (2.0 * PI * hz(m) * x).sin() * (-x * dec).exp() * amp).collect()
}

pub fn pad_voice(sc: &mut Score, midis: &[f64], dur: f64, bright: f64, att: f64, rel: f64) -> Vec<f64> {
    let t = tt(sc, dur + rel);
    let mut s = vec![0.0; t.len()];
    for &m in midis {
        for det in [-0.07, 0.0, 0.08] {
            let f = hz(m + det);
            let r0 = sc.random() * 6.0;
            let ph: Vec<f64> = {
                let mut acc = 0.0;
                t.iter().map(|x| {
                    acc += f * (1.0 + 0.003 * (2.0 * PI * 5.1 * x + r0).sin());
                    acc / sc.sr
                })
                .collect()
            };
            for k in 1..7 {
                let w = 1.0 / (k as f64).powf(1.6 - 0.5 * bright);
                for i in 0..t.len() {
                    s[i] += (2.0 * PI * k as f64 * ph[i]).sin() * w;
                }
            }
        }
    }
    let norm = (midis.len() * 3) as f64;
    t.iter().enumerate().map(|(i, x)| s[i] * (x / att).min(1.0) * ((dur + rel - x) / rel).clamp(0.0, 1.0) / norm).collect()
}

pub fn choir(sc: &mut Score, midis: &[f64], dur: f64, att: f64, rel: f64) -> Vec<f64> {
    let t = tt(sc, dur + rel);
    let mut s = vec![0.0; t.len()];
    for &m in midis {
        let f0 = hz(m);
        for det in [-0.1f64, 0.1] {
            let r0 = sc.random() * 6.0;
            let fd = f0 * 2f64.powf(det / 12.0);
            let ph: Vec<f64> = {
                let mut acc = 0.0;
                t.iter().map(|x| {
                    acc += fd * (1.0 + 0.006 * (2.0 * PI * 5.5 * x + r0).sin());
                    acc / sc.sr
                })
                .collect()
            };
            for k in 1..14 {
                let fk = k as f64 * f0;
                let w = (-((fk - 700.0) / 260.0).powi(2)).exp() + 0.6 * (-((fk - 1150.0) / 300.0).powi(2)).exp() + 0.15 / k as f64;
                for i in 0..t.len() {
                    s[i] += w * (2.0 * PI * k as f64 * ph[i]).sin();
                }
            }
        }
    }
    let norm = (midis.len() * 2) as f64;
    t.iter().enumerate().map(|(i, x)| s[i] * (x / att).min(1.0) * ((dur + rel - x) / rel).clamp(0.0, 1.0) / norm).collect()
}
