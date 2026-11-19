# Isometric explainer figures with /anatomy

Reelsmith includes the project-local [anatomy skill](../.agents/skills/anatomy/SKILL.md)
from [Ryan (@wheresryan22)](https://github.com/wheresryan22/anatomy).
It builds crafted, highly detailed isometric figures that explain a technical idea as a
real, working object (a test rig, a dial gauge, a lock, a plotter): many small precise
solid parts, four-tone shading, correct painter's order, a framed card with a live
readout, and calm eased motion. Figures are SVG with optional WebGL shader layers
(fire, water, lamp light) and can be printed as A4/Letter PDF sheets. Its original
instructions, kit, references, scripts, tests and example sources are included under
`.agents/skills/anatomy/` with their MIT license. See
[UPSTREAM.md](../.agents/skills/anatomy/UPSTREAM.md) for the pinned revision.

## Load it in your host

Open this repository as your working project. Hosts that discover `.agents/skills/`
(such as Codex and OpenCode) can load `anatomy` as a project skill. In other hosts,
point the agent at `.agents/skills/anatomy/SKILL.md` and `AGENTS.md`, or copy the
complete `anatomy` directory into the host's project-local skills directory
(for example `.claude/skills/anatomy/` for Claude Code). Claude Desktop's MCP setup
alone does not install agent skills: give it these instructions explicitly.

This is an agent workflow, not an `anatomy` MCP endpoint. The Rust server and binary
have no new runtime dependencies. Skill files travel with the repository, not inside
the standalone binary. No global installation, paid service or account is required.

## From figure to video

1. Ask the agent to use the skill to design and build the figure for the concept the
   video explains. Follow its workflow: concept and parts list first, geometry once,
   then its audit, line check and visual review.
2. Export a **static SVG still** of the pose you want on screen. For a standalone file
   use `figureSvg({ standalone: "dark" | "light" })` from `kit/iso-kit.mjs`, which
   embeds the styles. Keep the build script and model beside the generated assets.
   Reelsmith's renderer (resvg) draws static SVG only: no scripts, SMIL or CSS
   animations and no `foreignObject`, so the interactive page itself cannot be a video
   overlay — export the still (or one still per pose) instead.
3. Call Reelsmith `animate_svg` with the absolute `svg_path`, a chosen animation and
   duration, and an `output` inside your project's motion folder. `draw_on` animates
   stroked paths drawing themselves and refuses an SVG without strokes; figure SVGs
   are mostly filled solids, so `fade_scale`, `slide_up` or `pop` are the usual picks.
   `morph_svg` can morph one pose's filled shapes into another pose's shapes as a
   transition between two states of the mechanism.
4. Put the returned `asset_name` in a beat's `overlay.asset` or in `end_card.asset`,
   using the normal `write_brief` flow. See [animated SVG overlays](../README.md#animated-svg-overlays).
   For pure diagrams (flowcharts, sequences, mindmaps), `draw_diagram` remains the
   simpler path; anatomy figures are for when the explanation should be a machine.
5. Render and inspect the mandatory **360p preview** first. Check at phone size that
   the figure still reads: fine parts that vanish at 360p need a closer crop or a
   coarser pose, not more detail.

## Optional standalone dependencies

The kit (`kit/iso-kit.mjs`, `lathe`, `tube`, `audit`, `gl`) uses only Node's built-in
modules (Node 20.10+); no `npm install` is needed to build a figure page or export an
SVG. The verification and print scripts (`scripts/capture.mjs`, `drive.mjs`,
`lines.mjs`, `print.mjs`) launch a local Chrome/Chromium. The React kit
(`kit/react/`) and the regression tests need `react`, `react-dom`, `typescript` and
`esbuild` installed in the **figure's own project**, with a lockfile. None of these
are needed to run the Reelsmith MCP server.

Only run build scripts and pages you trust and have reviewed; they are executable
JavaScript, not untrusted data. Review imported SVGs for scripts and external
resources before use. Publishing a figure to the upstream community gallery
(`scripts/publish.mjs`) is never part of the Reelsmith flow; do not run it unless the
user asks for it explicitly.

## Verify the vendored skill

From `.agents/skills/anatomy/`, with Node 22+:

```sh
test_deps="$(mktemp -d)"
npm install --prefix "$test_deps" --no-audit --no-fund --ignore-scripts \
  react@19.3.0 react-dom@19.3.0 typescript@5.9.3 esbuild@0.28.2
NODE_PATH="$test_deps/node_modules" node --test scripts/*.test.mjs
```

These tests cover the kit builders, audit, capture/drive plumbing and React components.
They do not replace inspecting each generated figure and its exported files; follow
the skill's verification workflow (`references/verify.md`) for real assets.
