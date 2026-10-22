//! Caption font metrics read from the font file itself (family name for the SVG, advance widths to fit a caption to
//! the safe width), so any TTF/OTF can be dropped in without a hand-made width table.
use crate::error::{ConfigError, MissingInput};
use anyhow::Result;
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct FontInfo {
    /// family name as fframes / fontdb see it (typographic family, else the legacy family)
    pub family: String,
    /// advance width per char in em
    advances: HashMap<char, f64>,
    fallback: f64,
}

impl FontInfo {
    pub fn load(path: &Path) -> Result<Self> {
        let data = std::fs::read(path).map_err(|e| MissingInput(format!("caption font {}: {e}", path.display())))?;
        let face = ttf_parser::Face::parse(&data, 0).map_err(|e| ConfigError(format!("caption font {} is not a TTF/OTF: {e}", path.display())))?;
        let name = |id: u16| face.names().into_iter().filter(|n| n.name_id == id).find_map(|n| n.to_string());
        let family = name(ttf_parser::name_id::TYPOGRAPHIC_FAMILY).or_else(|| name(ttf_parser::name_id::FAMILY)).ok_or_else(|| ConfigError(format!("font {} has no family name", path.display())))?;
        let upem = face.units_per_em() as f64;
        let mut advances = HashMap::new();
        // Latin-1 + Latin Extended-A covers Spanish, Portuguese and the usual punctuation; others use the fallback
        for c in (0x20u32..0x250).filter_map(char::from_u32) {
            if let Some(a) = face.glyph_index(c).and_then(|g| face.glyph_hor_advance(g)) {
                advances.insert(c, a as f64 / upem);
            }
        }
        let fallback = advances.get(&'N').copied().unwrap_or(0.6);
        Ok(Self { family, advances, fallback })
    }

    /// width of `s` in em
    pub fn width_em(&self, s: &str) -> f64 {
        s.chars().map(|c| self.advances.get(&c).copied().unwrap_or(self.fallback)).sum()
    }

    /// the caption size: `size`, shrunk so the text fits `max_w` (design px)
    pub fn fit(&self, s: &str, size: f64, max_w: f64) -> f64 {
        let em = self.width_em(s).max(1e-6);
        size.min(max_w / em)
    }
}
