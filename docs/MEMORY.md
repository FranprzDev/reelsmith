# Agent memory (LOAD/OFFLOAD)

Reelsmith ships a persistent memory for the agent that authors the videos, modeled on
["Self-Improving Memory: Meta-Optimization and Harness Distillation for Continual Learning Agents"](https://facebookresearch.github.io/RAM/blogs/self_improving_memory/)
(Chen et al., Meta AI, 2026). The paper's finding: an agent gets better at a recurring task
when what it learned survives the chat it was learned in, and when the *strategy* for what to
remember is itself improved from the agent's own runs instead of hand-written once.

The paper has three steps; the first two map onto Reelsmith directly:

1. **LOAD / OFFLOAD harness** → the `memory_load` / `memory_offload` tools. Before authoring a
   brief the agent loads the memory; after each render or owner correction it offloads a lesson.
2. **Harness optimization (discover)** → the `memory_guide` tool. The memory guide
   (`GUIDE.md`, seeded on first use) is the "memory prompt". The agent rewrites it from the
   saved lessons and the render history (`renders.jsonl` next to each config) when the lessons
   show a better strategy, which is the paper's discover loop run by the agent itself.
3. **Distillation** (train the strategy into model weights) needs fine-tuning infrastructure and
   is out of scope for a local harness.

`docs/LESSONS.md` stays what it is: the hand-written, curated record of the first projects. The
memory is the live, per-install complement the agent maintains on its own from then on.

## Layout

```
<project>/memory/
  GUIDE.md            the standing strategy: what to load, what to offload (rewritable with memory_guide)
  lessons/
    2026-11-18-glow-overlay-washed-out-the-caption.md
    ...
```

One Markdown file per lesson, with a small front matter (`date`, `title`, `tags`, `context`) so
files stay readable and editable by hand. `memory_load` parses the front matter and returns the
guide plus the matching lessons, newest first.

## What a good lesson looks like

- **Symptom, cause, fix.** "Caption unreadable over the glow overlay; the bloom lifted the
  background behind the text; moved the caption 40 px below the overlay's hold."
- **A measured number when there is one.** Render seconds, QA loudness, words on screen. Never
  write a number nobody measured (the same rule as `docs/LESSONS.md`).
- **Traceable context.** The brief or config name, so the lesson can be re-checked against the
  render it came from.

## Sharing one memory across projects

Each project keeps its memory in `<project>/memory`. To build one memory across many projects
(closer to the paper's setup, where the memory directory is carried across the whole task
stream), pass the same existing folder as `project` to the three memory tools.

## Validation

Unit tests in `crates/reelsmith/src/memory.rs` cover the date conversion, slugging, the
offload → load roundtrip, same-day title collisions, the query filter and the guide rewrite.
The CI smoke check lists the three tools over MCP.
