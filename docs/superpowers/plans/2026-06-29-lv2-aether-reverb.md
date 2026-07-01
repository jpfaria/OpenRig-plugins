# Aether LV2 Reverb Import — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add the Aether algorithmic shimmer/cloud reverb to the OpenRig LV2 catalogue as a faithfully-imported, all-slots native plugin.

**Architecture:** Pin Aether as a git submodule under `deps/`, add a build recipe that drives its CMake (GUI disabled) through the existing `do_cmake`/`collect_libs` infra, let `build-libs.yml` CI produce the four platform binaries, capture the build-generated LV2 TTL and binaries into `plugins/source/lv2/aether/`, author a minimal `manifest.yaml` (category + plugin_uri + binaries only — TTL is the author's), and pass the `pack_plugins` gate.

**Tech Stack:** CMake (Aether), bash recipe (`scripts/build-lib-internal.sh`), GitHub Actions matrix CI (`build-libs.yml`), OpenRig LV2 plugin-loader (reads params from the TTL control ports).

## Global Constraints

- **Slot names are law** — only `macos-universal`, `windows-x86_64`, `linux-x86_64`, `linux-aarch64`. Never invent/rename a slot.
- **English everywhere** in code, manifest, commits, issue/PR comments. Chat stays Portuguese.
- **LV2 param policy = fidelity** — do NOT edit the author's generated `.ttl`. The manifest sets only `type` (category), `plugin_uri` (copied verbatim from the generated `aether.ttl`), `binaries:` per slot, and display metadata (`display_name`, `description`, `brand`, `license`, `homepage`).
- **Gate before every push** — `cargo run --release --bin pack_plugins` must exit 0 / `0 failed`. Same gate as OpenRig `release.yml`.
- **Isolated workspace** — all work in `.solvers/issue-94/` clone (this tree). Never the user's main checkout. No `git worktree`.
- **Upstream facts (verified this session):** Aether v1.2.1, MIT, default branch `master`, CMake ≥3.10, bundle `aether.lv2` containing the `aether_dsp` library + `manifest.ttl` + `aether.ttl`. `BUILD_GUI` defaults ON; build with `-DBUILD_GUI=OFF` to drop X11/OpenGL deps (OpenRig provides the UI).

---

## File Structure

- `deps/Aether/` — pinned submodule (created via `scripts/add-dep.sh`).
- `.gitmodules` — gains the Aether entry (written by `add-dep.sh`).
- `scripts/build-lib-internal.sh` — add `build_aether()`, register `aether` in the `PLUGINS=()` array and the `dispatch()` case.
- `scripts/plugin-recipes.tsv` — map `aether` folder → `aether` recipe.
- `plugins/source/lv2/aether/manifest.yaml` — new, minimal LV2 manifest.
- `plugins/source/lv2/aether/data/manifest.ttl`, `.../data/aether.ttl` — build-generated, committed verbatim.
- `plugins/source/lv2/aether/platform/<slot>/aether_dsp.<ext>` — four CI-built binaries.

---

### Task 1: Pin Aether as a submodule and register the recipe

**Files:**
- Create: `deps/Aether/` (submodule), `scripts/build-lib-internal.sh` (modify), `scripts/plugin-recipes.tsv` (modify), `.gitmodules` (modify via add-dep.sh)

**Interfaces:**
- Produces: a `build_aether` recipe selectable via `./scripts/build-lib.sh aether`, emitting `aether_dsp.<libext>` into `$OUTPUT_DIR`.

- [ ] **Step 1: Add the pinned submodule**

```bash
cd "$WORK"   # .solvers/issue-94
./scripts/add-dep.sh Aether https://github.com/Dougal-s/Aether.git
# pins deps/Aether at current master HEAD; record the SHA in the PR body
git -C deps/Aether rev-parse HEAD
```

- [ ] **Step 2: Add the recipe function** (insert next to `build_ojd`, another cmake recipe)

```bash
build_aether() {
    local src="$DEPS_DIR/Aether"
    # GUI off: OpenRig provides the UI; avoids X11/OpenGL on cross-builds.
    CMAKE_EXTRA="${CMAKE_EXTRA:-} -DBUILD_GUI=OFF -DBUILD_TESTS=OFF -DBUILD_BENCHMARKS=OFF" \
        do_cmake "$src"
    collect_libs "$LAST_BUILD_DIR" "aether_dsp"
}
```

- [ ] **Step 3: Register in the `PLUGINS=()` array and `dispatch()`**

Add `aether` to the `PLUGINS=( … )` list, and add to `dispatch()`:

```bash
        aether)           build_aether ;;
```

- [ ] **Step 4: Map the folder in `plugin-recipes.tsv`**

Append (tab-separated, matching existing rows): `aether<TAB>aether` (folder `aether` built by recipe `aether`).

- [ ] **Step 5: Verify the recipe is discoverable and builds the host slot**

Run: `./scripts/build-lib.sh --list | grep -x aether` → expects `aether`.
Run: `./scripts/build-lib.sh aether` (host = macos-universal on this machine).
Expected: a Release build of `aether_dsp.dylib` produced, no GUI/X11 errors. Confirm the build dir also contains `aether.lv2/manifest.ttl` and `aether.lv2/aether.ttl` (the generated TTL — captured in Task 2).

- [ ] **Step 6: Commit**

```bash
git add .gitmodules deps/Aether scripts/build-lib-internal.sh scripts/plugin-recipes.tsv
git commit -m "build(lv2): add Aether reverb submodule + recipe (#94)"
```

---

### Task 2: Capture the generated TTL + author the manifest

**Files:**
- Create: `plugins/source/lv2/aether/manifest.yaml`, `plugins/source/lv2/aether/data/manifest.ttl`, `plugins/source/lv2/aether/data/aether.ttl`

**Interfaces:**
- Consumes: the `aether.lv2/*.ttl` generated by the Task 1 host build.
- Produces: a loadable plugin folder whose `plugin_uri` matches the TTL's `lv2:Plugin` subject.

- [ ] **Step 1: Copy the build-generated TTL verbatim into `data/`**

```bash
mkdir -p plugins/source/lv2/aether/data
BD="$(./scripts/build-lib.sh aether >/dev/null; echo "$BUILD_WORK_DIR/Aether")"  # or locate aether.lv2 under the build dir
cp "$BD"/aether.lv2/manifest.ttl plugins/source/lv2/aether/data/
cp "$BD"/aether.lv2/aether.ttl   plugins/source/lv2/aether/data/
```

- [ ] **Step 2: Read the real plugin URI from the generated TTL**

Run: `grep -E "a lv2:Plugin|doap:name" plugins/source/lv2/aether/data/aether.ttl`
Record the `lv2:Plugin` subject URI verbatim — this is the manifest's `plugin_uri`. Do NOT guess it.

- [ ] **Step 3: Write `manifest.yaml`** (mirror the OJD/zamcomp shape; no `parameters:` block — params come from the TTL)

```yaml
manifest_version: 1
id: lv2_aether
display_name: Aether
description: Aether — algorithmic shimmer/cloud reverb based on Cloudseed.
brand: dougal
license: MIT
homepage: https://github.com/Dougal-s/Aether
type: reverb
backend: lv2
plugin_uri: <EXACT lv2:Plugin URI copied from data/aether.ttl in Step 2>
binaries:
  macos-universal: platform/macos-universal/aether_dsp.dylib
  windows-x86_64: platform/windows-x86_64/aether_dsp.dll
  linux-x86_64: platform/linux-x86_64/aether_dsp.so
  linux-aarch64: platform/linux-aarch64/aether_dsp.so
```

- [ ] **Step 4: Sanity-check the control ports** (fidelity review, no edits)

Run: `grep -E "lv2:name|lv2:symbol|lv2:port|InputPort|OutputPort" plugins/source/lv2/aether/data/aether.ttl | head -60`
Confirm the audio in/out ports and the control (knob) ports are present and that the URI/port count look complete. Record the control-port list in the PR body so the reviewer sees what knobs OpenRig will expose. Do not modify the TTL.

- [ ] **Step 5: Commit**

```bash
git add plugins/source/lv2/aether/manifest.yaml plugins/source/lv2/aether/data
git commit -m "feat(lv2): add Aether reverb manifest + generated TTL (#94)"
```

---

### Task 3: Produce the four platform binaries via CI

**Files:**
- Create: `plugins/source/lv2/aether/platform/{macos-universal,windows-x86_64,linux-x86_64,linux-aarch64}/aether_dsp.<ext>`

**Interfaces:**
- Consumes: the `build_aether` recipe (Task 1) on each CI runner.
- Produces: four committed binaries the manifest's `binaries:` block references.

- [ ] **Step 1: Push the branch**

```bash
git push -u origin feature/issue-94
```

- [ ] **Step 2: Trigger the slot-matrix build (CONFIRM WITH USER FIRST — CI is billable/shared)**

`build-libs.yml` on `workflow_dispatch` (branch) builds all slots and commits the binaries back to the branch. Trigger via `gh workflow run build-libs.yml --ref feature/issue-94` (confirm the exact input the workflow expects — plugin filter `aether` if supported).

- [ ] **Step 3: Watch the run; confirm all four slots are green for `aether`**

Run: `gh run watch` / `gh run view`. Expected: macos-universal, windows-x86_64, linux-x86_64, linux-aarch64 each produce `aether_dsp` and the commit-libs job pushes them under `plugins/source/lv2/aether/platform/<slot>/`.
If any slot fails (e.g. Windows MinGW link, aarch64), capture the log, fix the recipe (per-slot patch in `build_aether`, mirroring the gxplugins per-slot fallback pattern), and re-run. **No silent slot drops** — a missing slot is a finding, not a default.

- [ ] **Step 4: Pull the CI-committed binaries**

```bash
git pull --ff-only origin feature/issue-94
ls plugins/source/lv2/aether/platform/*/aether_dsp.*   # expect 4 files
```

---

### Task 4: Pass the gate and open the PR

**Files:** none new (verification + integration).

- [ ] **Step 1: Run the mandatory gate**

```bash
cargo build --release -p loudness-audit --bin qa_audit
cargo run   --release --bin pack_plugins
```

Expected: `qa_audit: … fail=0`, `pack_plugins: packed N, 0 failed`, exit 0. The `aether` row must show `ok` (it is a linear reverb block — verify it lands in the linear, not nonlinear, QA class). If qa flags it, treat as a real finding; never `QA_AUDIT_SKIP=1` to dodge.

- [ ] **Step 2: Comment the gate result on the issue**

```bash
gh issue comment 94 --body "Push <sha>: aether — 4 slots built; pack_plugins: <result>. Control ports: <list>."
```

- [ ] **Step 3: Open the PR (on user request)**

```bash
gh pr create --base main --head feature/issue-94 \
  --title "import LV2: Aether shimmer/cloud reverb (MIT)" \
  --body "Closes #94. <recipe + slots + gate summary + control-port list>"
```

---

## Roadmap (separate issues/plans after this pilot validates the flow)

- **x42-plugins** — `do_make` per-plugin recipe; `darc` (dyn), `dpl` (dyn/limiter), `whirl` (mod/rotary), `fil4` (EQ). All four slots official upstream. Build headless (skip robtk GUI) to avoid Cairo/X11.
- **DISTRHO-Ports** — `do_meson` recipe building single plugins' LV2 targets; `TAL-Reverb-2` (reverb), `TAL-Dub-3` + `PitchedDelay` (delay), `TAL-Filter-2` (mod), `Luftikus` (EQ). Per-plugin license check required (licenses are individual). Validate the Windows-via-Wine LV2 actually loads.

## Self-Review

- **Spec coverage:** add-dep (T1) ✓, recipe+register (T1) ✓, TTL fidelity capture (T2) ✓, category+uri manifest (T2) ✓, all-four-slots via CI (T3) ✓, gate+PR (T4) ✓. Param-fidelity policy enforced (no TTL edits, T2.S4).
- **Open risks flagged:** Windows MinGW + linux-aarch64 build success unverified upstream for GUI-off Aether (T3.S3 mitigation: per-slot recipe patch). Exact `workflow_dispatch` input name for the CI plugin filter to confirm at T3.S2. Generated-TTL location under the build dir to confirm at T2.S1 (the `aether.lv2/` path inside `$BUILD_WORK_DIR`).
- **No fabricated values:** `plugin_uri` deliberately left as a copy-from-TTL step, not guessed.
