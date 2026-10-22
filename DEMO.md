# Reelsmith: 3-minute demo

A live script: what to type in the host, which tool runs, and what it returns. It uses the procedural sample library, so
it needs no footage; the last step swaps in a real clips folder. Measured times are in [Reference numbers](#reference-numbers);
on an NVIDIA GPU the delivery MP4 is encoded with NVENC (`"encoder": "nvenc"`), elsewhere with libx264 (`"x264"`).

**Before the demo** (once): FFmpeg on PATH, `reelsmith` registered in the host (see [README](README.md#configure-your-mcp-host)),
host restarted. In Codex keep `tool_timeout_sec = 900`. Pick a demo folder, here `C:\reelsmith-demo` (macOS/Linux:
`~/reelsmith-demo`).

---

### 0:00 — What the host can do (10 s)

> **Type:** `What tools does reelsmith give you?`

The host lists the tools: `list_clips`, `probe_clip`, `list_templates`, `make_sample_clips`, `create_project`,
`write_brief`, `render`, `get_status`, `get_qa`, `system_info`.

### 0:10 — A clip library (20 s)

> **Type:** `Use reelsmith to make sample clips in C:\reelsmith-demo\media and list them.`

- `make_sample_clips` → `{"folder": "C:\\reelsmith-demo\\media", "clips": [...]}` with six files:
  `aurora`, `cells`, `grid`, `nebula` (role `clip`, 1280×720, 30 fps, 6 s) and `badge`, `cta`
  (role `motion`: they have an alpha channel, so they are overlays / end card).

With real footage, replace this step with: `List the clips in D:\Footage\raw` (`list_clips`) — the host sees every
recording's duration and resolution and picks in-points from that.

### 0:30 — The project and the brief (40 s)

> **Type:**
> ```
> Create a reelsmith project in C:\reelsmith-demo\ad with those clips, 9:16, procedural music, captions off.
> Then write a brief called sample_ad with 4 beats:
> 1. "WHAT IF | YOUR SERVER | HAD A TRAILER?" on nebula from 0.5 s, 2.6 s long, with the badge overlay near the top
> 2. "NEW WORLDS | EVERY WEEK" on aurora from 1 s with a flash and a push-in from 1.05 to 1.18
> 3. "BUILD | WITH FRIENDS" on cells (1 s from 0.5 s) then grid from 2 s
> 4. "JOIN | TONIGHT" on nebula from 3 s with a flash and a braam
> End card: aurora from 2 s with the cta asset, 3.5 s, text "SEE YOU THERE".
> ```

- `create_project` → `{"style_file": "...\\ad\\promo.toml", "out_dir": "...\\ad\\out", ...}`
- `write_brief` → `{"config": "...\\ad\\brief.toml", "plan": {"seconds": 9.733, "frames": 292,
  "width": 360, "height": 640, "shots": 7, "overlays": 2, "captions": 10,
  "captions_burned_in": false, "music": "procedural", ...}, "missing": []}`

`missing: []` means every clip and asset was found; a typo shows up here (`"clip ...\\media\\nebul.mp4"`) before any
render starts.

### 1:10 — Preview at 360p (40 s)

> **Type:** `Render a preview.`

- `render(config, resolution: "preview")` → `{"state": "succeeded", "elapsed_s": 16.4, "result":
  {"output": "...\\ad\\out\\sample_ad_preview_360x640.mp4", "srt": "...\\ad\\out\\sample_ad.srt", "encoder": "x264",
  "qa": {"ok": true, "duration_s": 9.733, "width": 360, "height": 640, "integrated_lufs": -14.02,
  "true_peak_dbtp": -2.0, "black_frames": [], "failures": [], "contact_sheet": "...\\qa\\contact.png"}}}`

If the host's timeout is short, the first answer is `{"state": "running", "job_id": "job-1"}` and the host follows up
with `get_status("job-1")` — same result.

Open the MP4 (and `qa\contact.png`, a 6×4 sheet of the whole ad) and play it.

### 1:50 — Iterate in words (40 s)

> **Type:** `Make beat 2 shorter, 1.5 s, and swap the end card clip to grid. Render the preview again.`

The host calls `write_brief` again with the change and `render`. Only the touched shots are re-cut and only the changed
chunks re-render, so the second preview is faster.

### 2:30 — Draft and final (30 s)

> **Type:** `Looks good. Render the 720p draft.`  … then … `Approved, render the final.`

- `render(resolution: "draft")` → `out\sample_ad_draft_720x1280.mp4` with the QA block.
- `render(resolution: "final")` → `out\sample_ad_final_1080x1920.mp4` (1080×1920, −14 LUFS ± 0.5, true peak ≤ −1 dBTP).

### 2:50 — Real footage (10 s)

Point the same flow at the real recordings and motion graphics:

> **Type:** `Create a project in D:\Footage\ad5 with clips from D:\Footage\raw and motion graphics from D:\Footage\motion, then
> write a brief from this script: …`

---

## Prompts per host

The prompts above work as typed in all three hosts. Host-specific notes:

| Host | Notes |
|---|---|
| Claude Desktop / Cowork | Approve the reelsmith tools when asked (or "Allow always"). Claude usually chains `create_project` → `write_brief` → `render` by itself from one message. |
| Codex | `tool_timeout_sec = 900` in `~/.codex/config.toml` lets a final render return in one call. Start with `codex` in any folder; prompts are the same. |
| OpenCode (e.g. DeepSeek) | Mention the server once ("use the reelsmith tools") so a smaller model picks them; paste the beats as a numbered list as above. |

### Exact opening prompt per host

Paste one message to start the demo; everything after it is the same in every host.

- **Claude Desktop / Cowork:**
  `Use the reelsmith tools. Make sample clips in C:\reelsmith-demo\media, create a 9:16 project in C:\reelsmith-demo\ad with them (procedural music, captions off), write a brief called sample_ad from the 4 beats below and render a preview.` (then the numbered beats from 0:30)
- **Codex** (`codex` in any folder, `tool_timeout_sec = 900`):
  `Using only the reelsmith MCP tools (no shell), make sample clips in C:\reelsmith-demo\media, create a 9:16 project in C:\reelsmith-demo\ad, write the brief sample_ad from these beats and render a preview. Show me the QA block.` (then the beats)
- **OpenCode** (e.g. `deepseek-flash`):
  `Call the reelsmith tools step by step: 1) make_sample_clips folder C:\reelsmith-demo\media 2) create_project project_dir C:\reelsmith-demo\ad, clips_folder C:\reelsmith-demo\media, aspect 9:16, music procedural 3) write_brief name sample_ad with these beats 4) render resolution preview.` (then the beats)

## Reference numbers

Measured on 2026-10-22 with the CI-built Linux arm64 binaries on a 2-vCPU machine without a GPU (encoder: libx264).
An RTX-class GPU with NVENC and a desktop CPU are faster; treat these as upper bounds.

| Run | Output | Time | QA |
|---|---|---|---|
| sample library: `make_sample_clips` | 6 files | 15.9 s | — |
| sample ad, `render` preview | 360×640, 9.733 s, 292 frames | 16.4 s | PASS, −14.02 LUFS, −2.0 dBTP |
| real 22.5 s ad (30 clips, 7 motion graphics), `render` preview | 360×640, 675 frames | 62 s | PASS, −14.04 LUFS, −1.49 dBTP |
| same ad, `render` draft | 720×1280, 675 frames | 146 s | PASS, −14.04 LUFS, −1.49 dBTP |
| same ad, `promo build --preview` (CLI) | 360×640, 675 frames | 76 s | byte-identical to the MCP preview |

`list_clips` over 30 recordings takes about 8 s the first time (one `ffprobe` per file).

In the sample the host's `elapsed_s` and the encoder differ per machine; the plan numbers (`seconds`, `frames`,
`shots`, `captions`) are exact.
