# Operating Reelsmith (for agents and new maintainers)

Reelsmith turns a folder of clips plus a short brief into vertical / horizontal promo videos. It ships two binaries
from one Rust workspace:

- `reelsmith`: an MCP server over stdio (official Rust SDK `rmcp`); an AI host lists clips, writes the brief and renders.
- `promo`: the same pipeline as a CLI (`config → plan → plates → audio → chunked render → mux → SRT → QA`).

Both produce the same pixels from the same config (verified byte-identical, see README "Verified"). Read this file
first, then [`docs/LESSONS.md`](docs/LESSONS.md) (what earlier video work taught: pitfalls, the owner's taste,
checklists) before making a video or changing the pipeline.

## Repository map

| Path | What |
|---|---|
| `crates/reelsmith` | MCP server: tools, briefs, render jobs, sample library, `animate_svg`, `draw_diagram`, `stylize_clip` |
| `crates/promo` | template library + `promo` CLI: config, plan, plates, audio, render (fframes or HyperFrames), encode, QA, preview gate, ledger, telemetry |
| `crates/datavid` | media layer: SVG helpers, easing, procedural audio + EBU R128 mastering, ffmpeg/ffprobe helpers, QA |
| `docs/` | `PROMO_VIDEOS.md` (config reference), `MOTION_DIRECTION.md`, `PROMO_COPY_DESIGN.md`, `EVALUATION.md`, `LESSONS.md`, `TEARDOWN.md`, `TESSERACT.md` |
| `PERFORMANCE.md`, `DEMO.md` | measured timings and GPU setup; a 3-minute MCP demo script |
| `examples/` | `sample/` (procedural media, complete brief), `logo-intro/` (`animate_svg` intro + end card) |
| `evaluations/` | reference renders: expected QA values + `check.py` |
| `scripts/` | `mcp_call.py` (drive the MCP server without a host), `compare_renders.py` (SSIM/PSNR/loudness of two renders), `tesseract-preview.py` |
| `.agents/skills/` | project-local skills: `teardown` (technical illustrations), `simready` (3D asset sources) |
| `server/` | optional render-telemetry API (Cloudflare Worker) |

## Build

Requirements: Rust stable (edition 2024), `clang` + `libclang-dev` (bindgen), `pkg-config`, `libfontconfig-dev` and
the **FFmpeg CLI on PATH** (plates, encode, mux and QA call it).

```sh
cargo build --release --locked -p promo        # build one at a time: keeps the link's peak memory low
cargo build --release --locked -p reelsmith
```

- Linux / macOS link a static portable FFmpeg that fframes downloads; Windows needs `FFMPEG_DIR` pointing at a shared
  FFmpeg 9.0 build (see README "Build from source" and `.github/workflows/release.yml`).
- A first build takes several minutes. If your shell has a short timeout, run it detached
  (`setsid nohup cargo build --release -p promo > build.log 2>&1 &`) and poll the log.
- On a machine with ~8 GB of RAM the `reelsmith` link can be OOM-killed. Then validate with the CI artifact instead:
  every CI run uploads `reelsmith-<target>` (both binaries) for 7 days. See `docs/LESSONS.md` § Build and environment.

## Run

**As an MCP server.** Register the absolute path of `reelsmith` in the host (Claude Desktop, Codex, OpenCode configs in
the README); raise the host's tool timeout (Codex: `tool_timeout_sec = 900`). Without a host:

```sh
python3 scripts/mcp_call.py target/release/reelsmith --list
python3 scripts/mcp_call.py target/release/reelsmith make_sample_clips '{"folder": "/tmp/demo/media"}'
python3 scripts/mcp_call.py target/release/reelsmith --steps steps.json --wait-render --transcript transcript.json
```

`steps.json` is a JSON list of `{"tool": ..., "arguments": {...}}`; `--wait-render` polls `get_status` while a render
job is running. The usual tool order: `system_info` → `list_clips` (or `make_sample_clips`) → `create_project` →
`write_brief` → `render(preview)` → review → `render(draft)` → owner approval → `render(final)`.

**As a CLI.** `promo check CONFIG` (seconds, lists missing files), `promo plan CONFIG` (resolved timeline as JSON),
`promo build CONFIG --preview` / default 720p draft / `--final`. `--json` gives NDJSON progress for apps; exit codes:
`0` ok, `1` failed (retryable), `2` bad config, `3` rendered but QA failed, `4` missing input. Full reference:
[`docs/PROMO_VIDEOS.md`](docs/PROMO_VIDEOS.md).

## Render gates and QA (do not bypass)

1. **Preview first.** `draft` and `final` refuse to run without a completed 360p preview of the same config and inputs
   (`<config>.preview.json`). Editing the brief or replacing an input invalidates it.
2. **QA runs on every render:** duration within one frame, size, fps, frame count, audio stream, −14 LUFS ± 0.5, true
   peak ≤ −1 dBTP, black frames, still-photo shots (`min_motion`), minimum caption size when burned in, contact sheet.
   Results in `build/<name>/qa/` (`QA.md`, `qa.json`, `contact.png`).
3. **Look at the contact sheet yourself** before showing anyone a render. QA numbers do not catch clipped text, empty
   cut frames or a wrong URL.
4. **Resolution policy:** iterate at 360p, show the owner 720p drafts, render 1080p only after the owner explicitly
   approves the cut. Never upload or publish a video until the owner says it is finished.
5. **Pipeline changes** are checked against the reference renders in [`docs/EVALUATION.md`](docs/EVALUATION.md)
   (`python3 evaluations/sample-ad/check.py`), and a change that should not alter pixels is proven with
   `python3 scripts/compare_renders.py before.mp4 after.mp4` (SSIM 1.0 / PSNR inf when identical).

## Engines and assets

- `fframes` (default): clip-based promos. HyperFrames (`[render] engine = "hyperframes"` or the `engine` argument):
  HTML/CSS/GSAP compositions rendered frame by frame; needs Node.js 22+ with `npx`, and does not support alpha motion
  overlays yet.
- Overlays: `animate_svg` (SVG → alpha `.mov`), `draw_diagram` (Mermaid → self-drawing `.mov`), `stylize_clip` (ASCII /
  dither clips). Put results in the project's motion (or clips) folder and reference them by `asset_name`.
- Technical illustrations: read `.agents/skills/teardown/SKILL.md` and its linked references; the handoff is in
  `docs/TEARDOWN.md`. The skill creates SVG/PNG/PDF assets, not a new MCP tool: export an SVG, animate it with
  `animate_svg`, then use the normal project / brief / render flow. Always render and inspect a 360p preview before
  requesting 720p or 1080p output.
- `simready`: optional guide for using an NVIDIA SimReady OpenUSD asset as a rendered video source
  (`.agents/skills/simready/SKILL.md`). Low priority; no native USD support.
- Motion direction: for briefs that should move (continuity, action cuts, draw-on overlays), read
  `docs/MOTION_DIRECTION.md`. It uses the existing tools; no new endpoint.
- Copy: `docs/PROMO_COPY_DESIGN.md` (hook, benefits, proof, CTA). Tesseract preview bridge: `docs/TESSERACT.md`.

## Contributing rules (the owner's)

- Everything in the repository is in **English**: code, comments, commit messages, docs. On-screen video text may be in
  any language.
- Never push to `main`. Branches: `feat/`, `fix/`, `chore/`, `docs/`, `perf/`; agents use `agent/<topic>`.
- Commits: `type: brief description` (`feat`, `fix`, `chore`, `docs`, `perf`, `refactor`). One concern per PR.
- PR body: **What / Why / Impact / Validation**, with real measured numbers only (never invented timings).
- A PR is merged only with a clean merge status (no conflicts) and green checks (`ci` builds both binaries on x86_64
  and aarch64 Linux and runs an MCP `tools/list` smoke check; `release` runs when its workflow file changes).
- **Never commit media or secrets**: no clips, motion graphics, renders, audio, SRTs, thumbnails or build outputs
  (`.gitignore` blocks them), no client material, tokens or personal data. A reference must be reproducible from
  committed sources plus a documented generation step.
- Prefer a short 360p smoke render over adding unit / integration test suites to the render path; keep unit tests for
  parsers and checks that need them.
- When something new is learned (a bug, a fix, a measured number, owner feedback on a draft), add it to
  `docs/LESSONS.md`.
