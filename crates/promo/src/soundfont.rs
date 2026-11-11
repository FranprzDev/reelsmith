//! Sampled music bed: renders a stereo f64 mix from a local SoundFont (SF2) bank with rustysynth.
//! The bed replaces the procedural bed; mastering, voice-over ducking and cues stay in `audio`.

use crate::config::{MusicKind, SoundfontStyle};
use crate::plan::Plan;
use anyhow::{Context, Result, bail};
use rustysynth::{SoundFont, Synthesizer, SynthesizerSettings};
use std::io::{Cursor, Read};
use std::sync::Arc;

pub const MAX_SF2_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Clone)]
struct Ev {
    time: f64,
    on: bool,
    channel: i32,
    key: i32,
    vel: i32,
}

fn energy_at(p: &Plan, t: f64) -> f64 {
    let frame = (t * p.fps as f64).round() as usize;
    for seg in &p.music_energy {
        if frame >= seg.start_frame && frame < seg.end_frame {
            return seg.level;
        }
    }
    0.5
}

fn vel(v: f64) -> i32 {
    (v.clamp(0.0, 1.0) * 127.0).round() as i32
}

fn note(events: &mut Vec<Ev>, t: f64, dur: f64, channel: i32, key: i32, velocity: f64, total: f64) {
    if !t.is_finite() || t < 0.0 || t >= total - 1e-4 {
        return;
    }
    if !velocity.is_finite() || velocity <= 0.0 {
        return;
    }
    let off = (t + dur).min(total);
    if !(off > t + 1e-6) {
        return;
    }
    let v = vel(velocity);
    if v <= 0 {
        return;
    }
    if !(0..=127).contains(&key) {
        return;
    }
    events.push(Ev { time: t, on: true, channel, key, vel: v });
    events.push(Ev { time: off, on: false, channel, key, vel: 0 });
}

fn style_bpm(style: SoundfontStyle) -> f64 {
    match style {
        SoundfontStyle::Acoustic => 84.0,
        SoundfontStyle::Cinematic => 96.0,
        SoundfontStyle::Funk => 112.0,
    }
}

fn required_presets(style: SoundfontStyle) -> &'static [(i32, i32, &'static str)] {
    match style {
        SoundfontStyle::Acoustic => &[
            (0, 0, "acoustic grand piano"),
            (0, 24, "nylon guitar"),
            (0, 32, "acoustic bass"),
            (128, 40, "brush drum kit"),
        ],
        SoundfontStyle::Cinematic => &[
            (0, 48, "string ensemble"),
            (0, 60, "french horn"),
            (0, 47, "timpani"),
        ],
        SoundfontStyle::Funk => &[
            (0, 4, "electric piano 1"),
            (0, 27, "clean electric guitar"),
            (0, 33, "finger bass"),
            (128, 0, "standard drum kit"),
        ],
    }
}

fn has_preset(sf: &SoundFont, bank: i32, prog: i32) -> bool {
    sf.get_presets().iter().any(|pr| pr.get_bank_number() == bank && pr.get_patch_number() == prog)
}

fn setup_channels(synth: &mut Synthesizer, style: SoundfontStyle) {
    match style {
        SoundfontStyle::Acoustic => {
            // ch0 piano, ch1 bass, ch2 nylon, ch9 drums
            synth.process_midi_message(0, 0xC0, 0, 0);
            synth.process_midi_message(1, 0xC0, 32, 0);
            synth.process_midi_message(2, 0xC0, 24, 0);
            synth.process_midi_message(9, 0xC0, 40, 0);
        }
        SoundfontStyle::Cinematic => {
            synth.process_midi_message(0, 0xC0, 48, 0);
            synth.process_midi_message(1, 0xC0, 60, 0);
            synth.process_midi_message(2, 0xC0, 47, 0);
        }
        SoundfontStyle::Funk => {
            synth.process_midi_message(0, 0xC0, 4, 0);
            synth.process_midi_message(1, 0xC0, 27, 0);
            synth.process_midi_message(2, 0xC0, 33, 0);
            synth.process_midi_message(9, 0xC0, 0, 0);
        }
    }
}

fn compose(p: &Plan, style: SoundfontStyle, events: &mut Vec<Ev>) {
    let total = p.seconds;
    let bpm = p.music.bpm.unwrap_or_else(|| {
        if let Some(d) = p.music.direction {
            d.default_bpm()
        } else {
            style_bpm(style)
        }
    });
    let beat = 60.0 / bpm;
    let bar = 4.0 * beat;
    let prog = p.music.progression.as_deref().unwrap_or(&[]);
    let fallback = crate::config::default_music_progression();
    let prog = if prog.is_empty() { fallback.as_slice() } else { prog };

    match style {
        SoundfontStyle::Acoustic => {
            // bass: one note per beat
            let mut i = 0usize;
            while (i as f64) * beat < total {
                let t = i as f64 * beat;
                let e = energy_at(p, t);
                if e > 0.2 {
                    let bar_i = (t / bar).floor() as usize;
                    let (root, _) = prog[bar_i % prog.len()];
                    let v = 0.72 * (0.6 + 0.4 * e);
                    note(events, t, beat * 0.85, 1, root as i32, v, total);
                }
                i += 1;
            }
            // chords (piano) every 2 beats
            let mut i = 0usize;
            while (i as f64) * beat * 2.0 < total {
                let t = i as f64 * beat * 2.0;
                let e = energy_at(p, t);
                let bar_i = (t / bar).floor() as usize;
                let (_, triad) = prog[bar_i % prog.len()];
                let mut seen = std::collections::HashSet::new();
                for &n in triad.iter() {
                    if !seen.insert(n as i64) { continue; }
                    note(events, t, beat * 1.9, 0, n as i32, 0.62 * (0.7 + 0.3 * e), total);
                }
                i += 1;
            }
            // melody (nylon) half-beat
            let mut j = 0usize;
            while (j as f64) * (beat / 2.0) < total {
                let t = j as f64 * (beat / 2.0);
                let e = energy_at(p, t);
                if e > 0.2 {
                    let bar_i = (t / bar).floor() as usize;
                    let (_, triad) = prog[bar_i % prog.len()];
                    let idx = j % 3;
                    let mm = triad[idx] + 12.0;
                    note(events, t, beat * 0.42, 2, mm as i32, 0.58 * (0.7 + 0.3 * e), total);
                }
                j += 1;
            }
            // brush drums
            let mut i = 0usize;
            while (i as f64) * beat < total {
                let t = i as f64 * beat;
                let e = energy_at(p, t);
                if e > 0.25 {
                    note(events, t, 0.12, 9, 42, 0.35 * (0.5 + 0.5 * e), total);
                    if i % 4 == 0 {
                        note(events, t, 0.18, 9, 36, 0.75 * (0.6 + 0.4 * e), total);
                    }
                    if i % 4 == 2 {
                        note(events, t, 0.18, 9, 38, 0.68 * (0.6 + 0.4 * e), total);
                    }
                }
                i += 1;
            }
            // crash near end of drop? add at first flash if any
            for &f in &p.flashes {
                note(events, f, 0.3, 9, 46, 0.3, total);
            }
        }
        SoundfontStyle::Cinematic => {
            // strings: chord per bar
            let mut i = 0usize;
            while (i as f64) * bar < total {
                let t = i as f64 * bar;
                let e = energy_at(p, t);
                let bar_i = i;
                let (_, triad) = prog[bar_i % prog.len()];
                let mut seen = std::collections::HashSet::new();
                for &n in triad.iter() {
                    if !seen.insert(n as i64) { continue; }
                    note(events, t, beat * 3.6, 0, n as i32, 0.55 * (0.7 + 0.3 * e), total);
                }
                i += 1;
            }
            // horn: one note per beat
            let mut i = 0usize;
            while (i as f64) * beat < total {
                let t = i as f64 * beat;
                let e = energy_at(p, t);
                if e > 0.2 {
                    let bar_i = (t / bar).floor() as usize;
                    let (_, triad) = prog[bar_i % prog.len()];
                    let pat = [0usize, 2, 1, 0, 2, 1, 0, 1];
                    let idx = pat[i % pat.len()];
                    let mm = triad[idx % 3] + 12.0;
                    note(events, t, beat * 0.9, 1, mm as i32, 0.62 * (0.7 + 0.3 * e), total);
                }
                i += 1;
            }
            // timpani: every other bar
            let mut i = 0usize;
            while (i as f64) * bar * 2.0 < total {
                let t = i as f64 * bar * 2.0;
                let e = energy_at(p, t);
                let bar_i = (t / bar).floor() as usize;
                let (root, _) = prog[bar_i % prog.len()];
                note(events, t, beat * 1.5, 2, (root as i32) - 12, 0.82 * (0.7 + 0.3 * e), total);
                i += 1;
            }
        }
        SoundfontStyle::Funk => {
            // ep chords every beat
            let mut i = 0usize;
            while (i as f64) * beat < total {
                let t = i as f64 * beat;
                let e = energy_at(p, t);
                if e > 0.2 {
                    let bar_i = (t / bar).floor() as usize;
                    let (_, triad) = prog[bar_i % prog.len()];
                    let mut seen = std::collections::HashSet::new();
                    for &n in triad.iter() {
                        if !seen.insert(n as i64) { continue; }
                        note(events, t, beat * 0.28, 0, n as i32, 0.58 * (0.7 + 0.3 * e), total);
                    }
                    // off-beat
                    let toff = t + beat * 0.5;
                    if toff < total {
                        let mut seen = std::collections::HashSet::new();
                        for &n in triad.iter() {
                            if !seen.insert(n as i64) { continue; }
                            note(events, toff, beat * 0.22, 0, n as i32, 0.50 * (0.7 + 0.3 * e), total);
                        }
                    }
                }
                i += 1;
            }
            // clean guitar riff every half beat
            let riff = [0i32, 2, 4, 7, 4, 2, 0, -5];
            let mut j = 0usize;
            while (j as f64) * (beat / 2.0) < total {
                let t = j as f64 * (beat / 2.0);
                let e = energy_at(p, t);
                if e > 0.2 {
                    let bar_i = (t / bar).floor() as usize;
                    let (_, triad) = prog[bar_i % prog.len()];
                    let ri = riff[j % riff.len()];
                    let base = triad[0];
                    let mm = base + ri as f64;
                    note(events, t, beat * 0.35, 1, mm as i32, 0.55 * (0.7 + 0.3 * e), total);
                }
                j += 1;
            }
            // finger bass every quarter beat (16th)
            let mut j = 0usize;
            while (j as f64) * (beat / 4.0) < total {
                let t = j as f64 * (beat / 4.0);
                let e = energy_at(p, t);
                if e > 0.25 {
                    let bar_i = (t / bar).floor() as usize;
                    let (root, _) = prog[bar_i % prog.len()];
                    let key = if j % 8 == 3 {
                        root as i32 + 5
                    } else if j % 8 == 7 {
                        root as i32 + 7
                    } else {
                        root as i32
                    };
                    note(events, t, beat * 0.22, 2, key, 0.68 * (0.7 + 0.3 * e), total);
                }
                j += 1;
            }
            // drums
            let mut i = 0usize;
            while (i as f64) * beat < total {
                let t = i as f64 * beat;
                let e = energy_at(p, t);
                if e > 0.25 {
                    // hats two per beat
                    for h in 0..2 {
                        let th = t + h as f64 * beat * 0.5;
                        if th < total {
                            note(events, th, 0.08, 9, 42, 0.32 * (0.5 + 0.5 * e), total);
                        }
                    }
                    if i % 2 == 0 {
                        note(events, t, 0.16, 9, 36, 0.78 * (0.6 + 0.4 * e), total);
                    } else {
                        note(events, t, 0.16, 9, 38, 0.72 * (0.6 + 0.4 * e), total);
                    }
                    if i % 4 == 3 {
                        note(events, t + beat * 0.75, 0.12, 9, 46, 0.34 * (0.5 + 0.5 * e), total);
                    }
                }
                i += 1;
            }
        }
    }
}

pub fn render(p: &Plan) -> Result<[Vec<f64>; 2]> {
    if p.music.kind != MusicKind::Soundfont {
        bail!("music.kind is not \"soundfont\"");
    }
    if !p.seconds.is_finite() || p.seconds <= 0.0 || p.seconds > 180.0 {
        bail!("invalid render duration {} (must be finite, > 0 and <= 180 s)", p.seconds);
    }
    if let Some(bpm) = p.music.bpm {
        if !bpm.is_finite() || !(1.0..=300.0).contains(&bpm) {
            bail!("music.bpm = {bpm}: must be between 1 and 300");
        }
    }
    let sf_path = p
        .music
        .soundfont
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("music.soundfont is required"))?;
    let style = p
        .music
        .soundfont_style
        .ok_or_else(|| anyhow::anyhow!("music.soundfont_style is required"))?;

    let meta = std::fs::metadata(sf_path).with_context(|| format!("reading soundfont {sf_path}"))?;
    if !meta.file_type().is_file() {
        bail!("soundfont {} is not a regular file", sf_path);
    }
    if meta.len() == 0 {
        bail!("soundfont {} is empty", sf_path);
    }
    let file = std::fs::File::open(sf_path).with_context(|| format!("opening soundfont {sf_path}"))?;
    let mut bytes = Vec::new();
    let mut limited = file.take(MAX_SF2_BYTES + 1);
    limited.read_to_end(&mut bytes).context("reading soundfont")?;
    if bytes.len() as u64 > MAX_SF2_BYTES {
        bail!(
            "soundfont {} exceeds the {} byte limit",
            sf_path,
            MAX_SF2_BYTES
        );
    }
    if bytes.is_empty() {
        bail!("soundfont {} is empty", sf_path);
    }
    let mut cursor = Cursor::new(&bytes);
    let sf = SoundFont::new(&mut cursor).map_err(|e| anyhow::anyhow!("invalid soundfont {sf_path}: {e}"))?;
    drop(cursor);
    drop(bytes);

    for (bank, prog, label) in required_presets(style) {
        if !has_preset(&sf, *bank, *prog) {
            bail!(
                "soundfont {} is missing required GM preset {} (bank={}, program={})",
                sf_path,
                label,
                bank,
                prog
            );
        }
    }

    let settings = SynthesizerSettings::new(48000);
    let mut synth = Synthesizer::new(&Arc::new(sf), &settings)
        .map_err(|e| anyhow::anyhow!("creating synthesizer: {e}"))?;
    synth.set_master_volume(0.8);
    setup_channels(&mut synth, style);

    // validate harmony pitches are integer MIDI notes in range before rendering
    if let Some(prog) = &p.music.progression {
        for (root, triad) in prog {
            if !root.is_finite() || root.fract() != 0.0 || !((0.0)..=127.0).contains(root) {
                bail!("soundfont requires integer MIDI progression roots 0..=127, got {root}");
            }
            for n in triad {
                if !n.is_finite() || n.fract() != 0.0 || !((0.0)..=127.0).contains(n) {
                    bail!("soundfont requires integer MIDI progression notes 0..=127, got {n}");
                }
            }
        }
    }
    let mut events = Vec::new();
    compose(p, style, &mut events);
    let mut events: Vec<(usize, bool, i32, i32, i32)> = events
        .into_iter()
        .map(|e| (((e.time * 48000.0).round() as usize), e.on, e.channel, e.key, e.vel))
        .collect();
    events.sort_by(|a, b| {
        if a.0 != b.0 {
            a.0.cmp(&b.0)
        } else if a.1 != b.1 {
            // offs (false) before ons (true)
            (a.1 as i32).cmp(&(b.1 as i32))
        } else if a.2 != b.2 {
            a.2.cmp(&b.2)
        } else {
            a.3.cmp(&b.3)
        }
    });

    let sr = 48000usize;
    let n = (p.seconds * sr as f64) as usize;
    if n == 0 || n > 180 * sr {
        bail!("invalid sample count {n}");
    }
    let mut left = vec![0.0f32; n];
    let mut right = vec![0.0f32; n];
    let mut pos = 0usize;
    let mut ei = 0usize;
    while pos < n {
        while ei < events.len() && events[ei].0 <= pos {
            let ev = &events[ei];
            if ev.1 {
                synth.note_off(ev.2, ev.3);
                synth.note_on(ev.2, ev.3, ev.4);
            } else {
                synth.note_off(ev.2, ev.3);
            }
            ei += 1;
        }
        let next_frame = if ei < events.len() {
            events[ei].0.max(pos).min(n)
        } else {
            n
        };
        let mut chunk = next_frame.saturating_sub(pos);
        if chunk == 0 {
            chunk = 1;
        }
        if pos + chunk > n {
            chunk = n - pos;
        }
        if chunk > 0 {
            synth.render(&mut left[pos..pos + chunk], &mut right[pos..pos + chunk]);
            pos += chunk;
        }
    }
    synth.note_off_all(true);

    let mut out = [vec![0.0f64; n], vec![0.0f64; n]];
    for i in 0..n {
        out[0][i] = left[i] as f64;
        out[1][i] = right[i] as f64;
    }
    Ok(out)
}
