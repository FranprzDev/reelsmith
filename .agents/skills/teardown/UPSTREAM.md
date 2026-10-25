# Upstream provenance

Vendored from https://github.com/jakubkrehel/skills, directory `skills/teardown`,
at commit `592aedc44d7d5b3e29740984252902f3e5402a1c` (2026-10-25).
Author: Jakub Krehel. License: MIT, preserved in `LICENSE` alongside this file.

The upstream skill and its assets, references and tests are unchanged. Reelsmith-specific
usage is in `docs/TEARDOWN.md`; no npm installer is run when using the MCP server.

To update, review the upstream diff first, replace this directory's upstream files,
preserve this notice and LICENSE, and run every `scripts/test-*.cjs` test.
The scripts execute model JavaScript supplied by the drawing author, so only run
trusted, reviewed models. Do not use untrusted model scripts as a sandbox.
