# OpenRig-plugins

Catalogue + build pipeline for every plugin OpenRig ships. Four backends coexist under `plugins/source/`:

- **`lv2/`** — 103 native LV2 plugins (effects, EQ, dynamics, modulation, delay, reverb, filter, wah). One `.so` / `.dylib` / `.dll` per supported platform plus `manifest.yaml`, built by the recipes in `scripts/build-lib-internal.sh`.
- **`nam/`** — 274 Neural Amp Modeler captures (preamp, amp, gain pedal). Each plugin is a single `.nam` model + `manifest.yaml`; no native binary, the runtime loads via the bundled `libNeuralAudioCAPI`.
- **`ir/`** — 134 impulse responses (cab + acoustic body). Mono `.wav` at 48 kHz + `manifest.yaml`; convolved by the engine.
- **`vst3/`** — native VST3 plugins. One cross-platform `.vst3` **bundle** (a `Contents/<arch>/…` directory tree carrying every OS) under `bundles/` plus `manifest.yaml`. OpenRig hosts these through the plugin's **native editor** (its own IPlugView window) and discovers parameters at runtime from the plugin's `IEditController`, so the manifest `parameters[]` block stays empty (unlike NAM/IR grids or LV2 TTL ports). Also built by the recipes in `scripts/build-lib-internal.sh`.

The full canonical catalogue — every `MODEL_ID` with display name, brand, and parameter schema — is in [`docs/blocks-reference.md`](docs/blocks-reference.md), auto-generated from the manifests by `scripts/gen_quick_reference.py`. The `openrig-plugins.zip` consumed by the OpenRig installer is produced from this tree by `scripts/bundle-into-openrig.sh`.

## Triggering a build

The build pipeline (`.github/workflows/build-libs.yml`) runs only when one of two things happens. There is no push-triggered auto-rebuild — every run is intentional.

### Option 1: Push a `plugins-build-*` tag

The tag name encodes the scope. Peel a platform prefix off the front (if any), then a recipe (if any); the trailing integer is an arbitrary build counter so two runs of the same scope don't collide on the tag namespace.

| Tag | Scope |
|---|---|
| `plugins-build-1` | every recipe, every platform |
| `plugins-build-windows-x86_64-1` | every recipe, only `windows-x86_64` |
| `plugins-build-artyfx-1` | only the `artyfx` recipe, every platform |
| `plugins-build-windows-x86_64-artyfx-1` | only `artyfx`, only `windows-x86_64` |
| `plugins-build-macos-universal-dragonfly-reverb-2` | `dragonfly-reverb` on macOS |
| `plugins-build-mda-lv2-7` | `mda-lv2` on every platform |

Recipe names that contain dashes (`dragonfly-reverb`, `caps-lv2`, `mod-utilities`, `mda-lv2`) work as-is — the parser matches the platform prefix first and treats whatever remains as the recipe name.

Examples:

```bash
# Build everything, everywhere
git tag plugins-build-1
git push origin plugins-build-1

# Rebuild only the Windows x64 binaries (after fixing a MinGW issue)
git tag plugins-build-windows-x86_64-2
git push origin plugins-build-windows-x86_64-2

# Rebuild one recipe across every platform (after bumping its submodule)
git tag plugins-build-gxplugins-1
git push origin plugins-build-gxplugins-1

# Surgical: one recipe, one platform
git tag plugins-build-macos-universal-artyfx-1
git push origin plugins-build-macos-universal-artyfx-1
```

**Tag-triggered runs publish artifacts only.** Tags refer to immutable commits, so the workflow can't commit the rebuilt binaries back. Download them from the run's Artifacts panel on the Actions page.

### Option 2: `workflow_dispatch` from the Actions tab or the CLI

Identical platform / recipe filters, but the workflow commits the resulting binaries straight back into the branch's `plugins/source/lv2/<plugin>/platform/<plat>/` and updates each plugin's `manifest.yaml`.

```bash
# Everything (default values)
gh workflow run "Build Plugin Libraries" \
  --repo jpfaria/OpenRig-plugins --ref main

# Windows x64 only
gh workflow run "Build Plugin Libraries" \
  --repo jpfaria/OpenRig-plugins --ref main \
  -f platform=windows-x86_64

# One recipe, one platform
gh workflow run "Build Plugin Libraries" \
  --repo jpfaria/OpenRig-plugins --ref main \
  -f recipe=artyfx -f platform=windows-x86_64
```

Available inputs:

- `recipe` — `all` (default) or any recipe name from `scripts/build-lib-internal.sh` (`nam`, `artyfx`, `dragonfly-reverb`, `caps-lv2`, `tap-lv2`, …).
- `platform` — `all` (default), `linux-x86_64`, `linux-aarch64`, `macos-universal`, `windows-x86_64`, `windows-aarch64`.

## Building locally

```bash
# Native (uses the host platform). Output lands in libs/<lv2|nam>/<plat>/
./scripts/build-lib.sh nam

# Cross-platform via Docker (Linux/Windows targets)
./scripts/build-lib.sh nam --platform windows-x86_64

# Everything for every platform
./scripts/build-lib.sh all --platform all

# List recipes the script knows about
./scripts/build-lib.sh --list
```

Output paths from `build-lib.sh` use the legacy `libs/{lv2,nam}/<plat>/` layout — that's for local inspection. The CI workflow uses the manifest-driven dispatcher to land binaries directly under each plugin folder.

## Repository layout

```
plugins/source/lv2/<plugin>/
├── manifest.yaml          # id, display_name, brand, plugin_uri, binaries map
├── assets/                # thumbnail, screenshot, anything visual
├── data/                  # presets, IR samples, ML model assets
└── platform/<plat>/<bin>  # .so / .dylib / .dll per supported platform

plugins/source/nam/<plugin>/
├── manifest.yaml          # id, display_name, brand, type (amp/preamp/gain_pedal),
│                          # per-capture grid + output_gain_db (boost-only, #4)
│                          # + per-capture noise_gate for high-gain idle hiss (#73)
├── assets/                # thumbnail
└── captures/*.nam         # neural amp model captures (loaded via libNeuralAudioCAPI)

plugins/source/ir/<plugin>/
├── manifest.yaml          # id, display_name, brand, type (cab/body), per-capture
│                          # output_gain_db (spectral-unity, #23)
├── assets/                # thumbnail
└── ir/*.wav               # mono 48 kHz IR files (DC-removed, ceiling-capped, #21)

plugins/source/vst3/<plugin>/
├── manifest.yaml          # id, display_name, brand, type, backend: vst3,
│                          # bundle: path + parameters: [] (empty — OpenRig uses
│                          # the plugin's native editor + runtime IEditController)
└── bundles/<Name>.vst3/   # ONE cross-platform bundle, Contents/<arch>/<bin>:
    └── Contents/          #   MacOS/ (universal) · x86_64-linux/ · aarch64-linux/
                           #   · x86_64-win/ — CI unions the per-arch subfolders

docs/
├── blocks-reference.md    # canonical catalogue (Quick Reference auto-generated)

deps/<upstream>/           # git submodule pinned to a known-good commit (LV2 + VST3)
scripts/
├── build-lib.sh           # Docker wrapper (local LV2 builds)
├── build-lib-internal.sh  # the LV2 + VST3 build recipes (consumed by the wrapper + CI)
├── add-dep.sh             # `add-dep <name> <url> <commit>` helper
├── bundle-into-openrig.sh # zips everything into ../OpenRig/plugins/openrig-plugins.zip
├── plugin-recipes.tsv     # documents plugin folder ↔ recipe (catalogue ↔ deps)
├── gen_quick_reference.py # regenerates docs/blocks-reference.md Quick Reference
└── native_models.yaml     # engine-side natives listed in the Quick Reference

tools/                     # in-repo Rust binaries
├── loudness_audit/        # writes per-plugin output_gain_db (NAM: boost-only #4;
│                          # IR: spectral-unity #23), the qa_audit gate (#12), and
│                          # nam_gate_audit — measures idle hiss, writes per-capture
│                          # noise_gate defaults (#73)
└── pack_plugins/          # invokes qa_audit then packs each plugin into a zip
```

## Dependencies

Each entry in `deps/DEPS.md` is a git submodule pinned to an exact upstream commit. Add new ones via `./scripts/add-dep.sh <name> <repo-url> <commit-hash>`. Six catalogue plugins have no upstream registered yet (`avocado`, `ewham_harmonizer`, `fat1_autotune`, `floaty`, `mud`, `paranoia`) — they are intentionally left out of every build until their source is discovered.

## Recipes ↔ catalogue

One recipe builds one upstream repo, which can ship several LV2 plugins:

| Recipe | Catalogue entries it covers |
|---|---|
| `dragonfly-reverb` | `dragonfly_early`, `dragonfly_hall`, `dragonfly_plate`, `dragonfly_room` |
| `caps-lv2` | the seven `caps_*` entries |
| `gxplugins` | the GxPlugins family (`gx_axisface`, `gx_boobtube`, …) |
| `tap-lv2` | the `tap_*` family |
| `mda-lv2` | the `mda_*` family |
| `artyfx` | `artyfx_filta`, `bitta`, `driva`, `roomy`, `satma` (one `.so`, five URIs) |
| `shiro-plugins` | `harmless`, `larynx`, `modulay`, `shiroverb` |
| `zam-plugins` | `zamcomp`, `zameq2`, `zamgate`, `zamgeq31`, `zamulticomp` |
| `fomp` | `fomp_autowah`, `fomp_cs_chorus`, `fomp_cs_phaser` |
| `mod-utilities` | `mod_hpf`, `mod_lpf` |
| `setbfree` | `b_reverb` |
| `wolf-shaper` | `wolf_shaper` |
| `invada-studio` | `invada_tube` |
| `mverb` | `mverb` |
| `ojd` | `ojd` |
| `chowcentaur` | `chow_centaur` (VST3 bundle — the only vst3 recipe so far) |
| `nam` | (consumed natively by OpenRig — no catalogue entry) |

A full enumeration lives in `scripts/plugin-recipes.tsv`.
