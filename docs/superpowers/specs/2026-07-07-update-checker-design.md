# Update-checker mechanism — design (issue #107)

## Problem

There is no repeatable way to answer "which of our plugins are outdated?".
Plugins come from two versioned source kinds and both drift after import:

- **LV2 + VST3** — built from git submodules under `deps/`. Our pinned
  commit/tag can fall behind the upstream default branch or a newer release tag.
- **NAM/IR (tone3000)** — captures imported from `tone3000.com/tones/<id>`. The
  source tone can gain new models or have an existing model re-trained after
  import.

Doing this by hand is error-prone, and network calls (`git ls-remote`, the
tone3000 API) run in a degraded sandbox locally, so ad-hoc shell loops fail.

## Goal

One reusable script plus a skill that knows how to drive it and act on the
result. Report-only: it never mutates plugin data and never touches shared
state (no issues/PRs) on its own.

## Non-goals

- Auto-updating submodules or re-importing captures. The checker only
  *reports*; rebuilding/re-importing stays a human-triggered, issue-tracked
  flow (`lv2-import-flow`, tone3000 re-import).
- Storing new provenance in the 330 manifests. The tone3000 fingerprint lives
  in a side cache; manifests are untouched.

## Architecture

`scripts/check_updates.py` — Python, stdlib only, same shape as
`scripts/param_gate.py` (uses `urllib`, no third-party deps). Two independent
checkers behind one CLI; each is a pure function producing a list of typed
records, so they can be unit-reasoned in isolation.

### Checker A — `--submodules` (LV2 + VST3), deterministic

1. Parse `.gitmodules` → `[(path, url)]`.
2. Read the pinned state from `git submodule status`:
   - the pinned SHA;
   - the descriptor in parens: `(v2.11.4)` = exactly on a tag; `(heads/master)`
     = tracking a branch tip; `(v1.7-15-gHASH)` / `(master-1-gHASH)` = N commits
     past a ref (i.e. not exactly on a tag).
3. Per submodule, one `git ls-remote <url>` (HEAD + `refs/tags/*`):
   - **Pinned exactly on a tag** → flag `new-tag` only when a strictly-newer
     release tag exists (version-sorted over tags sharing the pin's tag shape).
   - **Pinned on a branch / non-tag commit** → flag `behind` when the pinned SHA
     differs from the default-branch HEAD.
   - Unreachable remote → `err` (reported, never silently dropped).
4. Map each submodule to the plugins it produces via `scripts/plugin-recipes.tsv`
   (recipe ↔ submodule ↔ catalogue), so the report names affected plugins, not
   just `deps/` paths.

### Checker B — `--tone3000` (NAM + IR), best-effort + fingerprint

1. Collect every manifest under `plugins/source/{nam,ir}/*/manifest.yaml` and
   extract its `sources: .../tones/<id>` ids (312/318 NAM, 13/177 IR carry one).
2. Per tone id, `GET
   api.tone3000.com/rest/v1/models?tone_id=eq.<id>&select=name,model_url`
   (PostgREST; `apikey` + Bearer via the same env token `param_gate.py` uses).
3. **Fingerprint** = the sorted set of `model_url` basenames. The basename is a
   content hash (`75c8229262434946.nam`); a re-train changes it. Compare against:
   - the fingerprint recorded on the previous run
     (`scripts/.update_state.json`, keyed by tone id) → `changed-since-last-check`;
   - the count of captures declared in the manifest → `new-models` when the tone
     now exposes strictly more models than we imported;
   - a hash present in our recorded import but gone upstream → `removed-upstream`.
4. Manifests **without** `sources:` (~164 IR) are reported as
   `unchecked (no provenance)` — explicit, never silently skipped.
5. Refresh `scripts/.update_state.json` with the current fingerprints at the end,
   so the first run establishes the baseline and subsequent runs detect deltas.

### Common contract

- No flag → run both checkers.
- Output: a human-readable table on stdout; `--json` emits the same records as
  JSON for tooling.
- **Exit 0 always** — it is a report, not a gate. `--fail-on-outdated` makes it
  exit non-zero when anything is flagged (for a future CI job); off by default.
- Network requires the sandbox off; the skill documents this.

## Data / state

- `scripts/.update_state.json` (new, git-tracked): `{ "<tone_id>": { "fingerprint":
  ["<hash>.nam", ...] } }`. Committing it makes "changed since last check"
  meaningful across sessions and machines.
- No manifest writes. No `deps/` checkout needed (Checker A only reads
  `.gitmodules` + `git ls-remote`).

## Skill — `.claude/skills/openrig-check-updates/SKILL.md`

Authored through the `superpowers:writing-skills` gate. Covers:

- **When to invoke**: "which plugins are outdated", "check for updates",
  "upstream drift", "did tone3000 re-train X".
- **How to run**: the command, the token env var, and the sandbox-off note.
- **How to read** the report and the three tone3000 flags vs the two submodule
  flags.
- **How to act**: per outdated plugin, *propose* a tracking issue (never
  auto-create — shared-state law), then follow the existing rebuild/re-import
  flow. Cross-links `openrig-code-quality`, `lv2-import-flow`, the VST3 notes.

## Docs in sync

- `CLAUDE.md`: add the checker to the tooling/flow description.
- The new skill is itself the methodology doc.

## Verification

- Checker A against the live `.gitmodules`: a tag-pinned submodule already on the
  newest tag reports `current`; a branch-pinned one behind HEAD reports `behind`;
  an unreachable url reports `err`.
- Checker B against a known tone id: fingerprint is stable across two runs
  (second run reports no change); a hand-edited `.update_state.json` entry forces
  a `changed-since-last-check`.
- `.gitmodules` parser and `plugin-recipes.tsv` mapping covered by a small
  self-check run (dry, offline) over the committed files.
- Gate: `cargo run --release --bin pack_plugins` → exit 0 before push (the
  script adds no manifest/slot surface, so this is a regression guard).
