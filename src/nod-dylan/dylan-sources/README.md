# `nod-dylan/dylan-sources/`

This directory will hold the **ported Dylan kernel library**. Empty as of Sprint 01.

Per [MANIFESTO.md](../../../MANIFESTO.md), NewOpenDylan does **not** self-host. The upstream `dylan` library (91 files at `E:\opendylan\sources\dylan\`) is ported here as runnable Dylan source — not as compiler bootstrap. The Rust compiler reads these files at startup once the loader (Sprint 05) and codegen (Sprint 07) are online.

Sequence:
1. **Sprint 05** — LID parser, module graph, *empty* `dylan-sources/` produces a valid graph.
2. **Sprint 09** — first kernel files (constants, basic arithmetic) land.
3. **Sprints 12-15** — class system + sealed dispatch enables most of the kernel.
4. **Sprint 17+** — macros unlock the remaining ~30 files that use pattern macros heavily.

License: each ported file retains the upstream Open Dylan licence header (MIT-equivalent). See `../README.md` for attribution policy.
