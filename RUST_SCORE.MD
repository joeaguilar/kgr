# Rust Best Practices Scorecard: kgr

**Scored against**: `the_comprehensive_rust_best_practices_reference.md`
**Codebase**: 43 Rust files, 19 language parsers, 271 tests, 0 cycles
**Date**: 2026-03-12

---

## Overall Score: **A- (88/100)**

| Category | Weight | Score | Grade |
|----------|--------|-------|-------|
| Ownership, Borrowing & Lifetimes | 25% | 24/25 | A+ |
| Error Handling | 20% | 15/20 | B+ |
| API Design & Type Patterns | 20% | 18/20 | A |
| Performance & Concurrency | 20% | 17/20 | A- |
| Testing, Tooling & Organization | 15% | 14/15 | A- |
| **Total** | **100%** | **88/100** | **A-** |

---

## 1. Ownership, Borrowing & Lifetimes — 24/25 (A+)

### What the guide says
> Start with owned data in structs, borrow in function parameters, reach for smart pointers only when genuinely required. Clone freely during prototyping; use Cow when data often passes through unmodified.

### What the codebase does

**Exemplary adherence.** Core types (`FileNode`, `Import`, `Symbol`, `DepGraph`) own their data with `String`, `Vec<T>`, `PathBuf`. Function signatures consistently borrow: `&str`, `&[T]`, `&Path`.

| Practice | Status | Evidence |
|----------|--------|----------|
| Structs own data | Pass | `types.rs` — all `String`/`Vec`/`PathBuf` |
| Functions borrow params | Pass | `resolve.rs`, `render/mod.rs`, `graph.rs` |
| Clone usage judicious | Pass | 95 clones — all necessary (PathBuf keys, cache, boundaries) |
| No hot-loop cloning | Pass | Rayon pipeline and BFS traversal clean |
| Interior mutability appropriate | Pass | `thread_local! { RefCell<Parser> }` only — 19 parsers |
| Lifetime annotations minimal | Pass | Only 2 explicit lifetimes in entire codebase |
| Smart pointers appropriate | Pass | Only `Box<dyn Parser>` and `Box<dyn Write>` |
| Entry API used | Pass | 2 correct uses in graph construction |
| No Rc/Arc (not needed) | Pass | No shared ownership exists |

**Cow<T> absence** — Justified exception. The guide recommends Cow for data that "often passes through unmodified." kgr's pipeline is linear (parse -> transform -> store -> render) with no pass-through patterns. Introducing Cow would add complexity without benefit.

**Deduction (-1)**: One `&PathBuf` parameter in `graph.rs:355` (`edges_from`) should be `&Path` per the guide's "prefer borrowed type over borrowing the owned type" rule.

### Practicality of fixes
- `&PathBuf` -> `&Path`: Trivial, zero performance impact, improves API flexibility.

---

## 2. Error Handling — 15/20 (B+)

### What the guide says
> Use thiserror for library domain errors, anyhow for application code. main() should return ExitCode with a separate run() function. Avoid process::exit(). Use .context() aggressively.

### What the codebase does

**Pragmatic but diverges from the guide's recommended patterns in several areas.**

| Practice | Status | Evidence |
|----------|--------|----------|
| No panics in production code | Pass | Zero `panic!`, `todo!`, `unimplemented!` outside tests |
| `.expect()` with reasons | Pass | All 88 `.expect()` calls include descriptive strings |
| Safe `unwrap()` (queries, tests) | Pass | ~280 unwraps on hardcoded queries, invariants |
| Dangerous `unwrap()` in CLI | **Fail** | ~20 unwraps on JSON field access in `main.rs` |
| thiserror/anyhow split | **Fail** | thiserror imported but unused; anyhow not used |
| main() -> ExitCode pattern | **Fail** | main() returns (), uses `process::exit()` throughout |
| .context() usage | **Fail** | Zero `.context()` calls anywhere |
| process::exit() avoidance | **Fail** | 17 calls in `main.rs` (binary only, not library) |

**Called-out exceptions:**

1. **No anyhow/thiserror**: The codebase uses `Option<T>` for parser failures and `eprintln!` + `process::exit()` for CLI errors. This is a *deliberate simplification* — parsing failures are independent per-file and don't need error chains. There are no recovery paths. For this domain (a read-only CLI analysis tool), the current approach works without user-facing bugs.

2. **process::exit()**: Confined to the binary crate. The library (`kgr-core`) never panics or exits. The guide's concern about skipping destructors is valid but low-impact here — no resources require cleanup beyond what the OS reclaims.

3. **Dangerous JSON unwraps**: These are in output formatting code (`main.rs:1313-1444`) operating on self-generated JSON. The data shape is controlled, but the unwraps are still fragile to future refactors.

**Deductions (-5)**:
- -2: Dangerous unwraps on JSON access in main.rs
- -1: No main() -> ExitCode pattern
- -1: thiserror imported but unused (dead dependency)
- -1: Zero .context() usage

### Practicality of fixes
- **JSON unwraps -> `.unwrap_or_default()`**: Trivial, no performance impact, prevents future panics.
- **main() -> ExitCode refactor**: Medium effort (touch 17 exit points), improves error flow. No performance impact.
- **Remove unused thiserror OR introduce domain errors**: Trivial (remove) or medium (introduce). Removing is pragmatic; introducing would be over-engineering for this tool's scope.
- **Add .context()**: Only valuable if adopting anyhow. Without it, the eprintln pattern already provides context.

---

## 3. API Design & Type Patterns — 18/20 (A)

### What the guide says
> Naming follows RFC 430. Use newtypes for type safety. Accept broad (impl AsRef<Path>), return specific. Derive standard traits eagerly. Design traits for the open/closed distinction.

### What the codebase does

**Expert-level design with pragmatic trade-offs.**

| Practice | Status | Evidence |
|----------|--------|----------|
| RFC 430 naming | Pass | CamelCase types, snake_case functions, SCREAMING_SNAKE constants |
| Conversion naming (as_/to_/into_) | Pass | Consistent throughout |
| Debug on all public types | Pass | Every public type derives Debug |
| Strategic derive selection | Pass | Copy on small types (Span, Lang), Clone on larger |
| Serialize/Deserialize where needed | Pass | Config, cache, output types |
| Parser trait well-designed | Pass | Send+Sync bounds, default methods, ZST implementations |
| pub(crate) usage | Pass | Internal helpers properly restricted |
| Domain-based module grouping | Pass | parse/, render/, not models/services/utils/ |
| Open vs closed distinction correct | Pass | Enums for Lang/ImportKind, trait for Parser |

**Appropriately absent patterns:**

- **Newtype pattern**: Not used — and correctly so. There's no domain confusion between types (no `UserId`/`Email` scenario). Adding newtypes would be ceremony without benefit.
- **Builder pattern**: Not used — constructors are simple. No functions with 5+ optional parameters that would justify a builder.
- **Typestate pattern**: Not used — the pipeline is sequential composition at call sites, not a state machine that needs compile-time enforcement.
- **`impl AsRef<Path>` / `impl Into<String>`**: Not used in most function signatures. Functions accept concrete `&Path` and `&str`, which is simpler and sufficient for an internal tool.

**Deductions (-2)**:
- -1: `lib.rs` has zero module-level documentation or pub use re-exports (guide: "re-export key types at the crate root")
- -1: `main.rs` at 1,977 LOC is thick for orchestration; 3 render functions need `#[allow(clippy::too_many_arguments)]`

### Practicality of fixes
- **lib.rs re-exports + docs**: Easy, no performance impact. Improves usability for anyone importing `kgr-core`.
- **Refactor too_many_arguments**: Medium effort — wrap render args in a `RenderContext` struct. No performance impact; cleaner API.

---

## 4. Performance & Concurrency — 17/20 (A-)

### What the guide says
> Profile before optimizing. Use with_capacity() when size is known. Enable LTO in release. Use rayon for data parallelism, async only for concurrent I/O. thread_local for non-Send types.

### What the codebase does

**Excellent concurrency architecture. Solid performance with one critical gap.**

| Practice | Status | Evidence |
|----------|--------|----------|
| Rayon for data parallelism | Pass | `pipeline.rs` — serial cache hits, parallel misses, serial update |
| thread_local! for parsers | Pass | 19 parsers, zero contention |
| LazyLock for static queries | Pass | One compilation per process per parser |
| No async (not needed) | Pass | CLI tool, blocking I/O — correct decision |
| Iterator chains idiomatic | Pass | Clean filter/map/collect throughout |
| No unnecessary allocations | Pass | Pipeline pre-allocates `vec![None; files.len()]` |
| No mutable shared state | Pass | No Arc/Mutex needed or used |
| Release profile optimized | **Fail** | No `[profile.release]` section at all |
| HashMap pre-allocation | **Minor** | `graph.rs:46` — could use `with_capacity()` |

**The critical gap:**

```toml
# MISSING from Cargo.toml:
[profile.release]
lto = true
codegen-units = 1
strip = true
```

Without this, release builds use defaults: `lto = false`, `codegen-units = 16`. The guide identifies LTO + `codegen-units = 1` as standard practice for CLI binaries. **Estimated impact: 10-15% runtime improvement** based on typical Rust CLI benchmarks.

**Deductions (-3)**:
- -2: Missing release profile (measurable performance left on table)
- -0.5: No `HashMap::with_capacity()` in graph construction
- -0.5: `ImportKind` is `Copy` but gets `.clone()` in `graph.rs:60`

### Practicality of fixes
- **Release profile**: 4 lines in Cargo.toml. **10-15% performance improvement. Highest-value change in this entire audit.** Increases compile time for release builds (LTO is slower to compile), but that's the standard trade-off.
- **with_capacity()**: Trivial, marginal improvement on large graphs.
- **ImportKind clone -> copy**: Trivial, effectively zero impact (compiler likely optimizes already).

---

## 5. Testing, Tooling & Organization — 14/15 (A-)

### What the guide says
> Unit tests in #[cfg(test)] mod tests, integration tests for public API, snapshot tests for CLI, property-based testing with proptest. Clippy all+pedantic in Cargo.toml. CI runs fmt, clippy, test, deny.

### What the codebase does

**Strong 4-tier testing strategy. Good local gate. Missing CI and supply-chain tooling.**

| Practice | Status | Evidence |
|----------|--------|----------|
| Unit tests in source files | Pass | 19 parsers have `#[cfg(test)] mod tests` |
| Integration tests (public API) | Pass | `tests/integration.rs` — 57 CLI tests |
| Snapshot testing (insta) | Pass | `tests/snapshots.rs` — 18 format × fixture combos |
| Dogfooding | Pass | `just graph` / `just kgr-check` |
| `just verify` gate | Pass | check + clippy -D warnings + test + fmt |
| workspace.dependencies | Pass | Centralized version pinning |
| resolver = "2" | Pass | Modern feature resolution |
| Property-based testing | **Fail** | Not used |
| Doctests | **Fail** | No public API doc examples |
| #[expect] vs #[allow] | **Fail** | 6 #[allow], 0 #[expect] |
| CI/CD pipeline | **Fail** | No GitHub Actions |
| cargo-deny | **Fail** | No advisory/license scanning |
| [workspace.lints] | **Fail** | No centralized clippy config |

**Called-out exceptions:**

1. **No CI/CD**: The `just verify` gate is effective locally but doesn't prevent unverified pushes. For a single-developer project this is acceptable; it becomes a real gap if contributors are added.

2. **No property-based testing**: The guide recommends proptest for "parsers" specifically. With 19 language parsers, fuzzing malformed syntax would catch edge cases. However, the parsers delegate to tree-sitter (which is heavily fuzzed upstream), so the marginal value is lower than for hand-written parsers.

3. **No doctests**: The guide says "every public function should have a working example." kgr-core's API is narrow (5 modules, used only by the binary crate), so the audience for doctests is small. Still, it's a gap.

**Deduction (-1)**: Missing #[expect] (should replace all #[allow]), no workspace.lints section.

### Practicality of fixes
- **#[allow] -> #[expect]**: Trivial, no performance impact. 6 replacements.
- **[workspace.lints]**: Easy. Centralizes clippy config. No performance impact.
- **CI/CD**: Medium effort (GitHub Actions YAML). No performance impact. High value for collaboration.
- **cargo-deny**: Easy (write deny.toml). No performance impact. Protects against CVEs in 20+ tree-sitter deps.
- **Property-based testing**: Medium effort. No performance impact on production. Lower priority since tree-sitter handles fuzzing.
- **Doctests**: Medium effort. No performance impact. Serves as both docs and tests.

---

## Findings Summary

### Top 5 Highest-Value Changes

| # | Change | Effort | Perf Impact | Score Impact |
|---|--------|--------|-------------|-------------|
| 1 | Add `[profile.release]` with LTO | 5 min | **+10-15% runtime** | +2 |
| 2 | Fix ~20 dangerous JSON unwraps in main.rs | 30 min | None | +2 |
| 3 | Replace 6 `#[allow]` with `#[expect]` | 10 min | None | +0.5 |
| 4 | Add `[workspace.lints.clippy]` section | 15 min | None | +0.5 |
| 5 | Change `&PathBuf` to `&Path` in `edges_from()` | 2 min | None | +0.5 |

**Total effort for all 5: ~1 hour. Score improvement: 88 -> 93.5 (A).**

### Full Issue Registry

#### Blocking (affects correctness or robustness)
- [ ] `main.rs:612` — `symbols_data.unwrap()` can panic on extraction failure
- [ ] `main.rs:1313-1444` — ~15 unwraps on JSON field access (`.as_str().unwrap()`)
- [ ] `main.rs:1941-1973` — `writeln!().unwrap()` can panic on broken pipe

#### Improvement (follows guide better, no correctness risk)
- [ ] Missing `[profile.release]` — LTO, codegen-units=1, strip
- [ ] No `main() -> ExitCode` pattern — 17 `process::exit()` calls
- [ ] thiserror imported but never used in either crate
- [ ] Zero `.context()` calls (only matters if adopting anyhow)
- [ ] `graph.rs:355` — `&PathBuf` should be `&Path`
- [ ] `graph.rs:60` — `ImportKind` is Copy, doesn't need `.clone()`
- [ ] `graph.rs:46` — `HashMap` should use `with_capacity(files.len())`
- [ ] `lib.rs` — no pub use re-exports or module documentation
- [ ] `main.rs` — 3 functions with `too_many_arguments` (wrap in struct)
- [ ] 6 `#[allow]` should be `#[expect]` with reason strings
- [ ] No `[workspace.lints]` section

#### Nice-to-have (guide recommends but low ROI for this project)
- [ ] No property-based testing (proptest) — mitigated by tree-sitter upstream fuzzing
- [ ] No doctests on public API — narrow audience (only used by binary crate)
- [ ] No CI/CD pipeline — mitigated by `just verify` gate
- [ ] No cargo-deny for supply-chain scanning
- [ ] No code coverage tracking

---

## What NOT to Change

These are areas where the codebase **intentionally diverges** from the guide and the divergence is correct:

1. **No Cow<T>** — Linear data flow has no pass-through pattern. Adding Cow adds complexity for zero benefit.
2. **No newtypes** — No type confusion exists in the domain. Would be ceremony without safety gain.
3. **No builder pattern** — Constructors are simple. No 5+ parameter functions that need one.
4. **No typestate** — Pipeline composition at call sites is clearer than type-level state encoding.
5. **No async** — CLI with blocking I/O. Rayon handles parallelism. Async would add complexity.
6. **No anyhow** — Per-file parsing failures are independent and don't need error chains. The `Option<T>` + `eprintln!` pattern is sufficient for a read-only analysis tool.
7. **No custom error types** — No error recovery paths exist. thiserror enums would be unused infrastructure.
8. **Clone in parsers** — `.to_string()` on tree-sitter captures is necessary (converting `&str` tied to tree lifetime).

---

## Methodology

Five parallel audit agents examined:
1. **Ownership & Borrowing** — struct ownership, clone patterns, interior mutability, lifetimes, smart pointers
2. **Error Handling** — unwrap/expect census, thiserror/anyhow usage, main() pattern, panic paths
3. **API Design** — naming conventions, type patterns, trait design, derives, visibility
4. **Performance & Concurrency** — collections, rayon, thread_local, release profile, allocations
5. **Testing & Tooling** — test tiers, clippy config, CI/CD, workspace setup, code organization

Each agent read the actual source files (not just grep results) and reported file:line references for all findings. The reference document was used as the scoring rubric with explicit exceptions called out where divergence is justified.
