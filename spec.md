# spec: `kgr show` / `kgr slice` — print symbol bodies and line windows (and fix definition spans)

**Status:** proposed (validated against installed `kgr v0.2.0-20-gcf0e8f3`, 2026-07-02; no show/slice/body-print command exists; backlog scanned — nearest neighbor is open #46 "kgr context <file:line>", which is scope *analysis*, not body printing — complementary, not overlapping)
**Origin:** transcript mining across ~86 Claude Code sessions (`werkit/claude-reflection-notes.md` finding #3): ~240 `grep -n 'symbol' file` → `sed -n 'X,Yp'` / `awk 'NR>=A&&NR<=B'` chains slicing definitions out of multi-thousand-line files (rustglichur: 340bbdc1×27, 1bc1b853×18, 5412ff00×16, 72934b6a×15; BigGlichur ~63; Panthexia-web/B2B ~11) — plus ~480 raw `rg` symbol hunts that kgr should be absorbing. Killing the two-step chain gives sessions a reason to reach for kgr first.

---

## Part 0 (prerequisite) — capture real definition spans in the index

**Today the span data is a mirage.** `Span {start_line, start_col, end_line, end_col}` exists (types.rs:64-70) and every `Symbol` carries one (types.rs:99-104), but all 19 language parsers capture the span of the **name identifier node**, not the definition node (pattern at rust_lang.rs:148 → `capture.node` is `fn.name`; span taken at rust_lang.rs:168-179). Empirically: **all 588 symbols in this repo's own `.kgr-cache.json` have `end_line == start_line`.**

Change: in each `crates/kgr-core/src/parse/*_lang.rs`, when the query captures a name node, walk to the enclosing definition node (tree-sitter: the capture's ancestor matching the definition kind already targeted by the query — most grammars expose it as the match's outer capture; add an explicit `@def` capture per query rather than heuristics) and take the span from that node. Name stays from the name node; span becomes the full definition extent.

Knock-on effects (both wins):
- **Fixes a live bug:** `hotspots` computes function length as `end_line - start_line + 1` (main.rs:1913) — currently always `1`, so its complexity ranking is silently length-blind. Real spans make it honest.
- Cache: bump `.kgr-cache.json` version so stale name-node spans re-parse (top-level `version` field already exists for this).
- Surface `end_line` in symbol JSON output (`symbols`/`graph --symbols`/`refs` currently emit only `{name, kind, line, exported}` — main.rs:627, 1193, 1244; add `end_line`).

Per-language acceptance: for each of the 19 languages, at least one fixture function whose body spans ≥3 lines indexes with `end_line > start_line`. Languages where a grammar makes this genuinely hard may fall back to name-node span **explicitly** (`span_kind: "name"` marker) rather than silently wrong.

## Part 1 — `kgr show <symbol> [PATH]`

Print the definition body (or bodies) of a symbol, straight from source, located via the index.

```
$ kgr show render_table
── crates/kgr/src/render/table.rs:6-41 (function render_table) ──
   6  pub fn render_table(rows: &[Row], opts: &TableOpts) -> String {
   7      let widths = column_widths(rows);
   …
  41  }
```

| flag | meaning |
|---|---|
| `-c, --context <n>` | include n lines before/after the definition (default 0) |
| `--all` | print every match; default prints the first and lists the others as one-line pointers (`also: src/other.rs:88 (method)`) |
| `-k, --kind <fn\|class\|method>` | disambiguate same-named symbols |
| `-f <text\|json>` | json: `{name, kind, path, start_line, end_line, exported, body}` per match |
| `--no-linenos` | raw body, pipe-friendly |

Semantics:
- Resolution via the same lookup `refs` uses; file reading reuses the plumbing already prototyped at main.rs:1260-1291 (refs' one-line context read) — generalized from one line to the span.
- If the index has a name-only span (Part 0 fallback languages), print the name line plus `--context`-worth of lines and say so (`note: body extent unavailable for <lang>`), never a wrong window.
- Missing symbol: exit 1 with the near-miss suggestions `refs` gives (or plain not-found if refs has none).
- Stale cache: same mtime/size invalidation as every other command; no special handling.

## Part 2 — `kgr slice <file>:<start>[-<end>]`

The dumb half, index-free — replaces `sed -n '4990,5340p'` with numbered, bounded output:

```
$ kgr slice crates/bg-inference/src/lib.rs:4990-5340
$ kgr slice engine.py:1134 -c 20        # single line ± context
```

| flag | meaning |
|---|---|
| `-c, --context <n>` | expand a single-line target both ways (default 10 when no end given) |
| `--no-linenos` | raw text |
| `-f <text\|json>` | json: `{path, start_line, end_line, lines[]}` |

- Accepts `file:start`, `file:start-end`, and `file start end` (positional fallback).
- Out-of-range end clamps to EOF with a `note:` line; nonexistent file exits 2.
- Cap output at 500 lines unless `--max <n>` raises it — this tool exists to keep context lean, not to `cat` files.

## Why both

`show` is the 90% case ("print me `get_out_diff`"), but ~a third of the mined slices targeted *regions* that aren't a single symbol (match arms mid-function, impl-block runs, config blocks in non-code files). `slice` covers those and also works on files kgr doesn't parse (`.toml`, `.md`, logs), since it never touches the index.

## Worked replacements (mined → new)

```sh
# mined: grep -n 'get_out_diff' src/lora.hpp; sed -n "$(…|cut -d: -f1),+60p" src/lora.hpp
kgr show get_out_diff

# mined: awk 'NR>=4990 && NR<=5340 && /InferenceRequest::(Generate|Sprite)/' crates/bg-inference/src/lib.rs
kgr slice crates/bg-inference/src/lib.rs:4990-5340 | grep -E 'InferenceRequest::(Generate|Sprite)'

# mined: awk 'NR>=4360 && NR<=4445' js/17-levelselect.js   (read the enclosing function)
kgr show onLevelSelect -c 5
```

## Implementation notes

- New `Commands` enum variants in crates/kgr/src/main.rs:32ff + dispatch; standard global flags (`-f`, `-l`, `--no-progress`, `-v`) apply.
- `show` cost: index lookup (rayon-parallel parse with mtime/size cache already makes this cheap) + one file read. `slice`: one file read, no index at all.
- Update README (which still advertises 8 languages vs the actual 19 — fix that stale line while in there) and `agent-info` so agents discover `show`/`slice`; discoverability is half the value, per the same lesson as the itr audit.
- Relationship to open #46 (`kgr context <file:line>` scope analysis): unaffected; `show`/`slice` are printers, #46 is semantics. If #46 lands later, it can share slice's location-parsing.

## Acceptance

1. Part 0: fixture per language with a ≥3-line function indexes `end_line > start_line`; kgr's own repo re-indexes with >0 multi-line spans (today: 0 of 588); `hotspots` on kgr's repo shows length values >1.
2. `kgr show render_table` inside this repo prints the real body with correct line numbers (diff against `sed -n` ground truth in a golden test).
3. `kgr show <ambiguous>` lists all matches; `--all` prints all bodies; `-k` filters.
4. `kgr slice` clamps, caps at 500 lines, and round-trips `--no-linenos` output byte-identical to the source window.
5. `-f json` for both commands is stable and documented in agent-info.

## Non-goals

- No syntax highlighting, no pager integration (pipe to `bat` if wanted).
- No editing, no extraction-to-file.
- No cross-file "expand this call chain" — that's `impact`/`refs` territory.
