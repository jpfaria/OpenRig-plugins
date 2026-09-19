---
tags: [openrig-plugins, learnings]
created: 2026-09-14
updated: 2026-09-18
source: claude-code-sessions
---

# OpenRig-plugins — Learnings

Gotchas found while working in this repo that are not yet covered by `CLAUDE.md` or
`.claude/skills/openrig-code-quality/SKILL.md`. Promote an entry into the skill when it
becomes part of the standard flow.

## 2026-09-14 — `nam_gate_audit --apply` silently needs `--source`

- **Gotcha / invariant:** `nam_gate_audit --apply <report.tsv>` without
  `--source plugins/source` just prints its usage message and writes nothing.
- **Why it matters:** the run looks finished, but no `noise_gate` block lands in the manifests.
- **Applies to:** every NAM import (#136, #139).

## 2026-09-14 — zsh does not word-split `$VAR` when building the symlink root

- **Gotcha / invariant:** a loop like `for p in $NAMS; do ln -s … ; done` in zsh treats
  `$NAMS` as one word, so the restricted symlink root for `loudness_audit` ends up wrong.
  Use an explicit list or an array (`${=NAMS}` / `for p in a b c`).
- **Why it matters:** the audit then measures the wrong set (or nothing) and the gate
  result is meaningless.
- **Applies to:** any scoped `loudness_audit <root>` run from the default macOS shell.

## 2026-09-14 — `param_gate.py` rewrites `scripts/.tone_cache.json`

- **Gotcha / invariant:** running `scripts/param_gate.py` churns `scripts/.tone_cache.json`.
  Restore it with `git checkout scripts/.tone_cache.json` before committing.
- **Why it matters:** otherwise every import commit carries unrelated cache noise.
- **Applies to:** the param gate on every import.

## 2026-09-14 — concurrent import PRs conflict on `resume.json`

- **Gotcha / invariant:** every import PR edits the counts in `resume.json`, so two open
  import PRs always conflict. The second to merge must merge `main`, recompute the
  counts from the tree and re-run the scoped `qa_audit`.
- **Why it matters:** hand-resolving the numbers produces wrong totals (#137/#138).
- **Applies to:** parallel imports; also several agents writing into the same
  `.solvers/issue-N` clone (they see each other's untracked folders — expected).

## 2026-09-14 — one block type per plugin when importing tone3000 packs

- **Gotcha / invariant:** a plugin has exactly one block type. Packs that mix `[AMP]`,
  `[PRE]` and `[POW]` profiles (e.g. BadCat Lynx, tone 30122) import only one kind; the
  rest are skipped and noted in the manifest header. Only architecture-2 (A2) captures are
  imported; A1 duplicates are skipped.
- **Why it matters:** mixing kinds breaks the block contract in OpenRig.
- **Applies to:** `claude-plugin:openrig-tone3000-fetch` imports.

## 2026-09-14 — issues are closed by hand after merge

- **Gotcha / invariant:** commits and PRs here never use `Fixes #`, so GitHub never closes
  the issue. Close it (with a comment naming the PR) right after the user merges.
- **Why it matters:** merged work kept showing as open issues (#133).
- **Applies to:** every PR in this repo.

## 2026-09-14 — fixes can be orphaned on a branch pushed after its PR merged

- **Gotcha / invariant:** `2f7956c8` (ChowMatrix/BYOD "Blank" knobs) was pushed ~1h after
  PR #118 merged and never reached `main`. When asked "is everything on main", check
  `git branch -r --no-merged origin/main`, not just open PRs.
- **Why it matters:** the fix looked shipped but `main` still had the bug (landed via #141/#142).
- **Applies to:** release/status checks.

## 2026-09-14 — new `output_gain_db` only applies to new blocks

- **Gotcha / invariant:** the manifest `output_gain_db` becomes the default of the block's
  Output knob at creation time. Existing presets keep their old value until the block is
  removed and re-added (for IR, switching the capture also pulls the new value).
- **Why it matters:** a re-level (#143) does not change what users already have saved.
- **Applies to:** any loudness recalibration.

## 2026-09-14 — official speaker IRs cannot ship here

- **Gotcha / invariant:** "IR of speaker X" means the maker's official IRs (e.g. Celestion
  Plus), not community captures of another cabinet with that speaker. Official IRs are
  licensed per user and cannot be redistributed; package them only as a local plugin.
  Celestion DSR files are encrypted and only load in SpeakerMix Pro (no VST3), so they
  cannot run in OpenRig — buy the IR (WAV) version.
- **Why it matters:** committing licensed IRs to this repo would redistribute them.
- **Applies to:** IR imports from paid packs (also the ZETA and 1960a packs in the user's
  `IR-PACK.zip`).

## 2026-09-14 — measure the level peak over the whole render

- **Gotcha / invariant:** the `loudness_audit` level writer must take the peak over the
  entire rendered signal, including the opening attack, and over every capture of a NAM
  plugin, not only the first one. A peak taken from a partial window under-reports and
  the written `output_gain_db` then clips on real playing.
- **Why it matters:** the policy is "as loud as possible without clipping" (peak at
  −1 dBFS, where the end-of-chain limiter starts); a wrong peak breaks it in either direction.
- **Applies to:** `tools/loudness_audit` (`level.rs`, `nam_run.rs`) and any re-level run.

## 2026-09-14 — `brand:` is one snake_case id per real brand

- **Gotcha / invariant:** the same maker was spelled several ways across manifests
  (`ehx` / `electro-harmonix`, truncated ids like `two`, `jet`, `hughes`, misspellings like
  `freedman`, product names like `sansamp` instead of the maker `tech21`). Use one
  snake_case id per real brand, matching the brand ids the app ships, and change only the
  `brand:` field — never the plugin id.
- **Why it matters:** duplicate ids split one brand into several in the app's brand list.
- **Applies to:** every import; check `grep -rh '^brand:' plugins --include=manifest.yaml | sort | uniq -c`
  before adding a new brand.
