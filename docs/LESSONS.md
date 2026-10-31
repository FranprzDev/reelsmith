# Lessons: what earlier video work taught us

Reelsmith grew out of several real video projects built between October 10 and 31, 2026: a hackathon pitch and demo,
a 60 s open-data contest video, explainer Shorts and a long-form video for a YouTube channel, portfolio and product
intros, and a vertical ad for a Minecraft server. This file keeps what those projects taught, so that any person or
assistant picking up the harness starts from it instead of relearning it. Every number below was measured on one of
those projects; where a lesson comes from a project with its own toolchain (Python, plain resvg) it says so.

Read it together with:

- [`AGENTS.md`](../AGENTS.md): how to build, run and check the harness.
- [`PROMO_VIDEOS.md`](PROMO_VIDEOS.md) (config reference), [`MOTION_DIRECTION.md`](MOTION_DIRECTION.md) (briefs that move),
  [`PROMO_COPY_DESIGN.md`](PROMO_COPY_DESIGN.md) (copy), [`EVALUATION.md`](EVALUATION.md) (reference renders),
  [`../PERFORMANCE.md`](../PERFORMANCE.md) (measured timings).
- For data-story videos (open data, computed facts, readability rules, contest anonymity), the general guide is
  [`DATA_VIDEO_GUIDELINES.md` in FranprzDev/config-ai-bots](https://github.com/FranprzDev/config-ai-bots/blob/main/DATA_VIDEO_GUIDELINES.md).
  It is not repeated here.

When a new lesson comes up, add it to the matching section: the symptom, the cause, the fix, and a measured number if
there is one. Never write a number nobody measured.

## 1. Workflow: preview → draft → final

1. **Draft cheaply, finish once.** Iterate on 360p previews, show the owner a **720p draft**, and render 1080p (or any
   higher quality) **only after the owner says the cut is final**. This is a standing rule of the owner ("render every
   video at 720p until I give the final confirmation"; "first in lower quality so it is faster"). The harness enforces
   the first half: `draft` / `final` refuse to run without a completed 360p preview of the same config and inputs
   (`<config>.preview.json`, PR #20).
2. **Versions, not overwrites.** Each round of feedback is a new version (`v1`, `v2`, …, or a new brief name); keep the
   previous files so a change can be compared and undone.
3. **Do not publish anywhere until the owner says it is finished.** No uploads to Drive, YouTube or social media
   from a draft. A draft once uploaded to Drive by mistake had to be deleted the same night.
4. **Deliver with the video:** the SRT sidecar, the QA report and the contact sheet, plus a short note of what changed
   and what is still open (for example "voice-over still missing", "end-card text is my wording").
5. **Build the tool, not one video.** When the same kind of video is needed again, improve the harness or the template
   instead of producing another one-off. This is how Reelsmith itself started ("stop generating videos, the idea is to
   make it an app").
6. **Validate by rendering, not with test suites.** The owner prefers a short low-resolution smoke render (360p) and
   its QA over adding unit or integration tests to the video harness. Unit tests stay where a parser or a check needs
   them (for example the YDIF parser in `datavid`).

## 2. What the owner liked and disliked in drafts

Feedback collected from real review rounds. Treat it as taste guidance, not as hard rules.

**Liked / asked for**

- **"Epic", with aura.** Reveals need weight. A title that just appears drew "your introduction has no aura" and,
  on another video, "it lacks epicness"; the next versions answered with a centred reveal (spotlight halo, light
  wave, impact and a low sound hit) and with camera moves, glow and counting numbers.
- **Continuous motion.** One idea flowing into the next (the camera keeps travelling, the same map or object carries
  across beats) rather than a sequence of slides. See [`MOTION_DIRECTION.md`](MOTION_DIRECTION.md).
- **Fast overview, then slow detail.** Example: an opening that flies sun → Earth → country deliberately too fast to
  read, then slows down for the detail beats. Travel directly from one detail to the next instead of returning to
  the overview every time.
- **Motivated transitions.** A paper plane carrying a payment into an escrow, a rocket launching between scenes, a
  camera that dives *into* a screen, instead of a generic boom or flash on every cut.
- **The real thing on screen.** The product's exact palette and fonts, its real UI, its real mascot SVG from the
  repository, real photos of the people. Team members shown large, so viewers see who they are.
- **Identifying marks.** When a place or brand is shown, show its mark too (in one project: the province's flag
  every time a province appeared).
- **His exact wording.** When he dictates a line, use it verbatim.
- **A strong close.** The ending carries the message; no dead time after it.

**Disliked / asked to remove**

- **Too much text, too fast.** A tester said "it goes very fast, I can't read it, less text but legible". Fixed with
  at most ~12 words on screen per beat, text ≥ 48 px on the 1080p canvas and a hold of at least
  `max(2.5 s, 0.35 s × words + 1 s)`. Cut words, never shrink the rule.
- **Overused impacts.** "Don't abuse the booms": keep big hits for the two or three moments that matter.
- **Empty stretches.** "From 0:54 the screen looks empty": a credits card stood still for 9.4 s while the music faded
  to near silence. Fix: shorten the static card (to 6.4 s) and keep the music to the last frame.
- **Marketing tone when a demo was asked for.** "It should not look like marketing but a simple demonstration of the
  software": show the flow plainly.
- **Claims the client does not make.** For the server ad, on-screen claims were limited to the claims in the client's
  own finished ads; assets promising something else (a "free" badge) were left out.
- **Overstated status.** A prototype is described as an early-stage prototype, never as production-ready.
- **Details get reviewed.** He flagged a URL in one shot and an overlay that rendered misaligned on a map. Check
  URLs, names and alignment on the contact sheet before sending.

**Promo-specific defaults he set**

- Burned-in captions **off** by default; the texts still time the cut and ship as an SRT for subtitles added later.
- Vertical 9:16 for Reels / TikTok / Shorts ads; structure of the reference ads: a question as the hook (0–3 s), one
  feature per 1.3–2.5 s cut, slow push-in on every shot, white flash only on section changes, end card with the
  mascot and the server IP.

## 3. Pitfalls and fixes

### Rendering (fframes and the harness)

| Symptom | Cause | Fix |
|---|---|---|
| A long render is OOM-killed partway | fframes loaded every raster in the media folder | Render in chunks; each chunk process loads only the rasters it draws (hard-linked into a per-chunk folder). Memory stays flat. |
| A 1080p final is killed on one effects-heavy stretch | full-resolution layers decoded for every frame | Smaller chunks for that stretch, retries with a pause, skip chunks that already finished, mux once at the end. |
| fframes cannot read a frame from a video file | no per-frame raster input from video in this harness | `plate` caches frames: JPG for shots at the output size, cropped RGBA PNG for overlays at their drawn width. |
| CPU backend crawls on glow / bloom | full-resolution `feGaussianBlur` (σ 10–52 px over 1920×1080) and layers drawn twice (plain + bloom) | Blur at half resolution, precompute bloom into the PNGs, or draw glow as wide translucent strokes. Never rasterize the same layer group twice. |
| A photo passes as video | the default push-in makes a still read ~1.1 mean YDIF on the render | Measure motion on the **source window**: median YDIF of a frozen frame ~0.001 vs ~0.9 for the calmest sample clip; QA fails below `min_motion` (0.1) (PR #29). |
| Video encoded more work than needed | chunks were written lossless, decoded and re-encoded in every mux | Encode each chunk once to H.264 and stream-copy the join with explicit chunk durations: mux 10.4 s → 0.3 s, fully cached rebuild 12.8 s → 2.65 s (PR #27). |
| A QA check passed although the text was too small | the text-size check measured nothing (empty set) and passed vacuously | A check must fail, or say so, when it has nothing to measure. |
| Swapped headlines looked garbled | cross-fading two different texts in the same spot | Swap text with a hard cut on the beat. |
| A cut lands on an empty frame | the shot's hero entered after the cut | The hero is already visible on its first frame. |
| Screen goes near-black | several stacked dim layers multiplied | Use one union dim. |
| Text clipped at the frame edge | layout not checked against the frame and platform UI | Check stills at each beat; on Shorts the right ~15 % and bottom ~20 % are covered by the app's UI. |
| The first 1.5 s are empty | nothing scheduled before the first line | Something lands on the first beat; for Shorts, frame 0 is the thumbnail and already shows the hook. |

### Audio

- Target **−14 LUFS integrated**, measured on the **delivered file** (after AAC), not on the WAV. Reelsmith QA allows
  ±0.5 LU and a true peak ≤ −1 dBTP; data videos use ≤ −1.5 dBTP.
- A first master landed at −14.5 LUFS: add a limiter and re-run the two-pass loudness normalisation.
- Hits that land late feel off: transient peaks measured 0.6–1.7 frames after the visual; snapping each hit's peak to
  the visual frame + 1 fixed it. Clear the music bed under a signature sound (a bark, a logo sting) so its peak is not
  smeared.
- Sync cuts to a beat grid computed from the frame index (beat *n* at `round(n × frames_per_beat)`), never by
  accumulating durations, so rounding never drifts.
- Original synthesised music avoids rights problems; a third-party sound needs a clear licence.
- Offline TTS that worked for English narration: Kokoro-82M (Apache-2.0); time captions per spoken clause and check
  intelligibility with an ASR pass (word error rate below 0.06 on the first batch).

### Build and environment

- **`+faststart` on a network or FUSE filesystem** fails with "Error writing trailer" or leaves a file without its
  `moov` atom. Write the MP4 to local temp, then copy (`promo mux` already does).
- **Builds outlast short tool timeouts.** A first build takes ~4 minutes (fframes + ffmpeg-sys). Start it detached
  (`setsid nohup cargo build --release -p promo > build.log 2>&1 &`) and poll the log. Most edits are config-only and
  need no rebuild.
- **Low memory (~7–8 GB, 2 vCPU).** The `promo` crate alone built in 3–4.5 min (final link with
  `-Wl,--no-keep-memory`). Once the MCP crate was added, local release builds were killed: the `promo` library at
  `opt-level = 3` (SIGKILL, once SIGBUS), then the `reelsmith` binary even with a per-package profile override
  (`promo` at `opt-level = 2`, `codegen-units = 4`; `reelsmith` at `opt-level = 1`), `-j 1` and a low-memory linker
  (`lld`, or `-Wl,--no-keep-memory -Wl,--reduce-memory-overheads`); free memory fell below 100 MB. Reliable path on
  such a machine: let CI build, download the `reelsmith-<target>` artifact from the run, and validate with it (that is
  how most Reelsmith PRs were validated). The CI workflow builds `promo` and `reelsmith` one after the other to keep
  the peak memory of the links low.
- **Windows** links FFmpeg dynamically: a shared FFmpeg whose major version matches the bindings (BtbN 9.0 LGPL
  shared), `FFMPEG_DIR` pointing at it, and its DLLs on `PATH` at build time too, because the fframes proc-macro
  links FFmpeg. Ship the DLLs next to the exe.
- **Earlier harness only:** the FFmpeg prebuilt that `ffmpeg-sys-fframes` downloaded linked libx264 ABI 163 while
  the distro shipped a newer one, which needed a hand-placed library and an rpath. Reelsmith avoids it with the
  static portable FFmpeg on Linux/macOS (PR #3).
- **GitHub Actions warnings about Node 20** come from the actions themselves, not from the project: bump the action
  versions (PR #8).
- **Fallback renderer.** When a full fframes build was too slow for a one-off, a plain Rust → SVG → resvg → ffmpeg
  pipe produced a 45 s 720p intro. Keep it for one-offs; use the harness for anything repeatable.

### Delivery

- Re-encode an upload copy (x264 CRF ≈ 20, `yuv420p`, `+faststart`, audio copied) and keep the master.
- Browser-automated uploads failed repeatedly ("could not copy the file to the browser"); always have the file ready
  for a person to upload by hand.
- YouTube put a daily upload limit on a new channel after 9 uploads in one day (lifted by a one-time identity
  verification or after 24 h), and Studio's "we had trouble saving your video" error went away after a fresh sign-in.
- Never commit videos, thumbnails, client clips or other large binaries to a repository.

## 4. Performance: what helped and what did not

Full numbers and setup: [`../PERFORMANCE.md`](../PERFORMANCE.md). In short:

- **Helped:** cutting plates in parallel (#9), rendering chunks in parallel (#11, render −15 % on 2 cores), FFVHuff
  instead of FFV1 for the then-lossless chunks (#14, mux −20 to −35 %), GPU backend detection with per-tier presets
  (#13), and encoding chunks once with a stream-copied join (#27). 720p drafts instead of 1080p saved ~40 % of render
  time and ~30 % of peak memory per iteration in the data-video harness.
- **Did not help:** thin LTO + one codegen unit (#10, within noise; the hot code is fframes' rasteriser and the
  prebuilt FFmpeg), `panic = "abort"` (the MCP server must survive a panicking job), `target-cpu=native` (binaries go to
  unknown CPUs). In the data-video harness, parallel chunks on 2 cores gained nothing and doubled memory: the renderer
  already used both cores.
- **Benchmark honestly:** cold builds on local disk, the same input before and after, two runs, and a note on noise
  (the plate stage swung ±3 s between identical runs on 2 cores). Prove output equality before claiming a speed-up
  that should not change pixels: [`scripts/compare_renders.py`](../scripts/compare_renders.py) prints SSIM / PSNR,
  frame counts and loudness of two renders.
- **Still to measure** on a real NVIDIA machine: NVENC vs libx264 mux time, CUDA decode in the plate stage, `--jobs`
  scaling and 1080p peak memory on 8+ cores.

## 5. Checklist before sending a draft

- [ ] `promo check` lists no missing input; the timeline reads as intended (`promo plan`).
- [ ] QA green: duration, size, fps, frame count, audio stream, −14 LUFS ± 0.5, true peak ≤ −1 dBTP, no black
      frames, no still shots.
- [ ] Contact sheet opened and read by eye: nothing clipped or overlapping, hooks on the first frames, a hero on every
      cut frame, no empty stretch, legible at phone size.
- [ ] Every beat moves (push-in, overlay drawing on, camera travel); big impacts kept for the few moments that matter.
- [ ] Texts: few words per beat, held long enough to read, exact wording where the owner dictated it, no claim the
      client or product does not make, URLs and names checked.
- [ ] Music runs to the last frame; hits land on their visual frame.
- [ ] Rendered at the resolution the stage calls for (360p preview, 720p draft; 1080p only after the explicit OK).
- [ ] Delivered with the SRT, QA report and a note of open decisions; nothing uploaded or published.
