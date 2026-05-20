---
name: openrig-code-quality
description: Use when writing, editing, or refactoring anything in the OpenRig-plugins repo — manifests, build scripts, workflows, recipes, or docs. Covers the slot invariant, English-everywhere rule, docs-in-sync rule, the isolated .solvers development flow, and the pack_plugins gate.
---

# OpenRig-plugins — Code Quality

This is a repository of **precompiled plugin binaries** (native LV2 + NAM/IR captures) consumed by OpenRig. There is no application test suite here — the artifacts are `manifest.yaml`, `data/`, `platform/<slot>/<lib>`, the build scripts, and the workflows. Apply these rules BEFORE writing, not after. No exceptions.

---

## LAW — English everywhere

Everything that is committed or posted is written in **English**:

- **Code** — identifiers, comments, log/echo messages in scripts.
- **Docs** — every `.md`: `CLAUDE.md`, `README.md`, this `SKILL.md`, anything under `docs/`.
- **Issue / PR comments** — plan, progress, summary.
- **Commits** — message in English, no `Co-Authored-By`, no `Fixes #`.

Only the live chat with the user stays in Portuguese. The moment text leaves the chat (commit, file, issue), it is English. A Portuguese comment/doc/commit is a defect — fix it in the same change.

---

## LAW — docs always in sync with code (same commit)

Documentation is part of the task, not an afterthought. **Any change that alters behavior, the slot mapping, the build/pack process, the recipe set, or the repo layout MUST update the affected docs in the SAME commit:**

| Layer | Audience | Update when |
|---|---|---|
| `README.md` | contributors / users | recipe added, build/trigger flow changed, layout changed |
| `CLAUDE.md` | every Claude session | invariant, gate, gitflow, or general rule changed |
| `.claude/skills/*/SKILL.md` | future Claude session | methodology, anti-pattern, gate, or process changed |
| `deps/DEPS.md`, `scripts/plugin-recipes.tsv` | build pipeline | dependency or recipe↔catalogue mapping changed |

**Why:** the next session (Claude or human) needs to see the real state. Stale docs become a lie that propagates — the next contributor follows the wrong doc and breaks the OpenRig release; the next Claude reads the stale skill and applies a rule that no longer holds.

**How to apply:**
- Before `git commit`, run the mental list: did I change behavior/slot/build/recipe/layout? → which `.md` are affected? → did I update all of them in this commit?
- Renamed a recipe, slot, manifest field, or plugin id? → `grep -rn "<old>"` across `*.md`, `README.md`, `CLAUDE.md`, every `.claude/skills/*/SKILL.md`, `manifest.yaml` files, `scripts/`, `.github/workflows/` — and fix all of them in the same commit.
- Learned a new rule during the session (user feedback, anti-pattern discovered)? → write it into this skill **before** closing the session. Do not trust personal memory — write it.

**Anti-patterns:**
```
❌ A commit that changes behavior/slot/recipe without touching any .md
❌ "I'll update the doc later" — the session ends, the doc is orphaned, the next one breaks
❌ Stale skill because "I remember it" — next session's Claude does not remember
❌ Portuguese left in a doc/comment/commit
```

---

## LAW — development flow

1. **There must always be an issue.** No work without an issue tracking it.
2. **Comment the plan on the issue** before starting any change.
3. **Isolated workspace**: clone/copy into `.solvers/issue-{N}/`. NEVER edit in the user's working directory.
4. Branch from `main`: `bugfix/issue-N` or `feature/issue-N` (no description suffix).
5. **Everything being done is commented on the issue.** The issue is the running log:
   - the plan, before starting;
   - every push: commit hash + files touched + gate result;
   - a final summary when done.
6. Gate green before every push (see below).
7. **PR targets `main`.** Bugfix merges right after review. PR/merge only on explicit user request.

Do not invert the order. A push without the gate green = a red OpenRig release. Work without an issue = no trace of why a binary or manifest changed.

---

## Mandatory gate before any push

```
cargo run --release --bin pack_plugins
```

Exit 0 / `0 failed`. This is the SAME gate as the `Bundle plugins` job in OpenRig's `release.yml`. Red here = red release there. Run it in `.solvers/issue-{N}/`, confirm exit 0, then push, then comment the result on the issue.

**Forbidden** to silence the gate without a real fix: faking the slot, renaming to dodge a check, `--no-verify`. Always root cause or escalate to the user.

---

## Critical invariant — slot is OpenRig's single source of truth

Platform slot names in every `manifest.yaml` (`binaries:`) and in the toolchain (`scripts/build-lib.sh`, `.github/workflows/build-libs.yml`) **MUST match exactly** the OpenRig `Lv2Slot` enum (`crates/plugin-loader/src/manifest.rs`):

```
macos-universal · windows-x86_64 · windows-aarch64 · linux-x86_64 · linux-aarch64
```

The enum is the source. A serde alias on the OpenRig side is forbidden — so a divergent slot here cannot be papered over there; it makes `pack_plugins` fail and breaks the whole OpenRig release (issue #5).

**NEVER** invent or rename a slot here (e.g. `windows-x64`, `windows-arm64`). Changed a platform? Align to the OpenRig enum **first**, then change the manifests and toolchain together (same commit), then update the docs that list the slots.

```
❌ binaries: { windows-x64: ... }      // invented slot — pack_plugins fails
❌ binaries: { windows-arm64: ... }    // invented slot — pack_plugins fails
✅ binaries: { windows-x86_64: ..., windows-aarch64: ... }
```

---

## No trash

- [ ] No serde aliases for old slot/field names — fix the data (the manifest), not the reader.
- [ ] No dead/commented-out script or workflow blocks.
- [ ] No workarounds/hacks to dodge the gate.
- [ ] Renamed something? ALL references updated in the same commit — `manifest.yaml`, `scripts/`, `.github/workflows/`, `*.md`, `plugin-recipes.tsv`.
- [ ] One concern per commit — don't mix a recipe change with a doc rewrite with a slot rename.

---

## Audio QA — never validate by ear (LAW, issue #12)

**Asking the user "does it sound better now?" is FORBIDDEN.** Sonic
regressions are caught by `qa_audit` (deterministic thresholds against
the synthetic DI), gated by `pack_plugins`. Every failure mode that
ships once gets encoded in `tools/loudness_audit/src/qa.rs` with a
passing AND a failing unit test before the fix lands.

**How to apply:**

- Discovered a defect? Add a check + threshold in `qa.rs`, with
  pass/fail tests, BEFORE editing data to fix the catalogue.
- Catalogue defects fixable by data ops (DC remove, peak normalise,
  resample to 48 kHz with windowed sinc) → use `qa_fix`. Defects
  inherent to NAM model output (asymmetric distortion DC, harmonic
  HF) → relax the nonlinear-class threshold based on the observed
  catalogue distribution, never globally and never to silence the
  check.
- A check that flags healthy content (acoustic body IRs being
  brighter than electric cab IRs) is calibrated wrong — split by
  block class (`Linear` vs `Nonlinear`) or per-type, do not loosen
  the linear threshold to mask data defects.
- `QA_AUDIT_SKIP=1` exists for the case where the QA tool itself is
  broken. Using it to dodge a real failure is the same defect as
  bypassing `pack_plugins` and is treated identically.

```
❌ "Load it and tell me if it sounds better" — verification by ear
❌ Loosening a threshold to pass a defect instead of fixing the defect
❌ Adding a check without both a passing and a failing test
✅ Encode the symptom → tune threshold to observed catalogue → gate
```

---

## Communication with the user — terse, objective

User reply default = **1-3 sentences**. No essays.

- Yes/no question → yes/no + one sentence of context if needed.
- Status → one line per item.
- Decision → one direct recommendation. Other options only if asked.
- No headers/tables/nested bullets unless the content is mechanical reference.
- Cut greetings, preamble, recap of what the user just said, "hope this helps".
- Short code block is fine when it IS the requested content (a command, a slot list).

The chat is in Portuguese; the discipline of brevity still applies.

**Anti-pattern — wall-of-text that buys a rubber-stamp.** If a reply is long
enough that the user says they will not read it and will just approve, the
approval is worthless and the work is now unreviewed. The artifact (spec,
issue, file) is the place for detail — chat points to it in one line. A review
gate that requires reading an essay in chat is a fake gate.

```
❌ Pasting a spec/issue/design summary into chat for "approval"
❌ Re-explaining in chat what the committed file already says
✅ "Spec: <path>, commit <hash>. Approve to proceed?" — detail lives in the file
✅ Findings → the issue/spec; chat = one-line pointer + the single open decision
```

---

## Living Document

This skill is a LIVING DOCUMENT. Every time the user corrects a methodology mistake:
1. Identify the violated principle.
2. Add a rule or anti-pattern to this skill.
3. Commit the updated skill in the same change.

This ensures the same mistake is never repeated.
