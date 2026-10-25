use crate::animate::{self, AlphaCodec, AnimateSvgArgs, AnimatedSvg, Animation, Easing, Fps, Width};
use anyhow::{Context as _, Result, bail};
use rquickjs::{Context, Runtime};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    #[default]
    Light,
    Dark,
}

impl Theme {
    fn vars(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Theme::Light => &[],
            Theme::Dark => &[
                ("--pele-bg", "#100f0f"),
                ("--pele-fg", "#cecdc3"),
                ("--pele-muted", "#878580"),
                ("--pele-line", "#cecdc3"),
                ("--pele-surface", "#1c1b1a"),
                ("--pele-surface-alt", "#282726"),
                ("--pele-border", "#403e3c"),
                ("--pele-accent", "#4385be"),
                ("--pele-critical", "#d14d41"),
                ("--pele-series-1", "#d14d41"),
                ("--pele-series-2", "#da702c"),
                ("--pele-series-3", "#d0a215"),
                ("--pele-series-4", "#879a39"),
                ("--pele-series-5", "#3aa99f"),
                ("--pele-series-6", "#4385be"),
                ("--pele-series-7", "#8b7ec8"),
                ("--pele-series-8", "#ce5d97"),
            ],
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DrawDiagramArgs {
    #[schemars(description = "the Mermaid source of the diagram; every Mermaid 12.1 type works: flowchart, sequenceDiagram, stateDiagram-v2, classDiagram, erDiagram, gantt, pie, mindmap, timeline, sankey-beta, journey, quadrantChart, xychart-beta, block-beta, gitGraph, C4Context, requirementDiagram, Kanban, packet-beta, architecture-beta")]
    pub source: String,
    #[schemars(description = "light (Pele's defaults) or dark (Obsidian's dark palette); use dark when the diagram sits on dark footage")]
    #[serde(default)]
    pub theme: Theme,
    #[schemars(description = "entrance preset (default draw_on: the diagram's strokes draw themselves, then the fills fade in); also fade_scale, slide_up, pop, spin or glow")]
    #[serde(default = "d_draw_on")]
    pub animation: Animation,
    #[schemars(description = "entrance length in seconds (0.2 to 10)")]
    pub duration_s: f64,
    #[schemars(description = "30 or 60 (default 60)")]
    #[serde(default = "animate::d_fps")]
    pub fps: Fps,
    #[schemars(description = "canvas width: 540, 720, 1080 or 1440 px (default 1080)")]
    #[serde(default = "animate::d_width")]
    pub width: Width,
    #[schemars(description = "timing curve: ease_out (default), ease_in_out, back or spring")]
    #[serde(default)]
    pub easing: Easing,
    #[schemars(description = "seconds the diagram rests after the entrance (default 1.0)")]
    pub hold_s: Option<f64>,
    #[schemars(description = "exit preset, played as that entrance backwards (default: none)")]
    pub exit: Option<Animation>,
    #[schemars(description = "exit length in seconds (default: duration_s)")]
    pub exit_s: Option<f64>,
    #[schemars(description = "qtrle (default) or prores4444")]
    #[serde(default)]
    pub codec: AlphaCodec,
    #[schemars(description = "output path, a .mov; put it in the project's motion folder to use it as overlay.asset / end_card.asset. The resolved SVG is written next to it.")]
    pub output: PathBuf,
}

fn d_draw_on() -> Animation {
    Animation::DrawOn
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct DrawnDiagram {
    #[serde(flatten)]
    #[schemars(flatten)]
    pub motion: AnimatedSvg,
    #[schemars(description = "the resolved SVG (concrete colors, no CSS variables) that was animated")]
    pub svg: PathBuf,
    #[schemars(description = "the diagram type Pele detected, such as flowchart or sequence")]
    pub diagram_type: String,
}

#[derive(Debug, Deserialize)]
struct PeleOk {
    svg: String,
    #[serde(rename = "type")]
    diagram_type: String,
}

#[derive(Debug, Deserialize)]
struct PeleErr {
    error: String,
    line: Option<u32>,
    column: Option<u32>,
    snippet: Option<String>,
}

const RENDER_JS: &str = r#"(function(){
  try {
    var src = globalThis.__pele_src;
    var type = pele.detectType(src);
    var r = pele.render(src);
    return JSON.stringify({ ok: { svg: r.svg, type: type || "unknown" } });
  } catch (e) {
    return JSON.stringify({ err: { error: String((e && e.message) || e), line: e && e.line, column: e && e.column, snippet: e && e.snippet } });
  }
})()"#;

fn pele_render(source: &str) -> Result<(String, String)> {
    let rt = Runtime::new().context("starting the embedded diagram engine")?;
    let ctx = Context::full(&rt).context("starting the embedded diagram engine")?;
    ctx.with(|ctx| -> Result<(String, String)> {
        ctx.eval::<(), _>(include_str!("../vendor/pele.global.js")).map_err(|e| anyhow::anyhow!("loading the embedded Pele engine: {e}"))?;
        ctx.globals().set("__pele_src", source).map_err(|e| anyhow::anyhow!("passing the diagram source: {e}"))?;
        let out: String = ctx.eval(RENDER_JS).map_err(|e| anyhow::anyhow!("running the embedded Pele engine: {e}"))?;
        let v: serde_json::Value = serde_json::from_str(&out).context("reading the diagram engine's answer")?;
        if let Ok(err) = serde_json::from_value::<PeleErr>(v["err"].clone()) {
            let mut msg = format!("the Mermaid source does not render: {}", err.error);
            if let (Some(l), Some(c)) = (err.line, err.column) {
                msg.push_str(&format!(" (line {l}, column {c})"));
            }
            if let Some(s) = err.snippet.filter(|s| !s.is_empty()) {
                msg.push_str(&format!(":\n{s}"));
            }
            bail!(msg);
        }
        let ok: PeleOk = serde_json::from_value(v["ok"].clone()).context("reading the rendered diagram")?;
        Ok((ok.svg, ok.diagram_type))
    })
}

fn resolve_var_ref(inner: &str, theme: &HashMap<String, String>, aliases: &HashMap<String, String>, depth: usize) -> String {
    let inner = inner.trim();
    let (name, fallback) = match inner.find(',') {
        Some(i) => (inner[..i].trim(), Some(inner[i + 1..].trim())),
        None => (inner, None),
    };
    if let Some(v) = aliases.get(name).or_else(|| theme.get(name)) {
        return v.clone();
    }
    if let Some(fb) = fallback {
        if depth < 4 {
            if let Some(stripped) = fb.strip_prefix("var(").and_then(|s| s.strip_suffix(')')) {
                return resolve_var_ref(stripped, theme, aliases, depth + 1);
            }
        }
        return fb.to_string();
    }
    format!("var({inner})")
}

fn substitute_vars(svg: &str, theme: &HashMap<String, String>, aliases: &HashMap<String, String>) -> String {
    let mut out = String::with_capacity(svg.len());
    let mut rest = svg;
    while let Some(i) = rest.find("var(--") {
        out.push_str(&rest[..i]);
        let after = &rest[i + 4..];
        let mut depth = 1;
        let mut end = None;
        for (j, c) in after.char_indices() {
            match c {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(j);
                        break;
                    }
                }
                _ => {}
            }
        }
        match end {
            Some(j) => {
                out.push_str(&resolve_var_ref(&after[..j], theme, aliases, 0));
                rest = &after[j + 1..];
            }
            None => {
                out.push_str(&rest[i..]);
                return out;
            }
        }
    }
    out.push_str(rest);
    out
}

fn resolve_svg(svg: &str, theme: Theme) -> Result<String> {
    let theme_map: HashMap<String, String> = theme.vars().iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    let svg_start = svg.find("<svg").context("the rendered diagram has no <svg> root")?;
    let tag_end = svg[svg_start..].find('>').context("the rendered diagram has a truncated <svg> tag")? + svg_start;
    let root_tag = &svg[svg_start..tag_end];
    let style_attr = root_tag.find("style=\"").map(|i| {
        let from = i + 7;
        let to = root_tag[from..].find('"').map(|j| from + j).unwrap_or(root_tag.len());
        (i, to + 1, &root_tag[from..to])
    });
    let mut aliases = HashMap::new();
    let mut font_size = None;
    let mut font_family = None;
    if let Some((_, _, style)) = &style_attr {
        for decl in style.split(';') {
            let Some(colon) = decl.find(':') else { continue };
            let (key, value) = (decl[..colon].trim(), decl[colon + 1..].trim());
            if let Some(name) = key.strip_prefix("--") {
                let resolved = if value.starts_with("var(") && value.ends_with(')') {
                    resolve_var_ref(&value[4..value.len() - 1], &theme_map, &aliases, 0)
                } else {
                    value.to_string()
                };
                aliases.insert(format!("--{name}"), resolved);
            } else if key == "font" {
                for tok in value.split_whitespace() {
                    if tok.ends_with("px") {
                        font_size = Some(tok.trim_end_matches("px").to_string());
                    }
                }
                if font_family.is_none() {
                    if let Some(v) = value.split_whitespace().collect::<Vec<_>>().split_first() {
                        let fam = v.1.join(" ");
                        if !fam.is_empty() {
                            font_family = Some(fam);
                        }
                    }
                }
            } else if key == "font-family" {
                let resolved = if value.starts_with("var(") && value.ends_with(')') {
                    resolve_var_ref(&value[4..value.len() - 1], &theme_map, &aliases, 0)
                } else {
                    value.to_string()
                };
                font_family = Some(resolved);
            }
        }
    }
    let mut root_new = root_tag.to_string();
    if let Some((from, to, _)) = &style_attr {
        root_new.replace_range(*from..*to, "");
    }
    if let Some(size) = font_size {
        root_new.insert_str(4, &format!(" font-size=\"{size}\""));
    }
    if let Some(fam) = &font_family {
        let expanded = fam
            .split(',')
            .map(|f| {
                let f = f.trim();
                match f {
                    "sans-serif" => "Arial, Helvetica, Liberation Sans, DejaVu Sans, Noto Sans, sans-serif".to_string(),
                    "monospace" => "Courier New, Menlo, Liberation Mono, DejaVu Sans Mono, Noto Sans Mono, monospace".to_string(),
                    "serif" => "Times New Roman, Liberation Serif, DejaVu Serif, Noto Serif, serif".to_string(),
                    other => other.to_string(),
                }
            })
            .collect::<Vec<_>>()
            .join(", ");
        root_new.insert_str(4, &format!(" font-family=\"{expanded}\""));
    }
    let mut out = String::with_capacity(svg.len() + 64);
    out.push_str(&svg[..svg_start]);
    out.push_str(&root_new);
    out.push_str(&svg[tag_end..]);
    let out = substitute_vars(&out, &theme_map, &aliases);
    let mut final_out = String::with_capacity(out.len());
    let mut rest = out.as_str();
    while let Some(i) = rest.find("style=\"rx:") {
        final_out.push_str(&rest[..i]);
        let after = &rest[i + 10..];
        let close = after.find('"').context("unterminated rx style")?;
        final_out.push_str(&format!("rx=\"{}\"", &after[..close]));
        rest = &after[close + 1..];
    }
    final_out.push_str(rest);
    if final_out.contains("var(--") {
        bail!("the rendered diagram still has unresolved CSS variables");
    }
    Ok(final_out)
}

pub fn draw(a: &DrawDiagramArgs) -> Result<DrawnDiagram> {
    let (raw, diagram_type) = pele_render(&a.source)?;
    let resolved = resolve_svg(&raw, a.theme)?;
    let svg_path = a.output.with_extension("svg");
    if let Some(dir) = svg_path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    std::fs::write(&svg_path, &resolved).with_context(|| format!("writing {}", svg_path.display()))?;
    let motion = animate::animate(&AnimateSvgArgs {
        svg_path: svg_path.clone(),
        animation: a.animation,
        duration_s: a.duration_s,
        fps: a.fps,
        width: a.width,
        easing: a.easing,
        hold_s: a.hold_s,
        exit: a.exit,
        exit_s: a.exit_s,
        codec: a.codec,
        output: a.output.clone(),
    })?;
    Ok(DrawnDiagram { motion, svg: svg_path, diagram_type })
}
