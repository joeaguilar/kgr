# kgr — Handoff

_Last updated: 2026-06-10 (post-review backlog blitz complete — tracker empty)_

A continuity note bridging sessions. The **`itr` tracker is the source of truth** for task
detail — this doc is the narrative: where things stand, what's next, and the context you
can't get from code or the tracker alone. Issue IDs (`#N`) link to `itr get N`.

---

## TL;DR

**The backlog is empty.** The post-review blitz (epic #117) closed all 26 actionable
tasks across 10 waves, and epics #1 (release/install parity), #6 (triple-audit), and
#117 are closed. Waves 1–4 are committed at `d96ca50`; **waves 5–10 are in the working
tree, uncommitted** — review and commit them. Both gates are green: `just verify`
exit 0, `just kgr-check` "3 violation(s) suppressed by baseline / All checks passed".

Wave log (config, conflicts, interventions, per-wave outcomes, final report):
`sprint/_unscoped/blitz-2026-06-10T16-50-00Z.md`.

---

## Repo / branch state

- **`main` @ `d96ca50`** — blitz waves 1–4 committed (`fix: close post-review backlog blitz waves`).
- **Working tree** — waves 5–10: 17 files changed, ~2,945 insertions, plus new untracked
  `.kgr-baseline.json`. Gate green. **Not committed.**
- Suggested commit: `git add -A && git commit -m "fix: close final post-review blitz waves 5-10"`

## What shipped in waves 5–10 (this working tree)

- **W5** #23 scan root threaded into all `Resolver::new` call sites (tsconfig/go.mod
  load from scanned root, not CWD); #32 intentional 20-node parse/ cycle baselined in
  `.kgr-baseline.json`, `kgr-check` recipe passes `--baseline`.
- **W6** #41 Rust mod/use/pub-use file edges: inline-module-chain rebasing of
  self/super, crate/super anchor files for re-export consumption, glob re-exports
  resolve; lib.rs reachable via re-exports no longer orphan.
- **W7** #35 crate root derived from nearest ancestor `Cargo.toml` (src-dir first,
  scan-root fallback now logged); #39 Cargo targets (build.rs, bins, lib, examples,
  benches, tests) classified as `structural_entries` with stable JSON reasons.
- **W8** #43 `--first-party` flag + `first_party`/`vendor_globs` config: vendored paths
  excluded from heaviest/orphan summaries, JSON `first_party_filter` key when active.
- **W9** #44 `kgr check --exit-zero`: same diagnostics, exit 0 on findings, operational
  errors still nonzero; AGENT_DOCS updated for both #44 and #43.
- **W10** #45 skipped unsupported files reported (grouped by extension, bounded sample)
  in graph/check/orient JSON + stderr; `--lang` filtering never misreported as skipped.

## Carried-over context

- `.kgr-baseline.json` (repo root, NEW — must be committed) records **3 intentional
  cycles** in kgr's own code: the 20-node parse/ cycle, {render/mod,json,table,tree},
  and {rules,main,config,baseline}. The latter two surfaced when #41 made `super::`/
  `crate::` anchoring accurate — they are true positives of a benign pattern. Refresh
  with `cargo run --release -q -- check --no-progress --baseline .kgr-baseline.json --update-baseline crates`.
- Dogfood `just kgr-check` now warns that the 26 `.snap` snapshot files are unsupported
  — that's #45 working as intended, not a regression. If it grates, deny-list `.snap`
  in walk.rs's non-source extension list.
- The resolver is still heuristic where Cargo/tsconfig metadata is absent — but the
  old caveats (#16/#17 Python edges, #35 silent root fallback) are fixed; tests no
  longer need to route around them.
- clippy 1.95 is stricter than older toolchains.
- `.claude/settings.local.json` (gitignored) carries the blitz permission allowlist —
  leave it; future waves depend on it. Wave-agent sandboxes cannot run `just kgr-check`
  or the kgr binary directly — orchestrator verifies dogfood claims itself.

## Blitz process playbook

Unchanged from the previous run and still load-bearing — see the "Blitz process
playbook" section in git history (`git show dafb622:HANDOFF.md`) for the six rules:
permissions/plain-commands, formatter prohibition, stranded-agent recovery, snapshot
hand-editing, test-writing traps, and the hard wave gate. All six held; zero
quarantines and zero formatter wedges this run.

## How to resume

```sh
just verify                      # confirm still green
git status --short               # waves 5-10 + new .kgr-baseline.json
git diff --stat                  # review
git add -A && git commit -m "fix: close final post-review blitz waves 5-10"
itr ready -f json --fields id,title,urgency,status   # currently: empty
```

With the tracker empty, next work comes from new audits, dogfooding feedback, or
product direction — `/roadmap` or a fresh `/spec-writer` pass, not `/blitz`.
