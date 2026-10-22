//! QA report: one Markdown file (for people) plus one JSON file (for tools) with the facts checks, the readability table,
//! loudness, duration and the list of QA frames.
use anyhow::Result;
use serde_json::{Value, json};
use std::path::Path;

#[derive(Default)]
pub struct Report {
    pub md: String,
    pub json: serde_json::Map<String, Value>,
    pub failures: Vec<String>,
}

impl Report {
    pub fn new(title: &str) -> Self {
        Self { md: format!("# QA — {title}\n\n"), ..Default::default() }
    }
    pub fn section(&mut self, title: &str, body: &str) {
        self.md.push_str(&format!("## {title}\n\n{body}\n\n"));
    }
    pub fn put(&mut self, key: &str, v: Value) {
        self.json.insert(key.to_string(), v);
    }
    pub fn check(&mut self, name: &str, ok: bool, detail: &str) {
        self.md.push_str(&format!("- {} **{name}**: {detail}\n", if ok { "✅" } else { "❌" }));
        if !ok {
            self.failures.push(format!("{name}: {detail}"));
        }
        let checks = self.json.entry("checks").or_insert_with(|| json!([]));
        checks.as_array_mut().unwrap().push(json!({"name": name, "ok": ok, "detail": detail}));
    }
    pub fn save(&self, dir: &Path) -> Result<()> {
        std::fs::create_dir_all(dir)?;
        std::fs::write(dir.join("QA.md"), &self.md)?;
        let mut j = self.json.clone();
        j.insert("failures".into(), json!(self.failures));
        std::fs::write(dir.join("qa.json"), serde_json::to_string_pretty(&Value::Object(j))?)?;
        Ok(())
    }
}
