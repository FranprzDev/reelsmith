# Memory guide

This folder is the agent's persistent memory across videos (a LOAD/OFFLOAD memory harness: what
was learned survives the chat it was learned in). LOAD it before authoring a video, OFFLOAD to
it after each render or review round.

## LOAD (before write_brief)

1. Call `memory_load` with the project folder. Read this guide and the lessons it returns.
2. Let the lessons shape the brief: apply the fixes, avoid the repeated mistakes, reuse the
   settings that measured well.
3. When you are hunting something specific ("captions", "music", a client's name), pass it as
   `query` instead of reading everything.

## OFFLOAD (after each render and each owner correction)

Call `memory_offload` with:

- `title`: the problem or win in a few words ("glow overlay washed out the caption").
- `lesson`: symptom, cause, fix. Include a measured number when there is one (render seconds,
  QA loudness, words on screen). Never write a number nobody measured.
- `tags`: a few lowercase words to filter by later.
- `context`: the brief or config name, so the lesson can be traced back.

Offload right after it happens, not at the end of the chat: the context may be cleared in
between.

## Rewrite this guide (discover)

This file is yours to improve. When lessons repeat, contradict each other, or `memory_load`
stops helping, read the lessons and the render history (`renders.jsonl` next to each config),
decide what should have been remembered earlier, and rewrite this guide with `memory_guide`:
what to load, what to offload, how to organize it. Keep it short: it comes back on every load.

## Sharing one memory across projects

Each project keeps its memory in `<project>/memory`. To share one memory across several
projects, pass the same existing folder as `project` to the memory tools.
