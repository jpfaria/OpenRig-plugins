# OpenRig-plugins — Claude Code

Repository of **precompiled plugin binaries** (native LV2, NAM/IR captures, native VST3) consumed by [OpenRig](https://github.com/jpfaria/OpenRig). Each plugin lives in `plugins/source/<kind>/<name>/` with `manifest.yaml`, `data/` (TTL) and `platform/<slot>/<lib>` — except **VST3**, which ships one cross-platform `.vst3` **bundle** under `bundles/` (a `Contents/<arch>/…` tree carrying every OS). OpenRig hosts VST3 through the plugin's **native editor** (its own IPlugView window) and discovers parameters at runtime from the plugin's `IEditController`, so the `manifest.yaml` uses `type: vst3` (the block kind is the VST3 host itself, not a sonic category) + `backend: vst3` + `bundle:`. For **LV2 and VST3 alike the live plugin owns the parameter set** (LV2: TTL control ports; VST3: `IEditController`) — the manifest never declares ranges/defaults, only an optional `parameters[]` **overlay** that names the block-editor tabs: keyed by `vst3_id` (#117) or by the port's `symbol` (#119), each entry carrying a `group`. No overlay (or no `group`) → the app groups dynamically. `qa_audit` audits only NAM/IR; LV2 and VST3 are validated by clean packing (`validate_package` requires the `.vst3` bundle directory to exist). VST3 is a from-scratch pilot (ChowCentaur, issue #103); the import methodology graduates into `.claude/skills/openrig-code-quality/SKILL.md` once the pilot lands.

## Language — English everywhere (LAW)

All artifacts are written in **English**, no exceptions:

- **Code** — identifiers, comments, log messages.
- **Docs** — every `.md` (this file, `README*`, anything under `docs/`, every `.claude/skills/*/SKILL.md`).
- **Issue/PR comments** — plans, progress updates, summaries.
- **Commits** — messages in English, no `Co-Authored-By`, no `Fixes #`.

Only the live chat with the user stays in Portuguese. Everything that is committed or posted is English.

## Critical invariant — slot is OpenRig's single source of truth

Platform slot names in `manifest.yaml` (`binaries:`) and in the toolchain (`scripts/build-lib.sh`, `.github/workflows/build-libs.yml`) **MUST match exactly** the OpenRig `Lv2Slot` enum (`crates/plugin-loader/src/manifest.rs`):

```
macos-universal · windows-x86_64 · windows-aarch64 · linux-x86_64 · linux-aarch64
```

**NEVER** invent or rename a slot here (e.g. `windows-x64`, `windows-arm64`). The enum is the source; a serde alias on the OpenRig side is forbidden. A divergent slot makes `pack_plugins` fail → the whole OpenRig release breaks (issue #5). Changed a platform? Align to the OpenRig enum **first**.

## Mandatory gate before any push

```
cargo build --release -p loudness-audit --bin qa_audit
cargo run   --release --bin qa_audit -- --source plugins/source --plugins <kind/name,…>
```

Exit 0 / `0 failed`. `qa_audit` is the SAME audio validation the `Bundle
plugins` job in OpenRig's `release.yml` runs (`pack_plugins` invokes it
first there). Red here = red release there.

**The local gate does NOT zip.** Generating the `dist/` archives is
`pack_plugins`' job and it belongs to the OpenRig **release CI**, not to
the per-push loop here — the zip step is pure delivery overhead locally
and only slows the import. Locally we run `qa_audit` (scoped to what
changed) to catch a sonic regression before it ships; the full
`pack_plugins` pack + manifest-schema validation + zip is the release
CI's responsibility. Only run `pack_plugins` here when you specifically
need to reproduce a packing/manifest-parse failure.

`qa_audit` asserts hard thresholds per plugin (clip / silence / DC / HF
aliasing / LUFS sanity, per-class for linear vs nonlinear blocks) and a
chain-summation check. Any failure is a red gate.

**Validating audio by ear is FORBIDDEN.** Every sonic regression that
ships once is encoded as a deterministic threshold in
`tools/loudness_audit/src/qa.rs`. If a defect can be heard, it can be
measured; if it can be measured, it goes here and the gate enforces it
forever. Asking the user "does it sound better now" is a methodology
defect, not a verification step.

Emergency-only bypass: `QA_AUDIT_SKIP=1` skips the QA gate with a clear
warning (honoured by both `qa_audit` and `pack_plugins`). Use only when
the QA tool itself is broken; never to dodge a real failure.

## Development flow (LAW)

1. **There must always be an issue.** No work without an issue tracking it.
2. **Comment the plan on the issue** before starting.
3. **Isolated workspace**: `.solvers/issue-{N}/` MUST be an independent working tree (clone or copy). `git worktree add` is FORBIDDEN — worktrees share the parent `.git` and break the isolation guarantee. Never edit in the user's working directory. See `.claude/skills/openrig-code-quality/SKILL.md` for the canonical setup command.
4. Branch from `main`: `bugfix/issue-N` or `feature/issue-N` (no description suffix).
5. **Everything being done is commented on the issue** — plan before starting, every push (hash + files touched + gate result), and a final summary. The issue is the running log of the work.
6. Gate `cargo run --release --bin qa_audit -- --source plugins/source --plugins <kind/name,…>` → exit 0 before every push (audio validation only, no zip; the full `pack_plugins` pack is the OpenRig release CI's job).
7. **PR targets `main`.** Bugfix merges right after review; PR/merge only on explicit user request.

## Docs in sync with code (LAW)

Documentation is part of the task, not an afterthought. **Any change that alters behavior, API, flow, slot mapping, or the build/pack process MUST update the affected docs in the SAME commit:**

| Layer | Audience | Update when |
|---|---|---|
| `docs/**/*.md` | contributors / users | behavior, build, pack, slot, or platform changed |
| `CLAUDE.md` (this file) | every Claude session | invariant, gate, gitflow, or general rule changed |
| `.claude/skills/*/SKILL.md` | future Claude session | methodology, anti-pattern, gate, or process changed |
| `README*.md` | the world | tagline, plugin list, build/pack instructions changed |

A commit that changes behavior without touching any `.md` is wrong. A skill left stale because "I remember it" is wrong — the next session does not remember. Renamed something? `grep -rn "<old>"` across `*.md`, `README*`, `CLAUDE.md`, and every `.claude/skills/*/SKILL.md`, and fix all of them in the same commit.

## Communication (LAW)

Reply in **1-3 sentences**. No walls of text, no headers/tables/nested bullets in chat, no recap of what the user said. **Act, don't ask** — when the task is authorized, do the whole thing end-to-end (decide sensible defaults, proceed, let the user veto); never stop mid-flow to ask permission or confirm an obvious next step. Detail lives in the issue/commit/file; chat is a one-line pointer. A long message that makes the user skim-and-approve is a defect.

## Checking for updates

"Which plugins are outdated?" has one repeatable answer: `scripts/check_updates.py`
(report-only). It checks LV2/VST3 submodule drift (`git ls-remote` vs the pinned
tag/branch) and NAM/IR tone3000 sources (content-hash fingerprint vs
`scripts/.update_state.json`). Network needs the sandbox off. See
`.claude/skills/openrig-check-updates/SKILL.md` for how to run it, read the flags
(`new-models` is a soft signal, not a regression), and act without touching shared
state. Do NOT hand-roll `git ls-remote` loops.

## Code methodology

See `.claude/skills/openrig-code-quality/SKILL.md` — quality rules for this repo (slot invariant, single source of truth, docs in sync, English everywhere, isolated `.solvers` workflow). Invoke it before any non-trivial action.
