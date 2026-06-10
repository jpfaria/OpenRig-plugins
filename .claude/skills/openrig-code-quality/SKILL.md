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
❌ `git worktree add .solvers/issue-N …` — worktrees share the parent `.git`; use a clone/copy so each issue lives in a fully independent tree
❌ Editing files directly under the user's working tree (top-level `plugins/`, `docs/`, `.claude/`) instead of `.solvers/issue-{N}/`
```

---

## LAW — development flow

1. **There must always be an issue.** No work without an issue tracking it.
2. **Comment the plan on the issue** before starting any change.
3. **Isolated workspace** — `.solvers/issue-{N}/` MUST be an **independent working tree** with its own `.git`. NEVER edit in the user's working directory. **`git worktree add` is forbidden** — worktrees share the parent `.git`, which leaks state (refs, index, hooks) between the two trees and defeats the isolation. Use a clone or copy instead. Canonical setup:

   ```
   git clone . .solvers/issue-{N}
   cd .solvers/issue-{N}
   git remote set-url origin git@github.com:jpfaria/OpenRig-plugins.git
   git fetch origin main && git reset --hard origin/main
   git checkout -b feature/issue-{N}
   ```
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

## Per-capture noise gate — measure the threshold, never guess (issue #73)

High-gain NAM captures amplify the idle input noise floor (pickup / cable /
interface, "powered on, not playing"), so the block hisses the moment it is
enabled. The fix is the engine's existing `dsp::noise_gate` (sits BEFORE the
model in `cpp/nam_wrapper.cpp`), shipped per capture in the manifest. The
manifest schema + `from_package` plumbing is **OpenRig#675** (core); the
per-capture **values** are this repo's job. Schema:

```yaml
noise_gate:                 # manifest-level default (optional)
  enabled: true
  threshold_db: -32.0
captures:
- file: captures/dirty_a2.nam
  noise_gate:               # per-capture override (preferred — see below)
    enabled: true
    threshold_db: -32.0
```

`noise_gate.threshold_db` is **input-referred dBFS** (same unit the engine
already uses). The engine default is `enabled: false`, `-50.0` (#612 turned the
gate off to protect sustain). Precedence: user project > per-capture > manifest
default > engine default.

**The measurement IS the deliverable — `tools/loudness_audit/src/nam_gate_audit.rs`:**

- **Probe:** deterministic white noise at **−50 dBFS RMS** (the idle floor),
  run through the raw `.nam`; measure output RMS.
- **Decide on the AUDIBLE LEVEL, not the gain.** Small-signal gain is NOT a
  clean/dirty discriminator — across the catalogue its median is **+26 dB**
  (even a clean amp channel amplifies a −50 dBFS signal; a preamp has gain at
  tiny levels). What the user hears is `idle = out_rms + output_gain_db`. Ship
  the gate when `idle ≥ cutoff` (issue #73 chose **−20 dBFS**, the clearly-
  audible tier: 228/524 plugins); quieter captures keep the #612 off-default so
  their sustain is never strangled.
- **Threshold from a closed-loop simulation of the ENGINE'S ACTUAL gate —
  never from the static law.** The first pass inverted the gate's static
  formula (`reduction_dB = 0.1·(threshold − level)²`) and produced a
  near-constant threshold that did almost nothing for the hottest captures:
  the real gate (`dsp::noise_gate`, 10 ms mean-square follower +
  open/hold/close state machine) barely engages until the threshold is
  within ~15 dB of the idle level. `nam_gate_audit` ports that gate
  byte-faithfully and, per capture, runs `gate(probe, T) → model`:
  - binary-search `t_idle` = the gentlest T whose gated idle output falls
    to **−35 dBFS** (`IDLE_TARGET_DBFS`);
  - binary-search `t_sustain` = the hardest T whose gated synthetic DI
    keeps its integrated LUFS within **0.5 dB** (`SUSTAIN_TOL_DB`);
  - ship `threshold = min(t_idle, t_sustain)`, bounded to `[−45, −30]`
    (`T_LO`/`T_HI` — the ceiling keeps the gate below soft playing, which
    the loud DI cannot probe). Every value is measured against that
    specific model; there is NO uniform formula.
  `--probe <model.nam>` prints the full per-T curve for one capture (the
  per-plugin validator).
- **Writer:** `nam_gate_audit --apply <report.tsv>` upserts the per-capture
  `noise_gate` block (idempotent: strips a stale block first, so a re-run at a
  different cutoff converges). This mirrors the tested `output_gain_db` writer
  in `main.rs` — it is **not** the forbidden ad-hoc manifest transform script;
  it has unit tests and the `pack_plugins`/`plugin-loader` parse over all 524
  is the safety net.

**Build traps this work hit (the next session will hit them too):**

- **`.cargo/config.toml` must stay gitignored.** The local A2-core `paths`
  override was accidentally committed in #62, pointing at a non-existent
  `../openrig-dev` → it breaks every clean `cargo` build and the OpenRig release
  bundle. Removed + gitignored in #73. If you need the override locally, keep it
  out of git.
- **A2 models build from a fresh `develop` resolve.** With no `Cargo.lock`
  (it is gitignored) cargo re-resolves `develop` and builds the A2-capable
  NAM C++ core — no segfault, no `openrig-dev` clone needed. The old stale-lock
  segfault recipe is obsolete.
- **rpath patch.** `qa_audit` / `nam_gate_audit` link `@rpath/libnam_wrapper.dylib`
  with no `LC_RPATH`. After building, `cp` the dylib from
  `target/release/build/nam-*/out/lib/` into `target/release/` and
  `install_name_tool -add_rpath @executable_path target/release/<bin>`.

```
❌ threshold_db picked by ear / a round number / copied from the schema example
❌ deciding "needs gate" from small-signal gain (over-selects ~90% of catalogue)
❌ deriving threshold_db from the gate's STATIC law — the real follower/state
   machine behaves differently; simulate the engine gate in closed loop
❌ an ad-hoc python/sed script rewriting 228 manifests
✅ probe → measure audible idle → closed-loop t_idle/t_sustain search → tested --apply writer
```

---

## LAW — parameter names are REAL controls; read the description (issue #66)

A plugin's parameter NAME must be a control that actually exists on the gear —
an amp/preamp/pedal knob or switch (`gain`, `drive`, `tone`, `level`, `treble`,
`bass`, `mid`, `presence`, `master`, `volume`, `depth`, `reverb`, `channel`,
`mic`, `voicing`, `mode`, `boost`, `bias`, `comp`, `blend`, `filter`, `stab`,
`balance`, `transistor`, `voltage`, `feel`, `hf`, `load`, …) — or, for **IR**
plugins (cabs AND acoustic-guitar bodies), a real mic-ing / version axis
(`mic`, `position`, `distance`, `version`, `flavor`, `pickup`). The sanctioned
catch-all is `preset` (a genuine grab-bag) and the single-capture sentinel is
`default`.

**FORBIDDEN as a parameter name:** anything that is not a real control —
especially NAM training/capture metadata (`epochs`, `train`, `capture`,
`buffer`, `nam_size`, `arch`, `block`, `loop`, `module`, `take`), and the
invented abstractions (`model`, `size`, `variant`, `setting`, `version` on an
amp, `flavor` on an amp). If you reached for one of these, you did not decode
the real control — go back to the filename and the description.

**Two sources of truth, not one:** the capture **filename** AND the tone3000
**description**. Many tones spell the dial settings out only in the description
(e.g. *"File numbers = Presence, Bass, Middle, Treble, Volume I, Volume II"*,
*"everything at 12 o'clock"* = noon = 5, *"BCL_HG_2: Gain 5, Bass 7…"*).
Reading only the filename is how the #66 import produced 120+ plugins with
invented/metadata axis names. Always fetch and read both:

```
curl …/rest/v1/tones?id=eq.<id>&select=title,description
curl …/rest/v1/models?tone_id=eq.<id>&select=name,model_url
```

**A real knob axis is ALWAYS numeric — it may NEVER hold a string.** A knob
(`gain`/`bass`/`treble`/`volume`/`mid`/`middle`/`presence`/`master`/`level`/
`depth`/`reverb`/`cut`/`sustain`/`contour`/`drive`/`tone`/`dist`/`fuzz`/`sag`/
`bias`/`output`…) must hold numeric values. When you find string values on such
an axis, ONE of two things is true:

1. **They are knob POSITIONS** → decode to numbers: `noon`=5, `9_oclock`≈2.5,
   `3_oclock`≈7.5, fully-CW/`max`/`full` = the knob TOP for THAT pedal (TS808
   drive 0–10 ⇒ `max`=10); clock×100 `Tone900`=9.0, `1030`=10.3; concatenated
   `555`=5/5/5; underscore-decimal `8_5`=8.5, `2_0`=2.0; `off`/`min` = 0;
   **absent control (knob not present on this capture/channel) = `-1`** (a
   numeric sentinel, distinct from a real `0`); qualitative `low`/`mid`/`high`
   knob positions = `3`/`5`/`8`.
   **ORDERED positions encoded as labels are a KNOB, not an enum** — if the
   values are an N-step gain sweep written as `low1`/`low2`/`mid1`/`mid2`/`high1`
   /`high2`/`high3` (Marshall JVM: 8 gain levels) or `lg`/`mg`/`hg`, they form one
   numeric gain knob: order them lowest→highest and number `1..N`. Be smart — a
   set that clearly ranks is a knob, not a dropdown.
2. **They are NOT knob positions but a genuinely DISCRETE, UNORDERED selector —
   VOICINGS / CHANNELS / MODES / INPUTS / PEDALS** (`clean`/`crunch`/`od`,
   `in1`/`in2`, `standard`/`ultra_lo`/`ultra_hi`, pedal names…) → the **axis is
   MISNAMED**. Rename it to the right enum (`voicing`/`channel`/`mode`/`input`/
   `gain_stage`/`pedal`); the string values stay (it is a selector, not a knob).

Never list a value twice in an axis. Numbered hand-picked configs → one `preset`
axis, not sparse EQ knobs. **Numbered values are PLAIN integers** — never
zero-padded (`01`→`1`, `08`→`8`): leading zeros also hit the YAML octal trap
(`08`/`09` silently become strings while `01`–`07` become ints).

**Enforcement:** `scripts/param_gate.py` is the deterministic gate for this —
it flags any non-control axis name, any value that is not in the
filename+description, decimals written `N_M` (parse wrong), multi-knob enum
values, leftover tone3000 hash filenames, and capture data loss vs the baseline
commit. Run it; RED is a defect, not an opinion. See the canonical
`openrig-manifest-parameters` skill for the full derivation method.

```
❌ name: epochs / take / flavor(on an amp) / model / variant   → not a control
❌ decoded only from the filename, ignored the description       → missed settings
❌ value 8_5 (parses as 85/string)                               → write 8.5
❌ knob value `max` / `noon` / `9_oclock` / `900`               → decode to the number
✅ name ∈ real controls; values cross-checked against filename + description
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
