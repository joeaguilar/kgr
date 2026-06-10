# kgr — Handoff

_Last updated: 2026-06-10 (blitz session)_

A continuity note bridging sessions. The **`itr` tracker is the source of truth** for task
detail — this doc is the narrative: where things stand, what's next, and the context you
can't get from code or the tracker alone. Issue IDs (`#N`) link to `itr get N`.

---

## TL;DR

This session ran a **full six-reviewer code review** of kgr (69 new issues filed, #46–#114,
plus #116 found mid-run), then an **8-wave parallel agent blitz** that closed **47 of
them**, a follow-on **Wave 9** that closed **7 more**, and **Wave 10** that closed
another **7**. `just verify` is green after Wave 10. Waves 1–9 are committed through
`93b6f11`; Wave 10 is sitting uncommitted on `main` as 9 tracked files plus 5 new JSON
snapshot files. First order of business: review and commit Wave 10. Then 2 planned waves
(below) clear the remaining 9 review-batch issues.

Wave log with full per-wave outcomes and all 15 interventions:
`sprint/_unscoped/blitz-2026-06-09T22-25-41Z.md`. Blitz epic: **#115**.

---

## Repo / branch state

- **`main` @ `93b6f11`** — waves 1–9 committed (`fix: close wave 9 review-batch issues`).
- **Working tree** — Wave 10 changed 9 tracked files plus 5 new JSON snapshots, **all gates
  green** (`just verify` exit 0: check, clippy `-D warnings`, 48+67+26+35+445 tests, fmt).
  **Not committed** — review and commit before launching Wave 11.
- `.claude/settings.local.json` — gitignored; gained allowlist entries this session (see
  playbook below). Leave them; waves depend on them.
- `tests/fixtures/**/.kgr-cache.json` litter — eliminated (tests now run `KGR_NO_CACHE=1`).

## What shipped (waves 1–10, 61 issues)

- **W1** #49 single-file PATH support, #50 dotted-specifier resolution, #51 py decorated
  symbols, #52 ts/js arrow/generator symbols, #57 cpp inline methods
- **W2** #48 rust `::` calls in refs/dead/impact, #54 Ruby/PHP/Lua/Bash resolver arms,
  #53 php import capture, #58 c/cpp phantom-Class fix, #59 elixir def forms
- **W3** #47 init overwrite guard (`--force`), #55 zig imports, #60 cache-hermetic tests +
  build-fingerprint CACHE_VERSION + `KGR_NO_CACHE`, #89 csharp records/generics, #90 java
  records/interface visibility
- **W4** #46 config fields wired (languages/format/no_progress; dead fields removed),
  #56 go.mod resolution, #61 query/flag test coverage, #85 ruby singleton/scoped symbols,
  #93 scala grouped imports
- **W5** #77 rust mod child-dir-before-sibling, #72 DOT escaping, #75 table LOCAL-OUT
  degree fix, #76 cyclic tree rendering, #87 lua module-table symbols
- **W6** #116 `check --syntax` un-no-op'd (all 19 parsers), #66 `--format` validation
  everywhere, #78 tsconfig alias semantics, #71/#73 mermaid IDs/escaping/determinism,
  #102 vacuous-test hardening
- **W7** #81/#82 ts require-imports + abstract classes, #83/#84 php static refs +
  trait/enum symbols, #86/#111 ruby receivers + load/autoload, #91/#92 cpp template refs +
  c typedef structs, #94 elixir pseudo-call exclusion
- **W8** #62/#105 dead/impact `found:false` + multi-definition contracts, #79 angle-include
  System-kind fix, #95 haskell binds + export lists, #96/#113 objc `@import` + selector fix,
  #97 rust type/union/macro/trait-sig symbols
- **W9** #64/#65/#107 query empty JSON + target validation + selector ArgGroup, #80
  `.mts`/`.cts`/`.mm` detection and TS resolution candidates, #88 bash source first-arg-only,
  #98 zig top-level symbol filtering/classification, #112 python literal dynamic imports
- **W10** #63/#74/#106 direct who-imports + agent-doc synopsis + heaviest `--top`, #109 go
  nested-subpackage resolution, #67 invalid exclude-glob warnings, #104 cache pruning,
  #101 JSON graph snapshots

---

## Remaining waves (the plan — 9 review-batch issues, 2 waves)

Bundling convention: same-file sibling issues share one agent slot (they can't parallelize
anyway); that agent closes all its issue IDs. Within a wave no two slots share a file.

### Wave 11 (6 issues, 4 slots)
| Slot | Issues | Owns |
|---|---|---|
| 1 | #68 + #69 + #70 | main.rs, baseline.rs, tests/integration.rs — corrupted-baseline error, upgrade self-copy, zero-files exit |
| 2 | #114 | parse/rust_lang.rs, resolve.rs — `#[path]` attribute |
| 3 | #108 | types.rs, walk.rs — objc serde/Display alignment (JSON value may change → snapshot touch-ups possible) |
| 4 | #100 | tests/fixtures/** + NEW tests/e2e_languages.rs + detect.rs round-trip test (new test file avoids integration.rs conflict) |

### Wave 12 (3 issues, 3 slots)
| Slot | Issues | Owns |
|---|---|---|
| 1 | #99 | tests/integration.rs, tests/symbols.rs, tests/snapshots.rs — KGR_* env sanitization |
| 2 | #110 | detect.rs, walk.rs — shebang detection |
| 3 | #103 | .github/workflows/ci.yml, Cargo.toml — mac/windows + MSRV CI matrix |

Conflict notes baked into the ordering: #99 must come after every wave that edits the three
test files; #110 after #100/#80 (detect.rs) and #67/#108 (walk.rs); #114 after #109
(resolve.rs). #101 is now closed, so #99's snapshot-file dependency is clear.

## Remaining backlog beyond the review batch (~34 issues)

Not scoped into these waves — run `itr ready` for the live list:
- **Triple-audit findings** under epic #6 (~26 open): resolver semantics (#16/#17/#18/#19,
  #33/#34/#35, #23), rules anchoring (#11), cache key (#12), `--no-external` (#20), objc
  calls (#21), parse failures surfaced (#22), parser drops (#25/#26), cycles/orphans
  (#27/#28/#32), refs/dead features (#37/#38/#41/#42), entry-point modeling (#39/#40),
  product features (#43/#44/#45).
- **Release/install epic #1** (#2–#5): CI versioning, cargo-deny, release smoke tests,
  install.ps1 parity.
- Epics #6, #115 stay open until their children close.

---

## Blitz process playbook (hard-won — read before running waves 9–12)

1. **Permissions.** `.claude/settings.local.json` allows `just verify`, `cargo
   check/test/clippy`, `cargo fmt --check`, `itr *` so background wave agents can
   self-verify and self-close. The matcher denies compound forms (`cmd; echo $?`), pipes
   around allowed commands, and env-prefixed commands (`ITR_AGENT=x itr ...`) — wave prompts
   must say "plain commands only, plain `itr close`".
2. **Formatter rules.** Wave agents NEVER run `cargo fmt` (crate-wide; wipes neighbors) or
   `just update-snapshots`/`cargo insta accept` (bakes in neighbors' breakage). Agents
   hand-fix drift in their OWN files **before their final gate** — this rule (added in W8)
   eliminated the W7 wedges. Orchestrator may run `cargo fmt --all` only when ALL agents
   are terminal.
3. **Stranded agents.** Agents that "wait for a notification" die with their turn —
   sleep-watchers never fire. Prompts must say "retry the gate IN THIS TURN". When an agent
   dies with work done but task open: `git diff -- <its files>` to verify, run the gate
   yourself, close with attribution. If a DEAD agent left fmt drift, hand-fix it
   immediately — it will never self-clear and wedges every live agent.
4. **Snapshots.** Hand-edit only the `.snap` files a change provably affects, derived from
   verified runs. `cargo-insta` is not installed; `INSTA_UPDATE=new` + rename works for
   solo sessions but is forbidden mid-wave.
5. **Test-writing traps.** Python fixtures' edges are distorted by open #16/#17 — write
   edge-dependent tests against TS fixtures or tempdirs. Use the `kgr()` helper in
   integration.rs (sets `KGR_NO_CACHE=1`). Don't bake open bugs (#20 `--no-external`) into
   assertions; reference issue numbers in comments instead.
6. **Gate.** `just verify` exit 0 between waves, no exceptions, even when every agent
   reported green.

---

## How to resume

```sh
just verify                      # confirm still green
git add -A && git commit ...     # land Wave 10 first (branch if you prefer)
itr ready -f json --fields id,title,urgency,status
# then run /blitz and point it at Wave 11 above, or work issues solo
```

- Wave log: `sprint/_unscoped/blitz-2026-06-09T22-25-41Z.md` (config, conflicts, all
  interventions, per-wave outcomes).
- Carried-over context still true from the last session: the resolver is knowingly
  heuristic (`crate_src_base`, `module_dir` — see #33/#35 before "cleaning up"); kgr
  detects a real 20-node cycle in its own `parse/` module (#32) — break or baseline it;
  clippy 1.95 is stricter than older toolchains.
