//! Sound: the music bed (procedural from datavid's synth voices, an external file, or none), a whoosh + hit on every
//! cut flash, the configured cues (braam / hit / whoosh), each motion overlay's own sound effect at its slot, an
//! optional voice-over with the music ducked under it, mastered in Rust to the loudness target (−14 LUFS) with a
//! true-peak ceiling. Written to `build/audio.wav` and skipped when nothing that shapes it changed.
use crate::config::{CueKind, MusicKind};
use crate::plan::{Plan, key_of};
use crate::progress::Events;
use anyhow::{Context, Result};
use datavid::audio::{Score, dsp, master, synth, write_wav};
use datavid::media;
use serde_json::json;
use std::path::Path;

pub const SR: usize = 48000;

/// a bright saw-ish pluck: a few decaying harmonics
fn pluck(sc: &Score, m: f64, dur: f64, dec: f64, amp: f64) -> Vec<f64> {
    let mut s = synth::ping(sc, m, dur, dec, amp);
    for (h, g) in [(12.0, 0.45), (19.02, 0.18), (24.0, 0.12)] {
        let p = synth::ping(sc, m + h, dur, dec * 1.6, amp * g);
        s.iter_mut().zip(p).for_each(|(a, b)| *a += b);
    }
    s
}

/// The procedural bed: a soft pulse under the hook, then four-on-the-floor kick, snare, hats, off-beat bass, an
/// arpeggio and a pad on the chord progression from `drop` to `outro`; the arpeggio and pad ring on two more bars.
fn procedural_bed(sc: &mut Score, p: &Plan) {
    let m = &p.music;
    let total = p.seconds;
    let beat = 60.0 / m.bpm;
    let bar = 4.0 * beat;
    let (drop, outro) = (p.music_drop, p.music_outro);
    let prog = &m.progression;
    let arp = [0usize, 1, 2, 1, 2, 0, 1, 2];
    let kick = synth::boom(sc, 0.42, 165.0, 48.0, 9.0, 2.4);
    let mut t = 0.0;
    let mut step = 0usize; // 16th-note steps
    while t < total {
        let b16 = step % 16;
        let bar_i = (t / bar).floor() as usize;
        let (root, triad) = prog[bar_i % prog.len()];
        let groove = t >= drop - 1e-6 && t < outro;
        let hook = t < drop;
        if groove {
            if b16 % 4 == 0 {
                sc.add(t, &kick, 0.95, 0.0, 0.05);
            }
            if b16 == 4 || b16 == 12 {
                let sn = synth::snare(sc, 0.55);
                sc.add(t, &sn, 0.8, 0.0, 0.35);
            }
            if b16 % 2 == 0 {
                let h = synth::hat(sc);
                sc.add(t, &h, if b16 % 4 == 2 { 0.5 } else { 0.25 }, 0.25, 0.1);
            }
            if b16 % 2 == 0 && b16 % 4 != 0 {
                // off-beat bass (the kick owns the down-beat)
                let bs = pluck(sc, root, beat * 0.5, 7.0, 0.55);
                let bs = dsp::filt(&bs, 2, dsp::Kind::Low(900.0), sc.sr);
                sc.add(t, &bs, 0.9, 0.0, 0.0);
            }
        }
        if hook && b16 % 8 == 0 {
            sc.add(t, &kick, 0.35, 0.0, 0.2);
        }
        if t < outro + bar * 2.0 && b16 % 2 == 0 {
            let mm = triad[arp[(step / 2) % 8]] + 12.0;
            let g = if groove { 0.16 } else { 0.11 };
            let pl = pluck(sc, mm, beat * 0.9, 9.0, 1.0);
            sc.add(t, &pl, g, if (step / 2) % 2 == 0 { -0.35 } else { 0.35 }, 0.45);
        }
        if b16 == 0 {
            let pad = synth::pad_voice(sc, &triad, bar, 0.35, 0.25, 0.6);
            sc.add(t, &pad, if groove { 0.10 } else { 0.16 }, 0.0, 0.6);
        }
        step += 1;
        t = step as f64 * beat / 4.0;
    }
}

fn add_cue(sc: &mut Score, kind: CueKind, t: f64, gain: f64) {
    match kind {
        CueKind::Braam => {
            let br = synth::braam(sc, 33.0, 1.6);
            sc.add(t, &br, gain, 0.0, 0.5);
        }
        CueKind::Hit => {
            let hit = synth::boom(sc, 0.9, 120.0, 38.0, 4.0, 3.0);
            sc.add(t, &hit, gain, 0.0, 0.4);
        }
        CueKind::Whoosh => {
            let w = synth::whoosh(sc, 0.45, 300.0, 6000.0, false);
            sc.add(t - 0.45, &w, gain, 0.0, 0.3);
        }
    }
}

fn add_at(mix: &mut [Vec<f64>; 2], x: &[Vec<f64>; 2], t: f64, g: f64) {
    let a0 = (t * SR as f64).round() as isize;
    for ch in 0..2 {
        for (i, v) in x[ch].iter().enumerate() {
            let j = a0 + i as isize;
            if j < 0 {
                continue;
            }
            if let Some(d) = mix[ch].get_mut(j as usize) {
                *d += v * g;
            }
        }
    }
}

fn peak(x: &[Vec<f64>; 2]) -> f64 {
    x.iter().flat_map(|c| c.iter()).fold(0.0f64, |a, v| a.max(v.abs()))
}

fn db(g: f64) -> f64 {
    10f64.powf(g / 20.0)
}

/// sidechain gain under the voice: 1 where it is silent, `duck` where it speaks (30 ms attack, 350 ms release)
fn duck_gain(vo: &[Vec<f64>; 2], n: usize, offset: f64, duck_db: f64) -> Vec<f64> {
    let floor = db(-duck_db);
    let thr = db(-40.0);
    let (att, rel) = ((-1.0 / (0.03 * SR as f64)).exp(), (-1.0 / (0.35 * SR as f64)).exp());
    let a0 = (offset * SR as f64).round() as isize;
    let mut env = 0.0;
    (0..n)
        .map(|i| {
            let j = i as isize - a0;
            let x = if j >= 0 && (j as usize) < vo[0].len() { vo[0][j as usize].abs().max(vo[1][j as usize].abs()) } else { 0.0 };
            let target = if x > thr { 1.0 } else { 0.0 };
            let c = if target > env { att } else { rel };
            env = target + (env - target) * c;
            1.0 - (1.0 - floor) * env
        })
        .collect()
}

/// what shapes the mix (a change re-renders it)
fn audio_key(p: &Plan) -> String {
    let mtime = |s: &Path| std::fs::metadata(s).and_then(|m| m.modified()).map(|t| format!("{t:?}")).unwrap_or_default();
    let ovs: Vec<_> = p.overlays.iter().map(|o| json!([o.asset, o.t0, o.t1, o.sfx, mtime(&o.asset)])).collect();
    let files: Vec<String> = p.voiceover.iter().map(|v| mtime(Path::new(&v.file))).chain(p.music.file.iter().map(|f| mtime(Path::new(f)))).collect();
    key_of(&json!({"v": 1, "s": p.seconds, "f": p.flashes, "c": p.cues, "m": p.music, "d": p.music_drop, "o": p.music_outro, "vo": p.voiceover, "a": p.audio, "ov": ovs, "files": files}).to_string())
}

pub fn run(p: &Plan, ev: &Events) -> Result<()> {
    let out = p.audio_wav();
    let stamp = p.build_dir.join(".audio");
    let key = audio_key(p);
    if out.exists() && std::fs::read_to_string(&stamp).is_ok_and(|s| s == key) {
        ev.log("audio: unchanged, skipping");
        return Ok(());
    }
    std::fs::create_dir_all(&p.build_dir)?;
    let tmp = p.build_dir.join("decode.tmp.wav");
    let total = p.seconds;
    let mut sc = Score::new(SR, total, p.music.seed);
    if p.music.kind == MusicKind::Procedural {
        procedural_bed(&mut sc, p);
    }
    if p.music.flash_hits {
        for &f in &p.flashes {
            add_cue(&mut sc, CueKind::Whoosh, f, 0.35);
            add_cue(&mut sc, CueKind::Hit, f, 0.7);
        }
    }
    for c in &p.cues {
        add_cue(&mut sc, c.kind, c.t, c.gain);
    }
    let ir = sc.impulse(2.2);
    let mut mix = sc.mixdown(&ir);
    for ch in mix.iter_mut() {
        ch.truncate(sc.n);
    }
    if let (MusicKind::File, Some(f)) = (p.music.kind, &p.music.file) {
        let x = media::decode_audio(Path::new(f), &tmp, p.music.start, Some(total)).with_context(|| format!("music file {f}"))?;
        let pk = peak(&x).max(1e-9);
        add_at(&mut mix, &x, 0.0, 0.5 / pk * db(p.music.gain_db));
    }
    // the bed rings out over the last `fade_out` seconds
    let fo = p.music.fade_out.max(1e-3);
    let fade0 = total - fo;
    for ch in mix.iter_mut() {
        for (i, v) in ch.iter_mut().enumerate() {
            let x = i as f64 / SR as f64;
            if x > fade0 {
                *v *= (1.0 - (x - fade0) / fo).max(0.0).powi(2);
            }
        }
    }
    // bed level before the sfx and the voice go on top
    let pk = peak(&mix);
    if pk > 1e-9 {
        for ch in mix.iter_mut() {
            ch.iter_mut().for_each(|v| *v *= 0.5 / pk);
        }
    }
    let vo = match &p.voiceover {
        Some(v) => Some((v, media::decode_audio(Path::new(&v.file), &tmp, 0.0, None).with_context(|| format!("voice-over {}", v.file))?)),
        None => None,
    };
    if let Some((v, x)) = &vo {
        let g = duck_gain(x, sc.n, v.offset, v.duck_db);
        for ch in mix.iter_mut() {
            ch.iter_mut().zip(&g).for_each(|(s, g)| *s *= g);
        }
    }
    for o in &p.overlays {
        if o.sfx <= 0.0 || !media::has_audio(&o.asset)? {
            continue;
        }
        let x = media::decode_audio(&o.asset, &tmp, 0.0, Some(o.t1 - o.t0)).with_context(|| format!("sfx of {}", o.label()))?;
        add_at(&mut mix, &x, o.t0, o.sfx * 0.6);
        ev.log(&format!("audio: sfx {} at {:.2}s", o.label(), o.t0));
    }
    if let Some((v, x)) = &vo {
        let pk = peak(x).max(1e-9);
        add_at(&mut mix, x, v.offset, 0.9 / pk * db(v.gain_db));
        ev.log(&format!("audio: voice-over {} at {:.2}s", v.file, v.offset));
    }
    anyhow::ensure!(peak(&mix) > 1e-6, "the mix is silent: set music.kind, a voice-over or overlay sfx");
    let (y, l) = master::master(&mix, SR, &master::Target { lufs: p.audio.lufs, ceiling_db: p.audio.ceiling_db, ceiling_env: vec![] })?;
    write_wav(&out, SR, &y)?;
    std::fs::write(&stamp, &key)?;
    std::fs::write(p.build_dir.join("audio_loudness.json"), serde_json::to_string_pretty(&l)?)?;
    ev.log(&format!("audio: {} ({:.2} LUFS, {:.2} dBTP, LRA {:.1})", out.display(), l.integrated_lufs, l.true_peak_dbtp, l.lra));
    Ok(())
}
