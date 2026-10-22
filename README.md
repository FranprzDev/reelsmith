# Reelsmith

Reelsmith is an [MCP](https://modelcontextprotocol.io) server that turns a folder of gameplay clips into short-form promo
videos (Reels / TikTok / Shorts ads). You talk to your AI host (Claude Desktop, Codex, OpenCode or any MCP client); it
lists your clips, writes the brief, renders a 360p preview, a 720p draft and the 1080p final, and checks each render
(duration, size, −14 LUFS loudness, true peak, black frames).

Everything runs locally in one Rust binary, `reelsmith`, on top of a config-driven template rendered with
[fframes](https://fframes.studio): clips with slow push-ins, alpha motion graphics with their own sound effects, white
flashes on the cuts, an end card, a procedural (or your own) music bed mastered to −14 LUFS, and an SRT sidecar.
Burned-in captions are off by default.

## Tools

| Tool | What it does |
|---|---|
| `list_clips(folder)` | every video in a local folder: name, duration, resolution, fps, codec, audio, role (`clip`, or `motion` when it has an alpha channel) |
| `probe_clip(path)` | the same facts for one file |
| `list_templates()` | the commented beats brief and the timed voice-over script that ship in the binary |
| `make_sample_clips(folder)` | a procedural sample library (4 clips + 2 alpha motion graphics, drawn by FFmpeg) to try the flow without footage |
| `create_project(project_dir, clips_folder, motion_folder?, aspect, music, bpm?, music_file?, captions_burned_in)` | writes the project style, `promo.toml` |
| `write_brief(project_dir, name, beats[], end_card?)` | writes `brief.toml` (one beat per idea: text, clip names, optional overlay; `extends` the style) and returns the resolved timeline and any missing files |
| `render(config, resolution, encoder?, captions_burned_in?, out?, wait_seconds?)` | `preview` 360p, `draft` 720p or `final` 1080p; returns the MP4, SRT and QA report, or a `job_id` while it is still running |
| `get_status(job_id, wait_seconds?)` | the state of a render (`running`, `succeeded`, `qa_failed`, `failed`) and its result |
| `get_qa(config, resolution, video?)` | re-run QA on a rendered MP4 |
| `system_info(encoder?)` | the GPU / encoder backend this machine renders with, every probe tried and why it was chosen |

Finite choices are enums in the tool schemas: `aspect` = `9:16` · `16:9` · `1:1`, `music` = `procedural` · `file` ·
`none`, `resolution` = `preview` · `draft` · `final`, `encoder` = `auto` · `nvenc` · `amf` · `qsv` · `vaapi` · `videotoolbox` ·
`x264`,
`cue` = `braam` · `hit` · `whoosh`.

## Install

1. **FFmpeg on PATH** (the plates, the encode, the mux and QA use it):
   - Windows: `winget install Gyan.FFmpeg` (includes NVENC and libx264), then open a new terminal and check `ffmpeg -version`.
   - macOS: `brew install ffmpeg`. Linux: your package manager (`apt install ffmpeg`).
2. **The binary**: download the archive for your OS from the [Releases](https://github.com/FranprzDev/reelsmith/releases)
   page (Windows: `reelsmith-<version>-x86_64-pc-windows-msvc.zip`, which carries the FFmpeg DLLs it links against) and
   unpack it into a folder such as `C:\Tools\reelsmith\` (Windows) or `~/.local/bin/` (macOS / Linux). Until the first
   release is tagged, the same archives are attached to the latest run of the `release` workflow (Actions tab), or build
   from source (below).
3. **Register it with your host** (next section) and restart the host.

Reelsmith detects the GPU by test-opening each hardware encoder with a one-frame FFmpeg encode, in this order:
NVIDIA NVENC (+ CUDA decode), AMD AMF on Windows / VA-API on Linux, Intel QSV on Windows / VA-API on Linux, Apple
VideoToolbox, then libx264 on the CPU. The paired hardware decoder (CUDA, D3D11VA, VA-API, VideoToolbox) cuts the
source clips when it opens, otherwise they are decoded on the CPU. `system_info` and every `render` result report the
backend, each probe and the reason; pass `encoder` to `render` to force one (an error if it cannot open).

## Configure your MCP host

Use the absolute path of the binary. On Windows, escape backslashes in JSON/TOML (`C:\\Tools\\reelsmith\\reelsmith.exe`).

**Claude Desktop** (Settings → Developer → Edit Config, `claude_desktop_config.json`):

```json
{
  "mcpServers": {
    "reelsmith": {
      "command": "C:\\Tools\\reelsmith\\reelsmith.exe",
      "args": []
    }
  }
}
```

**Codex** (`~/.codex/config.toml`; renders can take longer than Codex's 60 s default, so raise the tool timeout):

```toml
[mcp_servers.reelsmith]
command = "C:\\Tools\\reelsmith\\reelsmith.exe"
args = []
startup_timeout_sec = 20
tool_timeout_sec = 900
```

**OpenCode** (`opencode.json` in the project, or `~/.config/opencode/opencode.json`):

```json
{
  "$schema": "https://opencode.ai/config.json",
  "mcp": {
    "reelsmith": {
      "type": "local",
      "command": ["C:\\Tools\\reelsmith\\reelsmith.exe"],
      "enabled": true
    }
  }
}
```

Any other MCP client: run `reelsmith` with no arguments; it speaks JSON-RPC over stdio and logs to stderr.

`render` answers within `wait_seconds` (default 45 s, under every host's default timeout). A longer render returns a
`job_id`; the host then calls `get_status` until the job is done. See [DEMO.md](DEMO.md) for a 3-minute walkthrough.

## Try it without footage

Ask your host: *"Use reelsmith to make sample clips in C:\reelsmith-demo\media, then make a 9:16 promo from them and
render a preview."* The procedural library and the example brief in [`examples/sample/`](examples/sample/) render a
complete 9.7 s ad (292 frames) in under 20 s at 360p on a 2-vCPU machine without a GPU.

## Build from source

Requirements: Rust stable (edition 2024), `clang` (bindgen) and the FFmpeg CLI on PATH.

- **Linux / macOS**: `cargo build --release -p reelsmith`. A static, portable FFmpeg library is downloaded as a
  prebuilt archive and linked into the binary.
- **Windows**: the FFmpeg libraries are linked from `FFMPEG_DIR`: download a *shared* FFmpeg **9.0** build with headers
  and import libraries (for example BtbN's `ffmpeg-n9.0-latest-win64-lgpl-shared-9.0.zip`, as the release workflow does), set `FFMPEG_DIR` to its folder (with `include\` and `lib\`), run
  `cargo build --release -p reelsmith`, and keep the FFmpeg DLLs next to `reelsmith.exe`.

The repository also builds the `promo` CLI (`cargo build --release -p promo`), the same pipeline from the command line;
see [`docs/PROMO_VIDEOS.md`](docs/PROMO_VIDEOS.md) for the config reference.

## Verified

- The MCP path and the CLI path render the same pixels: a 22.5 s, 13-shot vertical ad rendered through `write_brief` +
  `render` is byte-identical to `promo build --preview` on the equivalent hand-written config (SSIM 1.0, 675/675 frames,
  −14.04 LUFS on both).
- Every render runs QA: duration within one frame, size, fps, **video frame count**, audio stream, −14 LUFS ± 0.5,
  true peak ≤ −1 dBTP, black frames, minimum caption size, and a contact sheet.
- Measured render times are in [DEMO.md](DEMO.md#reference-numbers).

## Layout

| Path | What it is |
|---|---|
| `crates/reelsmith` | the MCP server (rmcp, stdio): tools, projects and briefs, render jobs, sample library |
| `crates/promo` | the promo template (library + `promo` CLI): config → plan → plates → audio → chunked fframes render → encode + mux → SRT → QA |
| `crates/datavid` | the media layer: SVG frame helpers, easing, procedural audio and EBU R128 mastering, ffmpeg/ffprobe helpers, QA reports |
| `docs/PROMO_VIDEOS.md` | the config reference and the step-by-step guide |
| `examples/sample/` | a complete brief for the procedural sample library |
| `templates/promo/` | a starting `brief.toml` and a timed voice-over script |
| `fonts/` | Pixelify Sans Bold (OFL), the default caption font, embedded in the binary |

No clips, renders or client material are ever committed: `.gitignore` blocks video, audio and caption files.

## Origin

The `datavid` and `promo` crates are vendored from the harness in
[FranprzDev/donde-arde-argentina](https://github.com/FranprzDev/donde-arde-argentina) (`harness-rs/`), at merge
`dc2382a` (PR #9, "feat: add the promo crate for config-driven short-form ads"). History of the vendored code upstream:

- `5dcfccc` all-Rust fframes harness (chunked render, QA)
- `ec8b78b` 720p drafts by default with pre-scaled rasters, 1080p only with `--final`
- `c5a6104` shared x264 rpath build script, `expand_env`
- `abfa411` datavid media helpers (probe, loudness, black frames, contact sheets)
- `c79ca1c` the `promo` crate for config-driven short-form ads

Changes made while vendoring, so the code builds and ships on its own:

- `datavid` keeps only the modules the promo template uses (`audio`, `ease`, `media`, `qa`, `svg`) plus `env`
  (`expand_env`, moved out of the data-story config module). The data-story modules and their dependencies stay
  upstream.
- Google Drive fetching (`drive_id`, `promo fetch`) is removed: clips are read from a local folder.
- The default caption font is embedded in the binary instead of being read from the repository at a compile-time path.

## License

MIT (see `LICENSE`). Pixelify Sans is under the SIL Open Font License (`fonts/OFL-PixelifySans.txt`).
