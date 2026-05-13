# OpenRig-plugins

Catalogue + build pipeline for every LV2 plugin OpenRig ships. Each plugin lives under `plugins/source/lv2/<name>/` with a `manifest.yaml`, the visual assets, and one `.so` / `.dylib` / `.dll` per supported platform under `platform/<plat>/`. The `openrig-plugins.zip` consumed by the OpenRig installer is produced from this tree by `scripts/bundle-into-openrig.sh`.

## Triggering a build

The build pipeline (`.github/workflows/build-libs.yml`) runs only when one of two things happens. There is no push-triggered auto-rebuild — every run is intentional.

### Option 1: Push a `plugins-build-*` tag

The tag name encodes the scope. Peel a platform prefix off the front (if any), then a recipe (if any); the trailing integer is an arbitrary build counter so two runs of the same scope don't collide on the tag namespace.

| Tag | Scope |
|---|---|
| `plugins-build-1` | every recipe, every platform |
| `plugins-build-windows-x64-1` | every recipe, only `windows-x64` |
| `plugins-build-artyfx-1` | only the `artyfx` recipe, every platform |
| `plugins-build-windows-x64-artyfx-1` | only `artyfx`, only `windows-x64` |
| `plugins-build-macos-universal-dragonfly-reverb-2` | `dragonfly-reverb` on macOS |
| `plugins-build-mda-lv2-7` | `mda-lv2` on every platform |

Recipe names that contain dashes (`dragonfly-reverb`, `caps-lv2`, `mod-utilities`, `mda-lv2`) work as-is — the parser matches the platform prefix first and treats whatever remains as the recipe name.

Examples:

```bash
# Build everything, everywhere
git tag plugins-build-1
git push origin plugins-build-1

# Rebuild only the Windows x64 binaries (after fixing a MinGW issue)
git tag plugins-build-windows-x64-2
git push origin plugins-build-windows-x64-2

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
  -f platform=windows-x64

# One recipe, one platform
gh workflow run "Build Plugin Libraries" \
  --repo jpfaria/OpenRig-plugins --ref main \
  -f recipe=artyfx -f platform=windows-x64
```

Available inputs:

- `recipe` — `all` (default) or any recipe name from `scripts/build-lib-internal.sh` (`nam`, `artyfx`, `dragonfly-reverb`, `caps-lv2`, `tap-lv2`, …).
- `platform` — `all` (default), `linux-x86_64`, `linux-aarch64`, `macos-universal`, `windows-x64`, `windows-arm64`.

## Building locally

```bash
# Native (uses the host platform). Output lands in libs/<lv2|nam>/<plat>/
./scripts/build-lib.sh nam

# Cross-platform via Docker (Linux/Windows targets)
./scripts/build-lib.sh nam --platform windows-x64

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

deps/<upstream>/           # git submodule pinned to a known-good commit
scripts/
├── build-lib.sh           # Docker wrapper (local builds)
├── build-lib-internal.sh  # the 20 build recipes (consumed by the wrapper + CI)
├── add-dep.sh             # `add-dep <name> <url> <commit>` helper
├── bundle-into-openrig.sh # zips everything into ../OpenRig/plugins/openrig-plugins.zip
└── plugin-recipes.tsv     # documents plugin folder ↔ recipe (catalogue ↔ deps)
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
| `nam` | (consumed natively by OpenRig — no catalogue entry) |

A full enumeration lives in `scripts/plugin-recipes.tsv`.
