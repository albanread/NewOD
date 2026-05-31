# NewOpenDylan — Engineering Journal

A running lab notebook. Where `SPRINTS.md` records *what* shipped per
sprint and the commit log records *what changed per file*, this
journal records the part that otherwise evaporates: **what we were
trying to do, how we approached it, why we chose what we chose, and
what we discovered along the way** — including the wrong turns, the
"oh, it's actually simpler" moments, and the lessons that should
outlive the session they were learned in.

The audience is us, six months from now, trying to remember why the
architecture is shaped the way it is.

## Convention

- One file per session or coherent work-arc:
  `YYYY-MM-DD-short-slug.md`.
- Index below, newest first.
- Each entry, loosely:
  1. **Goal** — what we set out to do this session.
  2. **What we did** — the arc, with commit refs.
  3. **Why** — the decisions, especially the ones we reversed.
  4. **Discovered** — the lessons. This is the part that matters
     most; be honest about surprises and dead ends.
  5. **Where it leaves us** — state + the obvious next move.
- Keep prose over bullet-spam where the reasoning is the point. This
  is a notebook, not a changelog.

## Entries

- [2026-05-31 — Front-end self-hosting: the breakthrough session](2026-05-31-front-end-self-hosting.md)
  — Sprints 51b–51e. The Dylan lexer and parser go live inside the
  driver; the architecture is reframed to a Dylan front-end on a
  permanent Rust+LLVM back-end; the parser coverage harness measures
  77% baseline and produces the extend-list.
