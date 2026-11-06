# The open-source motion stack, mapped to Reelsmith

Source: an X post by @renproton listing 16 open-source repos for a motion-design studio without After
Effects (https://x.com/renproton/status/2118392436863201677, sent by Fran on 2026-11-05 with "agrega esto
al harness de video"). This is where each one landed for Reelsmith: integrated, already covered, or not
applicable, with the reason.

## Integrated

- **Flubber** (shape morphing) - the `morph_svg` MCP tool: morphs every filled path of one SVG into the
  matching paths of another (pairwise, or one-to-many / many-to-one via flubber's `separate` / `combine`),
  lerping fill colors alongside, and renders an alpha `.mov` overlay with the same contract as
  `animate_svg`. flubber is vendored (MIT) and runs on the embedded QuickJS runtime, like Pele.

## Already covered by the harness

- **FFmpeg** - the core encoder/decoder under every render.
- **Anime.js** (timelines and easing) - the fframes plan eases every move in Rust (`datavid::ease`), and
  the HyperFrames engine builds its composition on GSAP timelines.
- **twigl** (shader loops) - `stylize_clip` rebuilds footage in Rust (ASCII, ordered dither); procedural
  visuals ship in `make_sample_clips`.
- **Pedalboard** (reverb, compression, audio effects) - `datavid::audio` does this in Rust: convolution
  reverb sends, filters, sidechain-style music ducking under the voice-over, and mastering to -14 LUFS
  with a true-peak ceiling.

## Not applicable (and why)

- **Remotion** - a third render engine on top of fframes + HyperFrames adds no capability; it duplicates
  HyperFrames' HTML/JS composition model.
- **Three.js / PlayCanvas / cannon-es / Effekseer** - need a real-time WebGL/WebGPU scene runtime in the
  render path, which the harness does not have (the `/teardown` skill covers technical 3D illustration as
  SVG overlays). Worth revisiting only if a WebGL scene engine ever joins the engines.
- **Sverchok** - a Blender add-on; Reelsmith is one Rust binary with no Blender dependency.
- **Storyboarder / Pixelorama / OBS Studio / Shotcut** - desktop GUI apps; the harness is a headless MCP
  server. Shot planning is `write_brief`, capture is out of scope, timeline editing is the render plan.
- **Bark** - neural voice/sound generation; too heavy to embed and out of the harness's deterministic,
  local-only design (voice-over comes in as a file).
