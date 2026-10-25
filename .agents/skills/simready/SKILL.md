---
name: simready
description: Optional guide for using an NVIDIA SimReady OpenUSD asset as the 3D source of a Reelsmith video. The asset is rendered outside Reelsmith, then composed with the normal harness flow.
---

# SimReady assets as video sources (optional)

Low priority. Use this only when a video would benefit from a ready-made 3D object or scene (a machine, a gripper, a piece of equipment) and modeling it from scratch is not worth the time.

Reelsmith has no native USD importer and this skill adds none. It does not install Blender or Isaac Sim, and it does not search or download assets for you.

## Sources

- Library: https://huggingface.co/spaces/nvidia/simready-central
- Spec and tooling: https://github.com/NVIDIA/simready-foundation
- Blender USD import: https://docs.blender.org/manual/en/latest/files/import_export/usd.html

SimReady assets are OpenUSD objects with simulation metadata such as collision geometry, mass, friction, joints and semantic labels. They are not AI-generated models. Metadata coverage varies by asset.

## Flow

1. The user picks the asset. Do not choose one for them or fetch it without being asked.
2. Check that asset's own license page before any public or commercial video. Licenses differ per asset. If it is unclear, say so and stop at a private preview.
3. Import the USD into a tool the user already has (Blender's USD importer, or Isaac Sim). Not every asset imports cleanly; materials and articulations may need manual fixes. Do not promise compatibility.
4. Render a short clip or turntable as an MP4 or PNG sequence outside Reelsmith.
5. Bring the clip into a normal Reelsmith project as a video asset. Add text, diagrams (`animate_svg`), ASCII or dither effects on top.
6. Follow the usual render flow: 360p preview first, inspect it, then 720p or 1080p.

## Limits

- Only the external render step touches 3D. Reelsmith composes the result.
- Credit the asset and keep its license note next to the project files.
- If the user has no 3D tool installed, ask before suggesting an install.
