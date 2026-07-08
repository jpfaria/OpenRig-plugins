---
name: openrig-check-updates
description: Use when asked which plugins are outdated / behind / stale, to check for upstream updates or drift, or whether a tone3000 capture was re-trained or gained models. Covers LV2/VST3 submodule drift and NAM/IR tone3000 sources in the OpenRig-plugins repo.
---

# OpenRig-plugins — Check for Updates

## Overview

`scripts/check_updates.py` is the single, repeatable answer to "which plugins
are outdated?". Do NOT hand-roll `git ls-remote` loops or guess the tone3000
endpoint — the tool already encodes both checkers. It is **report-only**: it
never edits manifests and never touches shared state.

## Run it

Network (`git ls-remote`, the tone3000 API) fails under the default sandbox —
**run with the sandbox off.**

| Goal | Command |
|---|---|
| Everything | `python3 scripts/check_updates.py` |
| LV2/VST3 only (fast, deterministic) | `python3 scripts/check_updates.py --submodules` |
| NAM/IR only | `python3 scripts/check_updates.py --tone3000` |
| Machine-readable | add `--json` |
| Investigate without moving the baseline | add `--no-write-state` |

A plain `--tone3000` run rewrites `scripts/.update_state.json` (the fingerprint
baseline). Use `--no-write-state` while merely investigating so you don't
silently advance it before deciding anything. Commit the refreshed baseline
only when you intend future runs to diff against it.

## Read the output

**Submodules (LV2/VST3) — deterministic, the real "get up to date" signal:**
- `behind` — pin is a branch/non-tag commit behind the upstream HEAD.
- `new-tag` — pin is exactly on a release tag and a newer tag exists.
- `current` / `err` — up to date / remote unreachable.

**tone3000 (NAM/IR) — flags differ sharply in reliability:**
- `changed-since-last-check` / `removed-upstream` — **hard** signals: the
  upstream fingerprint moved vs the committed baseline (a re-train or a
  deletion). These are the genuinely actionable ones.
- `new-models` — **soft** signal: the tone exposes more distinct captures than
  we imported. We routinely import a *subset* on purpose, so a bare
  `new-models` (no hard flag) is usually the intended state, NOT a regression.
  Never read "N outdated" as "N plugins to fix".
- `unchecked` — manifest has no `sources:` (most IR); can't be checked, not a
  failure.

## Act on it

1. **Submodule `behind`/`new-tag`** → rebuild the recipe via CI
   (`build-libs.yml` `workflow_dispatch recipe=<x> platform=all`, which commits
   the new slots back), then the mandatory gate `cargo run --release --bin
   pack_plugins` → exit 0. See `openrig-code-quality` and the LV2 import flow.
2. **tone3000 with a hard flag** → a methodology-driven re-import of that tone
   (real names + real params, skip-on-doubt). A bare `new-models` → report it
   as intentional-subset status; do not mass-import.
3. **Never auto-touch shared state.** Opening issues, triggering CI, pushing,
   or PRs are shared-state actions — propose them and wait for an explicit
   go-ahead. "Get them up to date" authorizes the work, not the GitHub
   side-effects. Follow the repo development-flow LAW (issue → `.solvers/issue-N`
   → gate → PR on request).

## Extending

Added a submodule or recipe? Update `RECIPE_SUBMODULE` in `check_updates.py`
(and the recipe in `plugin-recipes.tsv`) in the same commit — that map is how a
`deps/` path names its affected plugins.
