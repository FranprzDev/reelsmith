//! Agent memory: a persistent, file-based memory the agent LOADs before authoring a video and OFFLOADs to after
//! each render or review round. It implements the memory harness of "Self-Improving Memory" (Chen et al., 2026)
//! for the video task: the memory directory lives next to the project, survives the chat that wrote it, and the
//! standing strategy (`GUIDE.md`, seeded from `templates/memory/GUIDE.md`) is the agent's to rewrite with
//! `memory_guide` when the saved lessons show a better one (the paper's discover step, done by the agent itself).
//! Weight distillation (fine-tuning on the optimized runs) is out of scope for a local harness.
//!
//! Layout: `<project>/memory/GUIDE.md` plus one Markdown file per lesson under `<project>/memory/lessons/`,
//! each with a small front matter (date, title, tags, context) so files stay readable and editable by hand.
use anyhow::{Context, Result, bail};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const GUIDE_SEED: &str = include_str!("../../../templates/memory/GUIDE.md");

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MemoryLoadArgs {
    /// the project folder (its memory lives in `<project>/memory`), or any existing folder chosen as a shared
    /// memory root for several projects
    pub project: PathBuf,
    /// optional filter: only lessons whose title, tags, context and text together contain every word
    /// (case-insensitive)
    pub query: Option<String>,
    /// how many lessons to return, newest first (default 20)
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MemoryOffloadArgs {
    /// the project folder (its memory lives in `<project>/memory`), or the shared memory root
    pub project: PathBuf,
    /// the problem or win in a few words ("glow overlay washed out the caption")
    pub title: String,
    /// the lesson: symptom, cause, fix, and a measured number when there is one (render seconds, QA values).
    /// Never write a number nobody measured.
    pub lesson: String,
    /// a few lowercase words to filter by later (e.g. ["captions", "music"])
    pub tags: Option<Vec<String>>,
    /// the brief or config name the lesson came from, so it can be traced back
    pub context: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MemoryGuideArgs {
    /// the project folder (its memory lives in `<project>/memory`), or the shared memory root
    pub project: PathBuf,
    /// the new memory strategy (Markdown). Read the saved lessons and the render history (renders.jsonl next to
    /// each config) first, keep it short: memory_load returns it on every call.
    pub guide: String,
}

/// One saved lesson.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Lesson {
    /// file name inside `<project>/memory/lessons/`
    pub file: String,
    /// the day it was saved (UTC)
    pub date: String,
    pub title: String,
    pub lesson: String,
    pub tags: Vec<String>,
    pub context: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct MemoryLoad {
    pub memory_dir: PathBuf,
    /// the standing memory strategy (seeded on first use, rewritten with memory_guide)
    pub guide: String,
    /// matching lessons, newest first
    pub lessons: Vec<Lesson>,
    /// every lesson stored, before filtering and limiting
    pub total_lessons: usize,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct LessonSaved {
    pub file: PathBuf,
    pub total_lessons: usize,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct GuideSaved {
    pub file: PathBuf,
}

fn memory_dir(project: &Path) -> Result<PathBuf> {
    anyhow::ensure!(project.is_dir(), "no such folder: {} (create the project first)", project.display());
    Ok(project.join("memory"))
}

/// Create `<project>/memory` and seed GUIDE.md on first use.
fn ensure_memory(project: &Path) -> Result<PathBuf> {
    let dir = memory_dir(project)?;
    let lessons = dir.join("lessons");
    std::fs::create_dir_all(&lessons).with_context(|| format!("creating {}", lessons.display()))?;
    let guide = dir.join("GUIDE.md");
    if !guide.exists() {
        std::fs::write(&guide, GUIDE_SEED).with_context(|| format!("writing {}", guide.display()))?;
    }
    Ok(dir)
}

/// Today's date (UTC) as YYYY-MM-DD, from the system clock.
fn today_utc() -> String {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
    civil_from_days((secs / 86_400) as i64)
}

/// The civil date of a unix day count (Howard Hinnant's civil-from-days).
fn civil_from_days(days: i64) -> String {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

/// A file-safe slug of the title, capped so paths stay short.
fn slug(title: &str) -> String {
    let mut s = String::new();
    for c in title.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            s.push(c);
        } else if !s.is_empty() && !s.ends_with('-') {
            s.push('-');
        }
    }
    let mut s: String = s.trim_matches('-').chars().take(48).collect();
    while s.ends_with('-') {
        s.pop();
    }
    if s.is_empty() { "lesson".into() } else { s }
}

fn lesson_markdown(date: &str, a: &MemoryOffloadArgs) -> String {
    let tags = a.tags.clone().unwrap_or_default().join(", ");
    let mut out = format!("---\ndate: {date}\ntitle: {}\ntags: [{tags}]\n", a.title.trim());
    if let Some(c) = &a.context {
        out.push_str(&format!("context: {}\n", c.trim()));
    }
    out.push_str("---\n\n");
    out.push_str(a.lesson.trim());
    out.push('\n');
    out
}

/// Parse one lesson file: a small front matter (date / title / tags / context between `---` lines) and the body.
fn parse_lesson(file: &str, text: &str) -> Option<Lesson> {
    let mut lines = text.lines();
    if lines.next()?.trim() != "---" {
        return None;
    }
    let (mut date, mut title, mut tags, mut context) = (String::new(), String::new(), Vec::new(), None);
    for line in lines.by_ref() {
        let line = line.trim_end();
        if line.trim() == "---" {
            break;
        }
        let (key, value) = line.split_once(':')?;
        let value = value.trim();
        match key.trim() {
            "date" => date = value.to_string(),
            "title" => title = value.to_string(),
            "tags" => {
                tags = value
                    .trim_start_matches('[')
                    .trim_end_matches(']')
                    .split(',')
                    .map(str::trim)
                    .filter(|t| !t.is_empty())
                    .map(String::from)
                    .collect();
            }
            "context" => context = Some(value.to_string()),
            _ => {}
        }
    }
    let lesson = lines.collect::<Vec<_>>().join("\n").trim().to_string();
    if title.is_empty() || lesson.is_empty() {
        return None;
    }
    Some(Lesson { file: file.to_string(), date, title, lesson, tags, context })
}

/// Every stored lesson, newest first (date, then file name).
fn read_lessons(dir: &Path) -> Result<Vec<Lesson>> {
    let lessons_dir = dir.join("lessons");
    let mut lessons = Vec::new();
    for entry in std::fs::read_dir(&lessons_dir).with_context(|| format!("reading {}", lessons_dir.display()))? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.ends_with(".md") {
            continue;
        }
        if let Ok(text) = std::fs::read_to_string(entry.path()) {
            if let Some(lesson) = parse_lesson(&name, &text) {
                lessons.push(lesson);
            }
        }
    }
    lessons.sort_by(|a, b| b.date.cmp(&a.date).then_with(|| b.file.cmp(&a.file)));
    Ok(lessons)
}

fn matches(lesson: &Lesson, query: &str) -> bool {
    let hay = format!(
        "{} {} {} {}",
        lesson.title,
        lesson.lesson,
        lesson.tags.join(" "),
        lesson.context.as_deref().unwrap_or_default()
    )
    .to_lowercase();
    query.split_whitespace().all(|w| hay.contains(&w.to_lowercase()))
}

pub fn load(a: &MemoryLoadArgs) -> Result<MemoryLoad> {
    let dir = ensure_memory(&a.project)?;
    let guide = std::fs::read_to_string(dir.join("GUIDE.md")).context("reading GUIDE.md")?;
    let all = read_lessons(&dir)?;
    let total = all.len();
    let limit = a.limit.unwrap_or(20);
    let lessons = match &a.query {
        Some(q) if !q.trim().is_empty() => all.into_iter().filter(|l| matches(l, q)).take(limit).collect(),
        _ => all.into_iter().take(limit).collect(),
    };
    Ok(MemoryLoad { memory_dir: dir, guide, lessons, total_lessons: total })
}

pub fn offload(a: &MemoryOffloadArgs) -> Result<LessonSaved> {
    if a.title.trim().is_empty() || a.lesson.trim().is_empty() {
        bail!("title and lesson are required");
    }
    let dir = ensure_memory(&a.project)?;
    let date = today_utc();
    let base = format!("{date}-{}", slug(&a.title));
    let lessons_dir = dir.join("lessons");
    let mut name = format!("{base}.md");
    for n in 2.. {
        if !lessons_dir.join(&name).exists() {
            break;
        }
        name = format!("{base}-{n}.md");
    }
    let path = lessons_dir.join(&name);
    std::fs::write(&path, lesson_markdown(&date, a)).with_context(|| format!("writing {}", path.display()))?;
    let total = read_lessons(&dir)?.len();
    Ok(LessonSaved { file: path, total_lessons: total })
}

pub fn write_guide(a: &MemoryGuideArgs) -> Result<GuideSaved> {
    if a.guide.trim().is_empty() {
        bail!("the guide cannot be empty");
    }
    let dir = ensure_memory(&a.project)?;
    let path = dir.join("GUIDE.md");
    std::fs::write(&path, format!("{}\n", a.guide.trim())).with_context(|| format!("writing {}", path.display()))?;
    Ok(GuideSaved { file: path })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("reelsmith-memory-{tag}-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn offload_args(project: &Path, title: &str, lesson: &str) -> MemoryOffloadArgs {
        MemoryOffloadArgs {
            project: project.to_path_buf(),
            title: title.into(),
            lesson: lesson.into(),
            tags: Some(vec!["captions".into(), "music".into()]),
            context: Some("sample".into()),
        }
    }

    #[test]
    fn dates_look_like_dates() {
        let d = today_utc();
        assert_eq!(d.len(), 10);
        assert_eq!(&d[4..5], "-");
        assert_eq!(&d[7..8], "-");
    }

    #[test]
    fn civil_from_days_matches_known_dates() {
        assert_eq!(civil_from_days(0), "1970-01-01");
        assert_eq!(civil_from_days(20_775), "2026-11-18");
        assert_eq!(civil_from_days(18_262), "2020-01-01");
        assert_eq!(civil_from_days(18_322), "2020-03-01"); // after Feb 29 of a leap year
    }

    #[test]
    fn slugs_are_safe() {
        assert_eq!(slug("Glow overlay washed out!"), "glow-overlay-washed-out");
        assert_eq!(slug("  --weird//  "), "weird");
        assert_eq!(slug("!!!"), "lesson");
        assert!(slug(&"a very long title ".repeat(20)).len() <= 48);
    }

    #[test]
    fn offload_then_load_roundtrip() {
        let dir = tmp("roundtrip");
        let saved = offload(&offload_args(&dir, "Caption too small", "48 px minimum on the 1080p canvas.")).unwrap();
        assert!(saved.file.is_file());
        assert_eq!(saved.total_lessons, 1);
        let loaded = load(&MemoryLoadArgs { project: dir.clone(), query: None, limit: None }).unwrap();
        assert!(loaded.guide.contains("LOAD (before write_brief)"), "the guide is seeded on first use");
        assert_eq!(loaded.total_lessons, 1);
        assert_eq!(loaded.lessons[0].title, "Caption too small");
        assert_eq!(loaded.lessons[0].tags, vec!["captions", "music"]);
        assert_eq!(loaded.lessons[0].context.as_deref(), Some("sample"));
        assert_eq!(loaded.lessons[0].lesson, "48 px minimum on the 1080p canvas.");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn same_day_titles_do_not_clobber() {
        let dir = tmp("clobber");
        offload(&offload_args(&dir, "Same title", "first")).unwrap();
        let second = offload(&offload_args(&dir, "Same title", "second")).unwrap();
        assert!(second.file.file_name().unwrap().to_string_lossy().ends_with("-2.md"));
        assert_eq!(second.total_lessons, 2);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn query_needs_every_word() {
        let dir = tmp("query");
        offload(&offload_args(&dir, "Caption too small", "Use at least 48 px.")).unwrap();
        offload(&MemoryOffloadArgs {
            project: dir.clone(),
            title: "Music ended early".into(),
            lesson: "Keep the music bed to the last frame.".into(),
            tags: None,
            context: None,
        })
        .unwrap();
        let load_args = |q: &str| MemoryLoadArgs { project: dir.clone(), query: Some(q.into()), limit: None };
        assert_eq!(load(&load_args("caption 48")).unwrap().lessons.len(), 1);
        assert_eq!(load(&load_args("CAPTION")).unwrap().lessons.len(), 1);
        assert_eq!(load(&load_args("caption music")).unwrap().lessons.len(), 1, "tags match too");
        assert_eq!(load(&load_args("caption frame")).unwrap().lessons.len(), 0);
        assert_eq!(load(&load_args("  ")).unwrap().lessons.len(), 2, "a blank query lists everything");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn guide_roundtrip_and_seed_on_write() {
        let dir = tmp("guide");
        let saved = write_guide(&MemoryGuideArgs { project: dir.clone(), guide: "# My strategy\n\nLoad first.".into() }).unwrap();
        assert!(saved.file.is_file());
        let loaded = load(&MemoryLoadArgs { project: dir.clone(), query: None, limit: None }).unwrap();
        assert_eq!(loaded.guide, "# My strategy\n\nLoad first.\n");
        assert!(write_guide(&MemoryGuideArgs { project: dir.clone(), guide: "  ".into() }).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn missing_project_is_an_error() {
        let dir = std::env::temp_dir().join(format!("reelsmith-memory-nope-{}", std::process::id()));
        assert!(load(&MemoryLoadArgs { project: dir, query: None, limit: None }).is_err());
    }
}
