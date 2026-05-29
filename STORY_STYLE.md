# Story Style — kgr

_Last updated: 2026-05-29_

> How this project writes issues, tickets, and stories. Read by `/sprint` Phase 0 and any agent that creates issues for this repo.

## Title & Body

**Title shape:** Mostly imperative for actionable issues; epics may use noun or declarative titles.
**Title length:** Soft cap around 80 chars. Longer titles are acceptable when they preserve important technical specificity.
**Title prefix:** None.

**Body template:**
```text
<context, only when title + acceptance criteria are not enough>
```

**Required sections:** Acceptance criteria for actionable feature, bug, and task issues.
**Optional sections:** Context, files, tags, skills.

## Acceptance Criteria

**Format:** Compact observable outcomes.
**Observability rule:** Criteria should name concrete behavior, files, commands, tests, CI jobs, failure modes, or user-visible effects that an agent can verify.
**DoD reference:** Sprint-specific Definition of Done is appended by `/sprint` when relevant.

## Tags & Priority

**Tag taxonomy:** Flat lowercase tags.
**Common tag prefixes:** None. Use concise topic/area tags such as `ci`, `release`, `audit`, `resolver`, `rust`, `testing`, `install`, or `windows`.
**Priority scheme:** `critical`, `high`, `medium`, `low`.
**Epic linking:** No formal body-link convention. Use tracker parent relationships when needed.

## Language & Voice

**Terminology:** Prefer "issue".
**Voice:** Terse-technical, specific, implementation-aware.
**Banned phrases / anti-patterns:**
- Avoid vague acceptance criteria like "works correctly" without a verifiable signal.
- Avoid long explanatory bodies when the title and AC already carry the issue.
- Do not hide failure modes; name the exact silent failure, regression, or CI gap.

**Domain glossary:**
- **kgr** — the polyglot source dependency knowledge graph CLI.
- **snapshot** — committed expected CLI output used by insta-based tests.
- **baseline** — stored known violations for incremental `kgr check` enforcement.
- **resolver** — logic that maps imports to local, external, or system dependencies.
- **release matrix** — CI jobs that build/package kgr for target platforms.

**Other project-specific notes:**
- Epics may omit acceptance criteria when they only group child issues.
- AC can be one compact field, but separate clauses with semicolons or bullets when several checks are required.
- Prefer naming exact files, commands, config files, and tests when they are known.

## Worked Examples

### Example 1 — feature

Port itr's auto-version workflow: automate semver tagging + release dispatch

**Acceptance criteria:**
- On push to main, a feat/fix/breaking commit produces a new `v*` tag and triggers `release.yml`.
- Commits with no feat/fix and `[skip version]` commits create no tag.
- `release.yml` accepts the dispatched tag input and builds the full matrix.

### Example 2 — bug

Fix silent glob-drop in rules: invalid pattern disarms the boundary check

**Acceptance criteria:**
- `Glob::new` failure emits a warning to stderr naming the rule and bad pattern.
- A misconfigured rule causes `kgr check` to exit non-zero rather than silently pass.
- A test covers an invalid pattern.
