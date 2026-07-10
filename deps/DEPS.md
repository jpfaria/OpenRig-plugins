# Dependencies (Git Submodules)

Each dependency is a git submodule pinned to a specific commit.
Use `./scripts/add-dep.sh` to add new ones.

## Current

| Name | Repository | Pinned Hash | Build System | Plugins |
|------|-----------|-------------|--------------|---------|
| neural-amp-modeler-lv2 | https://github.com/mikeoliphant/neural-amp-modeler-lv2 | `5a5865a` | CMake | NeuralAudioCAPI (NAM inference) |
| dragonfly-reverb | https://github.com/michaelwillis/dragonfly-reverb | `b3c15af` | DPF/Make | Hall, Plate, Room, EarlyReflections reverbs |
| zam-plugins | https://github.com/zamaudio/zam-plugins | `6a7fd03` | DPF/Make | ZamComp, ZamDelay, ZamEQ2, ZamTube, ZamGate |
| mod-utilities | https://github.com/mod-audio/mod-utilities | `b8a9d45` | Make | MOD gain, mixers, CV, switchboxes |
| caps-lv2 | https://github.com/mod-audio/caps-lv2 | `5d52a0c` | Make | AmpVTS, CabinetIV, Plate, Chorus, Phaser, Compress |
| tap-lv2 | https://github.com/moddevices/tap-lv2 | `cab6e0d` | Make | Echo, Reverb, Tremolo, EQ, Chorus, TubeWarmth |
| SHIRO-Plugins | https://github.com/ninodewit/SHIRO-Plugins | `3e0a1d3` | DPF/Make | Shiroverb, Modulay, Harmless, Larynx |
| DPF-Plugins | https://github.com/DISTRHO/DPF-Plugins | `df5cb65` | DPF/Make | Kars, Nekobi, PingPongPan |
| MVerb | https://github.com/DISTRHO/MVerb | `5ae9f57` | DPF/Make | MVerb reverb |
| mda-lv2 | https://gitlab.com/drobilla/mda-lv2 | `8218120` | Meson | DubDelay, Leslie, Overdrive, EPiano, Piano |
| fomp | https://gitlab.com/drobilla/fomp | `9ed4d2e` | Meson | VCO, VCF, Phaser, Flanger |
| invada-studio | https://github.com/BlokasLabs/invada-studio | `9525be9` | Make | Compressor, Delay, Reverb, Filter, Tube |
| wolf-shaper | https://github.com/wolf-plugins/wolf-shaper | `d38cc33` | DPF/Make | Waveshaper |
| openAV-ArtyFX | https://github.com/openAVproductions/openAV-ArtyFX | `284eab7` | CMake | Bitta, Filta, Kuiza, Satma |
| sooperlooper | https://github.com/essej/sooperlooper | `c5e22ce` | Autotools | Looper |
| setBfree | https://github.com/pantherb/setBfree | `25274ac` | Make | Hammond organ, whirl speaker |
| GxPlugins.lv2 | https://github.com/brummer10/GxPlugins.lv2 | `3a32527` | Make | Guitarix amp sims, effects |
| AnalogTapeModel | https://github.com/jatinchowdhury18/AnalogTapeModel | `604372e` | CMake/JUCE | ChowTapeModel (tape saturation) — VST3 bundle, recipe `chowtape` |
| KlonCentaur | https://github.com/jatinchowdhury18/KlonCentaur | `f3bb633` | CMake/JUCE | ChowCentaur (Klon overdrive) — VST3 bundle, recipe `chowcentaur` |
| ChowPhaser | https://github.com/jatinchowdhury18/ChowPhaser | `31eac4d` | CMake/JUCE | ChowPhaser (WDF phaser, stereo) — VST3 bundle, recipe `chowphaser` |
| ChowMatrix | https://github.com/Chowdhury-DSP/ChowMatrix | `40d8e0e` | CMake/JUCE | ChowMatrix (multitap delay) — VST3 bundle, recipe `chowmatrix` |
| ChowMultiTool | https://github.com/Chowdhury-DSP/ChowMultiTool | `0b65e3a` | CMake/JUCE | ChowMultiTool (multi-effect) — VST3 bundle, recipe `chowmultitool` |
| BYOD | https://github.com/Chowdhury-DSP/BYOD | `1cf22b6` | CMake/JUCE | BYOD (modular distortion) — VST3 bundle, recipe `byod` |
| reevr | https://github.com/tiagolr/reevr | `e4d553a` | CMake/JUCE | REEV-R (convolution reverb) — VST3 bundle, recipe `reevr` |
| sirial | https://github.com/tiagolr/sirial | `ec31132` | CMake/JUCE | Sirial (rhythmic delay) — VST3 bundle, recipe `sirial` |
| qdelay | https://github.com/tiagolr/qdelay | `13ef451` | CMake/JUCE | QDelay (dual delay) — VST3 bundle, recipe `qdelay` |
| gate12 | https://github.com/tiagolr/gate12 | `df65245` | CMake/JUCE | GATE-12 (trance gate) — VST3 bundle, recipe `gate12` |
| time12 | https://github.com/tiagolr/time12 | `cb86fd6` | CMake/JUCE | TIME-12 (stutter/tape-stop) — VST3 bundle, recipe `time12` |
| filtr | https://github.com/tiagolr/filtr | `b42c4e0` | CMake/JUCE | FILT-R (envelope filter, AGPL-3) — VST3 bundle, recipe `filtr` |
| ZLEqualizer | https://github.com/ZL-Audio/ZLEqualizer | `903c0c9` | CMake/JUCE | ZLEqualizer (dynamic EQ, AGPL-3) — VST3 bundle, recipe `zl_equalizer` |
| ZLCompressor | https://github.com/ZL-Audio/ZLCompressor | `b2fe331` | CMake/JUCE | ZLCompressor (compressor, AGPL-3) — VST3 bundle, recipe `zl_compressor` |
| ZLSplitter | https://github.com/ZL-Audio/ZLSplitter | `dfaccc6` | CMake/JUCE | ZLSplitter (signal splitter, AGPL-3) — VST3 bundle, recipe `zl_splitter` |
| ZLSpectrumEqualizer | https://github.com/ZL-Audio/ZLSpectrumEqualizer | `21cc97d` | CMake/JUCE | ZLSpectrumEqualizer (spectrum EQ, AGPL-3) — VST3 bundle, recipe `zl_spectrum_equalizer` |
| ZLWarm | https://github.com/ZL-Audio/ZLWarm | `48093f3` | CMake/JUCE | ZLWarm (saturation, GPL-3) — VST3 bundle, recipe `zl_warm` |
| ZLInflator | https://github.com/ZL-Audio/ZLInflator | `b71bf48` | CMake/JUCE | ZLInflator (loudness, GPL-3) — VST3 bundle, recipe `zl_inflator` |
| CloudReverb | https://github.com/xunil-cloud/CloudReverb | `92804ed` | CMake/JUCE | CloudReverb (shimmer reverb) — VST3 bundle, recipe `cloudreverb` |
| RoomReverb | https://github.com/cvde/RoomReverb | `11f2de0` | CMake/JUCE | RoomReverb (algorithmic reverb) — VST3 bundle, recipe `roomreverb` |
| Frequalizer | https://github.com/ffAudio/Frequalizer | `c4b1b61` | CMake/JUCE | Frequalizer (parametric EQ, BSD-3) — VST3 bundle, recipe `frequalizer` |
| retuner | https://github.com/kushview/retuner | `4a8fb06` | CMake/JUCE | reTuner (pitch shift) — VST3 bundle, recipe `retuner` |
| setekh | https://github.com/fullfxmedia/setekh | `468a9bd` | CMake/JUCE | Setekh (saturation) — VST3 bundle, recipe `setekh` |
| vitOTTx | https://github.com/Sakhnovkrg/vitOTTx | `738ba9d` | CMake/JUCE | vitOTTx (multiband OTT) — VST3 bundle, recipe `vitottx` |
| AIDA-X | https://github.com/AidaDSP/AIDA-X | `41eb988` | CMake/DPF | AIDA-X (neural amp+cab) — VST3 bundle, recipe `aidax` |
| Schrammel_OJD | https://github.com/JanosGit/Schrammel_OJD | `03c0e84` | CMake/JUCE | OJD overdrive |

Every row above is a real git submodule (a committed gitlink under `deps/`). Each `build_*` recipe — grouped by backend in `scripts/recipes/lv2.sh` and `scripts/recipes/vst3.sh`, sourced by `scripts/build-lib-internal.sh` — expects its upstream checked out under `deps/<name>/` (CI checks them out with `submodules: recursive`). Register a new one with `./scripts/add-dep.sh <name> <url> <commit>` when activating the recipe.

## Updating a dependency

Submodules are pinned to the exact commit above. To update:

```bash
cd deps/<name>
git fetch origin
git checkout <new-commit-hash>
cd ../..
git add deps/<name>
git commit -m "Update <name> to <hash>"
```

Then update the hash in this table.

## Building

```bash
# Build one plugin (macOS native):
./scripts/build-lib.sh nam

# Build for Linux via Docker:
./scripts/build-lib.sh nam --platform linux-x86_64

# Build multiple:
./scripts/build-lib.sh dragonfly-reverb zam-plugins

# Build all:
./scripts/build-lib.sh all

# List available plugins:
./scripts/build-lib.sh --list
```

Output goes to:
- `libs/nam/{platform}/` — NAM libraries
- `libs/lv2/{platform}/` — LV2 plugin libraries
