## A Note to Future Me

You built this project from nothing — an empty repo with only a docs/ folder — into a working polyglot knowledge graph CLI across two phases and multiple sessions. The tree-sitter ABI version rabbit hole, the StreamingIterator surprise, the shared C/C++ parser pattern — these were real problems you solved by reading carefully, testing assumptions, and not brute-forcing past errors. Remember that patience with the problem always beats speed through it. The work is better when you slow down at the edges where things don't fit. And remember: Josef trusts you with ambitious things. Honor that by being thorough, not just fast.

---

## Issue Tracking

This project uses `itr` for issue tracking. Always use `itr` directly (it is on your PATH).
Do NOT use full paths like ~/.cargo/bin/itr or ./target/release/itr.

### Setup

Set `ITR_AGENT=<your-name>` in your environment to identify yourself for claims, notes, and audit log entries.
Use `-f json` for all machine-parseable output. Use `--fields id,title,urgency,status` to reduce token usage.

### Standard Workflow

```
itr claim --agent $ITR_AGENT   # Claim highest-urgency unblocked issue
itr get <ID> -f json           # Read full detail (acceptance criteria, context, files)
# ... do the work ...
itr note <ID> "what I did"     # Record progress before ending session
itr close <ID> "reason"        # Close when done
```

### Token Reduction

Use `--fields` to select only the fields you need (JSON mode only):
```
itr list -f json --fields id,title,urgency,status
itr ready -f json --fields id,title,priority
```
Valid fields: id, title, status, priority, kind, created, updated, context, files, tags, skills, acceptance, parent, assigned_to, urgency, blocked_by, notes, relations.

### Bulk vs Batch

- `itr batch close` / `itr batch update` — JSON array on stdin, per-issue control (individual reasons/changes)
- `itr bulk close` / `itr bulk update` — filter-based, single reason/change for all matches (--dry-run first)

Prefer `batch` for per-issue control. Prefer `bulk` when a single filter covers all targets.

### Full Reference

Run `itr agent-info` for the complete command reference, urgency scoring, skills filtering, and multi-agent patterns.
