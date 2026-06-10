# kgr — Handoff

_Last updated: 2026-06-10 (blitz session)_

A continuity note bridging sessions. The **`itr` tracker is the source of truth** for task
detail — this doc is the narrative: where things stand, what's next, and the context you
can't get from code or the tracker alone. Issue IDs (`#N`) link to `itr get N`.

---

## TL;DR

This session ran a **full six-reviewer code review** of kgr (69 new issues filed, #46–#114,
plus #116 found mid-run), then a **12-wave parallel agent blitz** that closed the full
review-batch backlog: 47 issues in Waves 1–8, 7 in Wave 9, 7 in Wave 10, 6 in Wave 11,
and the final 3 in Wave 12. `just verify` is green after Wave 12; hostile env checks
`KGR_EXCLUDE='["**"]' cargo test --workspace` and `KGR_MAX_FILE_SIZE_KB=abc cargo test
--workspace` are also green. Waves 1–11 are committed through `16aa368`; Wave 12 is
sitting uncommitted on `main`. First order of business: review and commit Wave 12.

Wave log with full per-wave outcomes and all 17 interventions:
`sprint/_unscoped/blitz-2026-06-09T22-25-41Z.md`. Blitz epic **#115** is closed.

---

## Repo / branch state

- **`main` @ `16aa368`** — waves 1–11 committed (`fix: close wave 11 review-batch issues`).
- **Working tree** — Wave 12 changed 9 tracked source/test/CI files plus the wave log and
  this handoff, **all gates green** (`just verify` exit 0: check, clippy `-D warnings`,
  54+1+72+26+35+450 tests, fmt). Hostile env checks for `KGR_EXCLUDE` and invalid
  `KGR_MAX_FILE_SIZE_KB` also pass. **Not committed** — review and commit Wave 12.
- `.claude/settings.local.json` — gitignored; gained allowlist entries this session (see
  playbook below). Leave them; waves depend on them.
- `tests/fixtures/**/.kgr-cache.json` litter — eliminated (tests now run `KGR_NO_CACHE=1`).

## What shipped (waves 1–12, 70 issues)

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
- **W11** #68 malformed-baseline error, #69 safe upgrade replacement, #70 zero-file check
  errors, #114 Rust `#[path]` mod resolution, #108 Objective-C `--lang` JSON round-trip,
  #100 all-language e2e coverage + detect round-trip
- **W12** #99 KGR_* test env sanitization, #110 extensionless shebang language detection,
  #103 Linux/macOS/Windows + MSRV CI matrix

---

## Review-batch status

The review-batch blitz is complete. Wave 12 closed the final three issues:

| Issues | Outcome |
|---|---|
| #99 | Host `KGR_*` env is stripped from CLI test subprocesses; config-loading unit tests use a shared clean-env guard; explicit env layering still has coverage |
| #110 | Extensionless files with recognized first-line shebangs are detected and parsed |
| #103 | CI tests stable Rust on Linux/macOS/Windows and checks MSRV 1.81.0 |

## Remaining backlog beyond the review batch (~33 issues)

Not scoped into these waves — run `itr ready` for the live list:
- **Triple-audit findings** under epic #6 (~26 open): resolver semantics (#16/#17/#18/#19,
  #33/#34/#35, #23), rules anchoring (#11), cache key (#12), `--no-external` (#20), objc
  calls (#21), parse failures surfaced (#22), parser drops (#25/#26), cycles/orphans
  (#27/#28/#32), refs/dead features (#37/#38/#41/#42), entry-point modeling (#39/#40),
  product features (#43/#44/#45).
- **Release/install epic #1** (#2–#5): CI versioning, cargo-deny, release smoke tests,
  install.ps1 parity.
- Epics #1 and #6 stay open for release/install and triple-audit work. Epic #115 is closed.

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
git diff --stat                  # review Wave 12
git add -A && git commit -m "fix: close wave 12 review-batch issues"
itr ready -f json --fields id,title,urgency,status
# review-batch blitz is done; next ready work is outside #115
```

- Wave log: `sprint/_unscoped/blitz-2026-06-09T22-25-41Z.md` (config, conflicts, all
  interventions, per-wave outcomes).
- Carried-over context still true from the last session: the resolver is knowingly
  heuristic (`crate_src_base`, `module_dir` — see #33/#35 before "cleaning up"); kgr
  detects a real 20-node cycle in its own `parse/` module (#32) — break or baseline it;
  clippy 1.95 is stricter than older toolchains.
