//! Caption font metrics read from the font file itself (family name for the SVG, advance widths to fit a caption to
//! the safe width), so any TTF/OTF can be dropped in without a hand-made width table.
use crate::error::{ConfigError, MissingInput};
use anyhow::Result;
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone)]
pub struct FontInfo {
    /// family name as fframes / fontdb see it (typographic family, else the legacy family)
    pub family: String,
    /// advance width per char in em
    advances: HashMap<char, f64>,
    fallback: f64,
    ascii: [f64; 128],
    /// bounded one-entry cache for the most recently measured caption
    cache: Arc<Mutex<Option<(String, f64)>>>,
}

impl FontInfo {
    pub fn load(path: &Path) -> Result<Self> {
        let data = std::fs::read(path).map_err(|e| MissingInput(format!("caption font {}: {e}", path.display())))?;
        let face = ttf_parser::Face::parse(&data, 0).map_err(|e| ConfigError(format!("caption font {} is not a TTF/OTF: {e}", path.display())))?;
        let name = |id: u16| face.names().into_iter().filter(|n| n.name_id == id).find_map(|n| n.to_string());
        let family = name(ttf_parser::name_id::TYPOGRAPHIC_FAMILY).or_else(|| name(ttf_parser::name_id::FAMILY)).ok_or_else(|| ConfigError(format!("font {} has no family name", path.display())))?;
        let upem = face.units_per_em() as f64;
        let mut advances = HashMap::new();
        for c in (0x20u32..0x250).filter_map(char::from_u32) {
            if let Some(a) = face.glyph_index(c).and_then(|g| face.glyph_hor_advance(g)) {
                advances.insert(c, a as f64 / upem);
            }
        }
        let fallback = advances.get(&'N').copied().unwrap_or(0.6);
        let mut ascii = [fallback; 128];
        for code in 0u32..128 {
            if let Some(c) = char::from_u32(code) {
                if let Some(advance) = advances.get(&c).copied() {
                    ascii[code as usize] = advance;
                }
            }
        }
        Ok(Self {
            family,
            advances,
            fallback,
            ascii,
            cache: Arc::new(Mutex::new(None)),
        })
    }

    /// width of `s` in em
    pub fn width_em(&self, s: &str) -> f64 {
        if let Ok(guard) = self.cache.lock() {
            if let Some((cached, width)) = guard.as_ref() {
                if cached == s {
                    return *width;
                }
            }
        }

        let width = s
            .chars()
            .map(|c| {
                let code = c as usize;
                if code < 128 {
                    self.ascii[code]
                } else {
                    self.advances.get(&c).copied().unwrap_or(self.fallback)
                }
            })
            .sum();

        if let Ok(mut guard) = self.cache.lock() {
            *guard = Some((s.to_string(), width));
        }
        width
    }

    /// the caption size: `size`, shrunk so the text fits `max_w` (design px)
    pub fn fit(&self, s: &str, size: f64, max_w: f64) -> f64 {
        let em = self.width_em(s).max(1e-6);
        size.min(max_w / em)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn font_path() -> PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fonts/PixelifySans-Bold.ttf")
    }

    fn hashmap_width(fi: &FontInfo, s: &str) -> f64 {
        s.chars().map(|c| fi.advances.get(&c).copied().unwrap_or(fi.fallback)).sum()
    }

    #[test]
    fn ascii_caption_width_matches_hashmap() {
        let fi = FontInfo::load(&font_path()).unwrap();
        let s = "HELLO, WORLD! The quick brown fox 0123456789";
        assert_eq!(fi.width_em(s), hashmap_width(&fi, s));
    }

    #[test]
    fn spanish_unicode_width_matches_hashmap() {
        let fi = FontInfo::load(&font_path()).unwrap();
        let s = "Señor pingüino: acción, ñáéíóú";
        assert_eq!(fi.width_em(s), hashmap_width(&fi, s));
    }

    #[test]
    fn empty_string_width_is_zero() {
        let fi = FontInfo::load(&font_path()).unwrap();
        assert_eq!(fi.width_em(""), 0.0);
        assert_eq!(hashmap_width(&fi, ""), 0.0);
    }

    #[test]
    fn unsupported_char_uses_fallback() {
        let fi = FontInfo::load(&font_path()).unwrap();
        let s = "😀中";
        assert_eq!(fi.width_em(s), hashmap_width(&fi, s));
        assert_eq!(fi.width_em("😀"), fi.fallback);
    }

    #[test]
    fn clone_preserves_cached_lookup() {
        let fi = FontInfo::load(&font_path()).unwrap();
        let cloned = fi.clone();
        assert_eq!(fi.width_em("ABC"), cloned.width_em("ABC"));
        assert_eq!(fi.width_em("Señor"), cloned.width_em("Señor"));
    }

    #[test]
    fn missing_or_invalid_font_returns_error() {
        assert!(FontInfo::load(std::path::Path::new("/definitely/missing/font.ttf")).is_err());
        let path = std::env::temp_dir().join("reelsmith-font-unsupported.ttf");
        std::fs::write(&path, b"not a font").unwrap();
        assert!(FontInfo::load(&path).is_err());
    }

    #[test]
    #[ignore = "benchmark: cargo test -p promo --lib font::tests::bench_width_em_many_calls -- --ignored --nocapture"]
    fn bench_width_em_many_calls() {
        use std::hint::black_box;
        use std::time::Instant;

        let fi = FontInfo::load(&font_path()).unwrap();
        let inputs = [
            "HELLO, WORLD! The quick brown fox 0123456789",
            "Señor pingüino: acción, ñáéíóú",
            "",
        ];
        let iters = 100_000usize;

        let mut optimized = Vec::with_capacity(5);
        for _ in 0..5 {
            let start = Instant::now();
            let mut total = 0.0;
            for _ in 0..iters {
                for s in &inputs {
                    total += black_box(fi.width_em(s));
                }
            }
            black_box(total);
            optimized.push(start.elapsed());
        }

        let mut hashmap = Vec::with_capacity(5);
        for _ in 0..5 {
            let start = Instant::now();
            let mut total = 0.0;
            for _ in 0..iters {
                for s in &inputs {
                    total += black_box(hashmap_width(&fi, s));
                }
            }
            black_box(total);
            hashmap.push(start.elapsed());
        }

        optimized.sort();
        hashmap.sort();
        eprintln!("font width_em optimized median of 5: {:?}", optimized[2]);
        eprintln!("font width_em hashmap median of 5: {:?}", hashmap[2]);
    }

    #[test]
    #[ignore = "benchmark: cargo test -p promo --lib font::tests::bench_width_repeat_cache -- --ignored --nocapture"]
    fn bench_width_repeat_cache() {
        use std::hint::black_box;
        use std::time::Instant;

        let fi = FontInfo::load(&font_path()).unwrap();
        let caption = "A".repeat(100);
        let iters = 200_000usize;

        // warm the one-entry cache
        let _ = fi.width_em(&caption);

        let mut cached = Vec::with_capacity(5);
        for _ in 0..5 {
            let start = Instant::now();
            let mut total = 0.0;
            for _ in 0..iters {
                total += black_box(fi.width_em(&caption));
            }
            black_box(total);
            cached.push(start.elapsed());
        }

        let mut exact = Vec::with_capacity(5);
        for _ in 0..5 {
            let start = Instant::now();
            let mut total = 0.0;
            for _ in 0..iters {
                let w: f64 = caption
                    .chars()
                    .map(|c| {
                        let code = c as usize;
                        if code < 128 {
                            fi.ascii[code]
                        } else {
                            fi.advances.get(&c).copied().unwrap_or(fi.fallback)
                        }
                    })
                    .sum();
                total += black_box(w);
            }
            black_box(total);
            exact.push(start.elapsed());
        }

        cached.sort();
        exact.sort();
        eprintln!("width_em cached median of 5: {:?}", cached[2]);
        eprintln!("exact ascii no-mutex median of 5: {:?}", exact[2]);
    }

    #[test]
    fn alternating_ascii_unicode_empty_widths_match_hashmap() {
        let fi = FontInfo::load(&font_path()).unwrap();
        let texts = ["", "HELLO, WORLD!", "Señor pingüino: acción, ñáéíóú", "0123456789", "😀中", ""];
        for t in &texts {
            assert_eq!(fi.width_em(t), hashmap_width(&fi, t));
        }
        for _ in 0..10 {
            for t in &texts {
                assert_eq!(fi.width_em(t), hashmap_width(&fi, t));
            }
        }
    }

    #[test]
    fn clone_shares_cache_across_alternating_texts() {
        let fi = FontInfo::load(&font_path()).unwrap();
        let cloned = fi.clone();
        let texts = ["ABC", "Señor", "", "HELLO, WORLD! The quick brown fox"];
        for t in &texts {
            assert_eq!(fi.width_em(t), cloned.width_em(t));
        }
        for _ in 0..10 {
            for t in &texts {
                let a = fi.width_em(t);
                let b = cloned.width_em(t);
                assert_eq!(a, b);
            }
        }
    }
}
