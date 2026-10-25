# Technical illustrations with /teardown

Reelsmith includes the project-local [teardown skill](../.agents/skills/teardown/SKILL.md)
from [Jakub Krehel](https://github.com/jakubkrehel/skills), the skill in
[this post](https://x.com/jakubkrehel/status/2114379959263502712).
It builds configurable technical illustrations, exploded objects and layered concepts,
with SVG, PNG and PDF exports. Its original instructions, viewer, geometry helpers,
references and tests are included under `.agents/skills/teardown/` with their MIT license.
See [UPSTREAM.md](../.agents/skills/teardown/UPSTREAM.md) for the pinned revision.

## Load it in your host

Open this repository as your working project. Hosts that discover `.agents/skills/`
(such as Codex and OpenCode) can load `teardown` as a project skill. In other hosts,
point the agent at `.agents/skills/teardown/SKILL.md` and `AGENTS.md`, or copy the
complete `teardown` directory into the host's project-local skills directory
(for example `.claude/skills/teardown/` for Claude Code). Claude Desktop's MCP setup
alone does not install agent skills: give it these instructions explicitly.

This is an agent workflow, not a `teardown` MCP endpoint. The Rust server and binary
have no new runtime dependencies. Skill files travel with the repository, not inside
the standalone binary. No global installation, paid service or account is required.

## From illustration to video

1. Ask the agent to use the skill to build the object. Keep invented dimensions and
   hidden construction labeled as illustrative. Follow its evidence and visual checks.
2. Export the selected view as SVG. For an overlay, choose a transparent background.
   Keep the editable source, configuration and evidence beside the generated assets.
3. Call Reelsmith `animate_svg` with the absolute `svg_path`, `animation = "draw_on"`,
   a chosen duration and an `output` inside your project's motion folder. Use another
   preset, such as `fade_scale`, if the SVG has no stroked paths.
4. Put the returned `asset_name` in a beat's `overlay.asset` or in `end_card.asset`,
   using the normal `write_brief` flow. See [animated SVG overlays](../README.md#animated-svg-overlays).
5. Render and inspect the mandatory **360p preview** first. Only then request a 720p
   review or 1080p final. Check linework and labels at the actual video size.

The viewer's interactive assembled/exploded transition is not automatically converted
into a video: `animate_svg` animates a single exported SVG view. Keep geometry changes
or full assembly animation as a separate, explicitly requested workflow.

## Optional standalone viewer dependencies

The SVG modeling helpers use Node's built-ins. To build the optional interactive HTML
viewer with PDF/PNG exports, use Node 22+ and install `jspdf`, `svg2pdf.js` and
`@fontsource-variable/inter` in the **drawing's own project**, with a lockfile. Follow
[viewer-assets.md](../.agents/skills/teardown/viewer-assets.md) for configuration and
build/check commands. These packages are not needed to run the Reelsmith MCP server.
The default viewer embeds dependencies and fonts and works offline after building.
Custom fonts can introduce network requests, so keep fonts embedded and run the
viewer's no-network check before sharing it.

Only run model scripts you trust and have reviewed. They are executable JavaScript,
not untrusted data, and the proof helper's VM is not a security boundary. Review
imported SVGs for scripts and external resources before use. The other upstream
`better-*` skills mentioned in the instructions are optional companions, not bundled
here; do not install them automatically.

## Verify the vendored skill

From the repository root, with Node 22+:

```sh
for test in .agents/skills/teardown/scripts/test-*.cjs; do node "$test" || exit; done
```

These tests cover the geometry/projection helpers, viewer builder, render queue,
flipbook and appearance state. They do not replace inspecting each generated drawing
and its exported files. Follow the skill's verification workflow for real assets.
