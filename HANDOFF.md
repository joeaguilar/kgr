# kgr — Handoff

_Last updated: 2026-05-29_

A continuity note bridging sessions. The **`itr` tracker is the source of truth** for task
detail — this doc is the narrative: where things stand, what's next, and the context you
can't get from code or the tracker alone. Issue IDs (`#N`) link to `itr get N`.

---

## TL;DR

A multi-agent **triple-audit** (comedy diff roast + architecture audit + adversarially
verified bug hunt) of `kgr` ran and produced **32 confirmed findings**. They're filed as a
deduped backlog under epic **#6** (`itr get 6`). The headline finding — kgr's Rust resolver
misclassifying a crate's own modules as external packages — has been **fixed and merged to
`main`**. Everything else is open and prioritized.

---

## Repo / branch state

- **`main`** — has the resolver fix (`12e0462 fix(resolve): resolve crate-local Rust imports in all layouts`). `just verify` is green here.
- **`fix/rust-import-resolution`** — the resolver fix branch, already fast-forward-merged into `main`. Safe to delete (`git branch -d fix/rust-import-resolution`).
- **`chore/comedy-code-roaster-agent`** — adds `.claude/agents/comedy-code-roaster.md`. **NOT yet merged.** To land:
  ```sh
  git checkout main && git merge --ff-only chore/comedy-code-roaster-agent
  ```
- `HANDOFF.md` (this file) is currently **untracked** — commit it if you want it versioned.
- `.claude/settings.local.json` is gitignored (local config) — leave it alone.

---

## What shipped this session

1. **Resolver fix** (#8, #9 — closed). `resolve_rust` rewritten in `crates/kgr-core/src/resolve.rs`:
   anchors `crate::` at the owning crate's `src/` (works in workspaces), resolves bare
   `use foo::Bar;` against crate-local modules, handles `self::`/`super::`, and shortens
   trailing item segments. Parser (`parse/rust_lang.rs`) now expands grouped `use a::{b,c};`.
   `graph.rs` dedups parallel edges. Replaced the fragile `../itr` snapshot tests with a
   deterministic in-repo fixture (`tests/fixtures/rust/local_modules/`). +16 unit tests.
2. **Backlog filed** — 30 issues under epic #6, plus follow-ups #32–#36 (see below).
3. **`comedy-code-roaster` agent** — a meme-fluent code-review subagent (`.claude/agents/`),
   committed on its own branch (unmerged). It did a live roast of the resolver branch that
   surfaced four real findings (#33–#36).

---

## What's next (prioritized)

Run `itr ready` for the live, urgency-sorted list. Current top of queue:

### Do first
- **#7 — CRITICAL** — silent glob-drop in `rules.rs`: a typo'd rule pattern is silently
  dropped and `kgr check` still passes. A guardrail that disarms itself. High value, contained.
- **#33 — high** — phantom-edge regression from the resolver fix: a bare import colliding
  with a local module name (e.g. `use time::Duration;` when `src/time.rs` exists) draws a
  fake Local edge. **Bug in code we just shipped** — worth closing the loop.

### Then the highs (mostly silent-failure footguns)
- **#10** `.kgr.toml` parse error silently wipes the entire config.
- **#11** rule globs anchored to full path → common patterns silently never match (dead rules).
- **#12** parse cache: whole-second mtime + size key misses same-second / same-length edits.
- **#13** `kgr graph --format json --symbols` omits `external_deps` (JSON contract violation).
- **#14** four wired commands (`orient`/`impact`/`hotspots`/`skeleton`) undocumented in AGENT_DOCS.

### Mediums / lows
~20 issues spanning determinism (#15), Python resolution (#16/#17), `--no-external` no-op
(#20), parser correctness (#21, #25, #26), portability (#24), and the resolver follow-ups
below. See epic #6 children.

### Resolver follow-ups (from the roaster)
- **#33** high — phantom edges (above).
- **#34** medium — `external_packages` lists item-level paths + duplicates (`std` ×3), not
  package names. This is the unfinished half of #8's acceptance.
- **#35** low — `crate_src_base` silently falls back to repo root when no `src/` ancestor.
- **#36** low — add a test pinning glob imports (`super::*`) to intentional-`None`.

### Separate epic
- **#1** — release & install pipeline parity with `itr` (#2–#5). Independent of the audit work.

---

## Key context & decisions (the stuff you'd otherwise have to rediscover)

- **The resolver fix uses heuristics, knowingly.** `crate_src_base` finds the nearest
  ancestor dir literally named `src`; `module_dir` special-cases `mod`/`lib`/`main`;
  `try_module` pops trailing segments. These are correct for normal layouts but have the
  documented gaps in #33/#35. Don't "clean them up" without reading those issues first.
- **kgr now detects a real cycle in its own `parse/` module (#32).** Every parser does
  `use crate::parse::Parser;` while `parse/mod.rs` declares `pub mod <lang>;` — a genuine
  20-node SCC. The *old* resolver was blind to it; the fix exposes it. So **`kgr check` on
  kgr itself will now report this cycle** — decide whether to break it (extract the `Parser`
  trait to a leaf module) or baseline it. Not a regression; better detection.
- **Newer clippy is stricter.** The toolchain here (rust-1.95.0) flags `unnecessary_sort_by`,
  which tripped pre-existing code. Two were fixed in the resolver commit. If CI used an older
  clippy, expect divergence.
- **`comedy-code-roaster` agent doesn't hot-reload mid-session.** A freshly created
  `.claude/agents/*.md` isn't picked up by the `Agent` tool / workflow `agentType` in the
  same session that created it. Workarounds: start a new session, or run a `general-purpose`
  agent with the roaster persona inlined (that's how the live roast was done).
- **Snapshots:** `cargo-insta` is not installed. Use `INSTA_UPDATE=new cargo test -p kgr
  --test snapshots` then rename `.snap.new` → `.snap` (strip the `assertion_line:` field to
  match repo style). The fixture writes a gitignored `.kgr-cache.json` on each run — ignore it.

---

## How to resume

```sh
itr ready -f json --fields id,title,urgency,status   # what's next
itr get 7 -f json                                    # read full detail before starting
itr update <ID> --assigned-to <you> --status in-progress
# ... work ...
cargo check --workspace && cargo clippy --workspace --all-targets -- -D warnings \
  && cargo test --workspace && cargo fmt --all -- --check   # the verify gate (just verify, if `just` is installed)
itr note <ID> "what changed" && itr close <ID> "reason"
```

- **Verify gate must be green** (zero warnings/errors) before committing.
- **Branch before committing** if you're on `main`; merge with `--ff-only`.
- To roast more code: see the agent persona in `.claude/agents/comedy-code-roaster.md`;
  run it via `general-purpose` with the persona inlined until a fresh session loads it natively.
