# Reelsmith agent skills

Project-local skills live in `.agents/skills/`. For technical illustrations, exploded
objects or layered concepts, read `.agents/skills/teardown/SKILL.md` and its linked
references. The Reelsmith handoff is documented in `docs/TEARDOWN.md`.

The skill creates SVG/PNG/PDF assets, not a new MCP tool. Use `animate_svg` to turn
its exported SVG into a motion overlay, then use the normal project/brief/render flow.
Always render and inspect a 360p preview before requesting 720p or 1080p output.
