//! Caption text and timing: splitting a beat's text into the 1–2 word groups that pop in, the timed script format
//! (for captions written by hand or aligned to a voice-over) and the SRT sidecar.
//!
//! Timed script (`.txt`), one caption per line, `#` starts a comment:
//! ```text
//! 0.10   ¿Y SI
//! 0.55   MINECRAFT
//! 0:02.8 EN SPOOKMC          # m:ss.s works too (an inline comment needs a space on both sides of `#`)
//! 3.50-4.20 YA ES POSIBLE    # an explicit end
//! end 18.9                   # when the last caption disappears (default: the next start or the video end)
//! ```
//! A caption without an explicit end lasts until the next caption starts. `.srt` files are read as well.
use crate::error::ConfigError;
use serde::{Deserialize, Serialize};
use std::fmt::Write;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Caption {
    pub t0: f64,
    pub t1: f64,
    pub text: String,
}

/// Longest group (characters) that short words are merged into by [`groups`].
const GROUP_MAX_CHARS: usize = 13;
/// a word this short (characters) joins its neighbour ("EN SPOOKMC", "A OTRAS")
const SHORT_WORD: usize = 3;

/// Split a beat's text into caption groups. `|` splits explicitly ("¿Y SI | MINECRAFT"); without it, each word is its
/// own group except runs of short words, which join the following word while the group stays ≤ 13 characters.
pub fn groups(text: &str) -> Vec<String> {
    if text.contains('|') {
        return text.split('|').map(|s| s.split_whitespace().collect::<Vec<_>>().join(" ")).filter(|s| !s.is_empty()).collect();
    }
    let mut out: Vec<String> = vec![];
    let mut cur: Vec<&str> = vec![];
    for w in text.split_whitespace() {
        let all_short = cur.iter().all(|c| c.chars().count() <= SHORT_WORD);
        let len = cur.iter().map(|c| c.chars().count() + 1).sum::<usize>() + w.chars().count();
        if !cur.is_empty() && !(all_short && len <= GROUP_MAX_CHARS) {
            out.push(cur.join(" "));
            cur.clear();
        }
        cur.push(w);
    }
    if !cur.is_empty() {
        out.push(cur.join(" "));
    }
    out
}

/// Reading time of one caption group (s): long words stay a little longer.
pub fn group_time(g: &str) -> f64 {
    (0.32 + 0.03 * g.chars().count() as f64).clamp(0.45, 0.8)
}

/// Spread `gs` over `[t0, t1)`: each group gets a share proportional to its length (+3 so short words still read).
pub fn spread(gs: &[String], t0: f64, t1: f64) -> Vec<Caption> {
    let w: Vec<f64> = gs.iter().map(|g| 3.0 + g.chars().count() as f64).collect();
    let sum: f64 = w.iter().sum();
    let mut acc = 0.0;
    let mut v = Vec::with_capacity(gs.len());
    for (g, wi) in gs.iter().zip(&w) {
        let a = t0 + (t1 - t0) * acc / sum;
        acc += wi;
        let b = t0 + (t1 - t0) * acc / sum;
        v.push(Caption { t0: a, t1: b, text: g.clone() });
    }
    v
}

/// `12.5`, `0:12.5`, `1:02:03.5` or SRT's `00:00:12,500`
pub fn parse_time(s: &str) -> Option<f64> {
    let s = s.trim().replace(',', ".");
    let mut t = 0.0;
    for part in s.split(':') {
        t = t * 60.0 + part.parse::<f64>().ok()?;
    }
    Some(t)
}

/// Captions from `[[t, text]]` pairs: each lasts until the next start, the last one until `end`.
pub fn from_words(words: &[(f64, String)], end: f64) -> Vec<Caption> {
    words.iter().enumerate().map(|(i, (t0, text))| Caption { t0: *t0, t1: words.get(i + 1).map(|w| w.0).unwrap_or(end), text: text.clone() }).collect()
}

/// Parse a timed script (`.txt` lines or SRT). `end` closes the last open caption. `offset` shifts every time.
pub fn parse_script(src: &str, is_srt: bool, end: f64, offset: f64) -> Result<Vec<Caption>, ConfigError> {
    let mut out = if is_srt { parse_srt(src)? } else { parse_lines(src, end)? };
    for c in out.iter_mut() {
        c.t0 += offset;
        c.t1 += offset;
    }
    for p in out.windows(2) {
        if p[1].t0 < p[0].t0 {
            return Err(ConfigError(format!("script captions are out of order at «{}» ({:.2} s)", p[1].text, p[1].t0)));
        }
    }
    Ok(out)
}

fn parse_lines(src: &str, end: f64) -> Result<Vec<Caption>, ConfigError> {
    let mut raw: Vec<(f64, Option<f64>, String)> = vec![];
    let mut end = end;
    for (n, line) in src.lines().enumerate() {
        let line = line.split(" # ").next().unwrap_or("").trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (head, text) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
        let err = || ConfigError(format!("script line {}: expected `SECONDS TEXT`, got «{line}»", n + 1));
        if head == "end" {
            end = parse_time(text).ok_or_else(err)?;
            continue;
        }
        let (a, b) = match head.split_once('-') {
            Some((a, b)) => (parse_time(a).ok_or_else(err)?, Some(parse_time(b).ok_or_else(err)?)),
            None => (parse_time(head).ok_or_else(err)?, None),
        };
        let text = text.trim();
        if text.is_empty() {
            return Err(err());
        }
        raw.push((a, b, text.to_string()));
    }
    Ok(raw.iter().enumerate().map(|(i, (a, b, t))| Caption { t0: *a, t1: b.unwrap_or_else(|| raw.get(i + 1).map(|r| r.0).unwrap_or(end)), text: t.clone() }).collect())
}

fn parse_srt(src: &str) -> Result<Vec<Caption>, ConfigError> {
    let mut out = vec![];
    let src = src.replace("\r\n", "\n");
    for block in src.split("\n\n") {
        let lines: Vec<&str> = block.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
        let Some(ti) = lines.iter().position(|l| l.contains("-->")) else { continue };
        let (a, b) = lines[ti].split_once("-->").unwrap();
        let p = |s: &str| parse_time(s.split_whitespace().next().unwrap_or("")).ok_or_else(|| ConfigError(format!("bad SRT time «{}»", lines[ti])));
        let text = lines[ti + 1..].join(" ");
        if !text.is_empty() {
            out.push(Caption { t0: p(a)?, t1: p(b)?, text });
        }
    }
    Ok(out)
}

fn srt_time(t: f64) -> String {
    let ms = (t.max(0.0) * 1000.0).round() as u64;
    format!("{:02}:{:02}:{:02},{:03}", ms / 3_600_000, ms / 60_000 % 60, ms / 1000 % 60, ms % 1000)
}

/// The SRT sidecar (for subtitles added downstream).
pub fn to_srt(caps: &[Caption]) -> String {
    let mut s = String::new();
    for (i, c) in caps.iter().enumerate() {
        let _ = write!(s, "{}\n{} --> {}\n{}\n\n", i + 1, srt_time(c.t0), srt_time(c.t1), c.text);
    }
    s
}
