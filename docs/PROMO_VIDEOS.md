# Promo videos: from texts and clips to a vertical ad

The `promo` crate (`crates/promo`, binary `promo`) turns one config file into a short-form promo: gameplay or product
clips with slow push-ins, alpha motion graphics with their own sound effects, white flashes on section cuts, an end
card, a music bed (procedural or your own file), an optional voice-over, mastered to −14 LUFS, with QA built in.
It was generalised from a real Minecraft server ad: nothing brand-specific is in the code, everything lives in the
config files.

**Burned-in captions are off by default.** The texts you write still set the timing and are written as an SRT
sidecar (`out/<name>.srt`) for subtitles added downstream. `captions.enabled = true` (or `--captions on`) burns them in
with the word-by-word pop.

## What the template supports

| Area | Config | Notes |
|---|---|---|
| Canvas | `[video] aspect = "9:16" \| "16:9" \| "1:1"` | design canvas 1080×1920 / 1920×1080 / 1080×1080; 30 fps |
| Resolution | CLI | `--preview` 360p, default 720p draft, `--final` 1080p |
| Shots | `[[beat]] clip/clips` or `[[shot]]` | clip by file stem (`stem@in`), or path; `in`, `dur`, `zoom = [start, end]` push-in, `blur`, `focus` (crop centre when the clip's aspect differs, `look.fit = "cover"`) |
| Motion overlays | beat `overlay` or `[[overlay]]` | alpha `.mov`/`.webm`; `crop = "auto"` (measured from the alpha), `"full"` or `[x, y, w, h]`; `x`, `y`, `w` on the canvas; slot `delay`/`dur`; the asset's own sound at `sfx` gain; an asset shorter than its slot holds its last frame |
| Cuts | beat `flash = true` or `[video] flashes` | white flash (`look.flash_opacity`, `flash_dur`) + whoosh and hit (`music.flash_hits`) |
| End card | `[end_card]` | the clip stays sharp for `sharp` s, then blurs (`blur`) behind the full-frame `asset` |
| Captions | `[captions]` | off by default; font file (any TTF/OTF, metrics read from it; default Pixelify Sans Bold, OFL), `size`, `max_w` (long words shrink), `y`, `fill`, `outline`, `outline_w`, `shadow`, pop scale/duration/blur, `uppercase`; timing from the beats, `words`, a timed `script` or the voice-over script |
| Music | `[music]` | `kind = "procedural"` (bpm, seed, chord `progression`, `drop`, `outro`), `"file"` (`file`, `gain_db`, `start`) or `"none"`; `fade_out` |
| Cues | beat `cue` or `[[cue]]` | `braam`, `hit`, `whoosh` at a time with a gain |
| Voice-over | `[voiceover]` | `file`, `gain_db`, `offset`, `duck_db` (music ducks under the voice), `script` (timed captions aligned to the voice) |
| Loudness | `[audio]` | `lufs = -14`, `lufs_tolerance`, `ceiling_db` (limiter), `true_peak_max` (QA) |
| QA | `[qa]` | duration, size, fps, audio stream, integrated loudness, true peak, black frames, still shots (per-shot source motion, median YDIF; `min_motion`, 0 disables), min caption size (only when burned in), contact sheet, caption frames |
| Styles | `extends = "style.toml"` | inherit everything from another config (tables merge, arrays are replaced); paths resolve against the file that declares them |

## Make a new video from texts + clips

1. **Set up the toolchain** (once per sandbox / machine): see "Sandbox lessons" below, then
   `cargo build --release -p promo` (about 4 minutes the first time). The binary is
   `$CARGO_TARGET_DIR/release/promo`; below it is just `promo`.
2. **Start from the template:** `cp -r templates/promo my-ad` (any folder; it can live outside the repo).
3. **Put the media in place:** recordings in `my-ad/clips/`, alpha motion graphics in `my-ad/motion/` (or point
   `[paths]` / `--clips` / `--motion` at existing folders).
4. **Write the script** in `my-ad/brief.toml`: one `[[beat]]` per idea.
   ```toml
   [[beat]]
   text = "WHAT IF | MINECRAFT | HAD | NEW | DIMENSIONS?"          # "|" splits the caption groups
   clip = "sky_island@0.3"                                       # file stem @ in-point (s)
   overlay = "title_card"                                        # optional alpha asset

   [[beat]]
   text = "ON OUR SERVER IT IS REAL"     # without "|", short words join their neighbour
   flash = true
   clips = ["forest@7.1", { clip = "plains", in = 0.3, zoom = [1.1, 1.0] }]
   ```
   `dur` is optional: without it a beat lasts as long as its text takes to read (0.45–0.8 s per group, at least
   1.2 s). Several clips share the beat evenly unless one sets its own `dur`.
5. **Close with an end card:** `[end_card] clip = "outro@3.0"`, `asset = "cta"`, `dur = 4.5`.
6. **Check** (seconds, no rendering): `promo check my-ad/brief.toml` prints the timeline summary and every missing
   file; `promo plan my-ad/brief.toml` prints the full resolved timeline as JSON.
7. **Preview** at 360p: `promo build my-ad/brief.toml --preview`, then look at `build/<name>/qa/contact.png`.
8. **Draft** (720p, the default): `promo build my-ad/brief.toml`. Iterate on the config; only changed shots are
   re-cut and only changed chunks re-render.
9. **Final** (1080p, only after the draft is approved): `promo build my-ad/brief.toml --final`.
10. **Deliver:** `out/<name>_final_1080x1920.mp4` and the subtitles in `out/<name>.srt`.

Several ads in one go: `promo batch a/brief.toml b/brief.toml` or `promo batch ads.txt` (one config path per line,
relative to the list). It keeps going past a failed config and exits with the first non-zero code.

### Voice-over

```toml
[voiceover]
file = "vo.wav"
script = "vo.txt"     # what is said, timed in voice-over seconds
offset = 0.3          # the voice starts 0.3 s into the video
duck_db = 8
```

`vo.txt` (see `templates/promo/vo.txt`): one caption per line, `SECONDS TEXT` (`m:ss.s` works too), `A-B TEXT` for an
explicit end, `end S` to close the last caption, `#` comments. `.srt` files are read as well. The voice-over script
becomes the captions (and the SRT sidecar), so on-screen text follows the voice. Beat durations still come from the
beats: set each beat's `dur` to match the voice.

### Explicit form

An explicit config writes every time by hand (`[[shot]] t0/t1`, `[[overlay]] t0/t1`,
`[captions] words = [[t, "TEXT"], ...]`): use it when an editor's cut list has to be reproduced exactly. A config uses
either beats or shots, not both; `[[overlay]]` entries with absolute times can be added to either.

## Commands

```text
promo build  CONFIG [flags]      check -> plate -> audio -> render -> mux -> srt -> qa
promo batch  CONFIG... | LIST.txt
promo check  CONFIG              promo plan CONFIG
promo plate [shots|overlays] | audio | render [--chunk K] | mux [--out F] | srt | qa [VIDEO]   (single stages)

--preview | --final               resolution (default 720p draft; PROMO_FINAL=1 also asks for the final)
--captions on|off                 override captions.enabled
--clips DIR --motion DIR          override the media folders
--out FILE --out-dir DIR --build-dir DIR
--encoder auto|nvenc|amf|qsv|vaapi|videotoolbox|x264   GPU / encoder backend (auto: first that opens, else libx264)
--chunk-frames N (60) --retries N (3) --no-qa
--jobs N                          plates cut and chunks rendered at once (default: every core)
--json                            machine-readable progress on stdout
```

Outputs: `out/<name>_<preview|draft|final>_<w>x<h>[_captions].mp4`, `out/<name>.srt`, `<mp4>.result.json`;
caches and QA in `build/<name>/` (`plan.json`, `crops.json`, `frames_*`, `chunks_*`, `audio.wav`,
`qa/QA.md`, `qa/qa.json`, `qa/contact.png`).

## Driving it from an app

The CLI is designed so a thin upload app (clips folder + script file in, video out) can drive it without parsing
logs:

- **Input:** a config as TOML or JSON (same keys; `.json` extension). An app writes a brief with the user's texts
  and clip names, uploads the clips into one folder and passes it with `--clips DIR` (and `--motion DIR`,
  `--out-dir DIR`, `--build-dir DIR` per job).
- **Progress:** `--json` prints one JSON object per line on stdout: `{"event":"stage","stage":"render","status":"start"}`,
  `{"event":"progress","stage":"render","done":3,"total":12}`, and a last `{"event":"result", ...}`. Human logs stay on
  stderr.
- **Result:** the `result` event (also written to `<mp4>.result.json`) carries `ok`, `exit`, `output`, `srt`, the plan
  summary and the QA summary (LUFS, true peak, black frames, failures, report paths).
- **Exit codes:** `0` ok, `1` failed (unexpected; retryable), `2` bad config or usage (fix the input), `3` rendered but QA
  failed, `4` an input file is missing (upload it).
- `promo check CONFIG --json` validates a job before queueing it (exit 4 lists every missing file).

## Sandbox lessons

- **2 CPUs, no GPU.** fframes renders on the CPU backend. Plates and chunks run on a worker pool (`--jobs`, default
  every core; lower it on machines with little RAM, since each job is its own process). A 720p frame of this template
  costs ~40 ms; the 22.5 s reference draft renders in ~30 s, plating takes longer than rendering. Measured timings:
  [`PERFORMANCE.md`](../PERFORMANCE.md); more pitfalls and fixes: [`LESSONS.md`](LESSONS.md).
- **Rebuilds take ~4 minutes** (fframes + ffmpeg-sys). Most edits are config-only and need no rebuild. `cargo build`
  runs past a 120 s tool limit: start it detached (`setsid nohup cargo build --release -p promo > build.log 2>&1 &`)
  and poll the log.
- **Write MP4s to local disk, then copy.** `+faststart` rewrites the file at the end, which fails with "Error
  writing trailer" on JuiceFS / network filesystems. `promo mux` always writes to the system temp folder and copies.
- **System packages**: `build-essential clang libclang-dev pkg-config libfontconfig-dev`, plus the `ffmpeg` CLI. On
  Linux and macOS the FFmpeg libraries come from a static, portable prebuilt archive (fframes `build-portable`), so no
  system codec libraries are linked.
- **Encoding.** Each chunk streams RGBA frames from fframes into the system FFmpeg and encodes H.264 once with the
  selected backend and quality tier. Chunks remain resumable and cached. The final mux stream-copies video and encodes
  the mastered audio once; explicit chunk durations preserve frame timing at joins. The encoder settings and pipeline
  version are part of each chunk key, so old lossless chunks are not reused. Hardware backends still require a working
  system FFmpeg and driver; `--encoder auto` probes them and falls back to libx264.
- **Plates.** fframes takes no per-frame raster input from a video file in this harness, so `plate` caches every frame
  as JPG (shots, at the output size) and RGBA PNG (overlays, cropped, at their drawn width). Each chunk process only
  loads the rasters it draws (hard-linked into a per-chunk folder), which keeps memory flat.
- **Never commit media:** clips, motion graphics, renders, SRTs, `build/` and `out/` are ignored. Fonts only with a
  license that allows it (Pixelify Sans: OFL, `fonts/OFL-PixelifySans.txt`).

## Roadmap

- 60 fps output (the fframes `Video` takes FPS as a constant: add a second monomorphisation).
- A `still` command (one PNG at a time, for config previews without a chunk render) and a vertical cover/thumbnail
  preset.
- Forced alignment of a voice-over to a plain-text script (no hand-timed `vo.txt`), and beat durations derived from it.
- Per-overlay entrance/exit animation (fade, slide, scale) and a keyframed position for a moving overlay.
- More caption styles (karaoke highlight, two-line groups, per-word colour) and safe-area guides per platform.
- Music: loop / trim an external track to the cut and snap cuts to its beats.
- Platform presets (Reels / TikTok / Shorts loudness and length limits) checked in QA.
- A 360p smoke render on the procedural sample clips in CI (CI already builds both binaries and checks the MCP
  `tools/list`).
