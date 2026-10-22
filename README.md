# Reelsmith

Config-driven short-form promo videos (Reels / TikTok / Shorts ads) rendered in Rust with
[fframes](https://fframes.studio): gameplay or product clips with push-ins, alpha motion graphics with their sound
effects, flashes on the cuts, an end card, a procedural or external music bed, −14 LUFS mastering and built-in QA.

## Layout

| Path | What it is |
|---|---|
| `crates/datavid` | the media layer: SVG frame helpers, easing, procedural audio and EBU R128 mastering, ffmpeg/ffprobe helpers, QA reports |
| `crates/promo` | the promo template (library + `promo` CLI): config → plan → plates → audio → chunked fframes render → mux → SRT → QA |
| `docs/PROMO_VIDEOS.md` | the config reference and the step-by-step guide |
| `templates/promo/` | a starting `brief.toml` and a timed voice-over script |
| `fonts/` | Pixelify Sans Bold (OFL), the default caption font, embedded in the binary |

## Build

Requirements: Rust (stable, edition 2024), `clang`, `nasm`, `pkg-config` and the `ffmpeg` / `ffprobe` CLIs on `PATH`.
The first build compiles FFmpeg bindings and fframes and takes a few minutes.

```sh
cargo build --release -p promo
target/release/promo help
```

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
