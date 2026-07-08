# shellcheck shell=bash
# VST3 build recipes — one cross-platform `.vst3` bundle per plugin.
#
# Sourced by scripts/build-lib-internal.sh (which defines the helpers used here:
# do_cmake, collect_bundle, and the DEPS_DIR / BUILD_WORK_DIR / CMAKE_EXTRA env).
# Each recipe builds ONLY the VST3 target (AU / Standalone / LV2 / CLAP skipped)
# and collects the resulting `<Name>.vst3` directory tree; the CI merge job then
# unions each platform's Contents/<arch>/ subfolder into the shipped bundle.

build_chowcentaur() {
    # ChowCentaur (jatinchowdhury18/KlonCentaur): a JUCE/CMake Klon Centaur
    # overdrive (WDF + RNN). This is the repo's first VST3 recipe, so it emits a
    # cross-platform VST3 *bundle* (a `Contents/<arch>/` directory tree), not a
    # single shared lib — hence collect_bundle instead of collect_libs. Only the
    # VST3 target is built (AU / Standalone / LV2 are skipped). The macOS slot is
    # a universal binary via CMAKE_OSX_ARCHITECTURES (set by the workflow).
    local src="$DEPS_DIR/KlonCentaur"
    do_cmake "$src" ChowCentaur_VST3
    collect_bundle "$LAST_BUILD_DIR" "ChowCentaur.vst3"
}

build_chowtape() {
    # ChowTapeModel (jatinchowdhury18/AnalogTapeModel): JUCE/CMake tape-saturation
    # emulation. The CMake project lives under Plugin/. CLAP is disabled (it needs
    # CMake 3.21 + the clap-juce-extensions submodule); only the VST3 target is
    # built. macOS is universal via CMAKE_OSX_ARCHITECTURES (set by the workflow).
    local src="$DEPS_DIR/AnalogTapeModel/Plugin"
    CMAKE_EXTRA="${CMAKE_EXTRA:-} -DCHOWTAPE_BUILD_CLAP=OFF" \
        do_cmake "$src" CHOWTapeModel_VST3
    collect_bundle "$LAST_BUILD_DIR" "CHOWTapeModel.vst3"
}

build_chowphaser() {
    # ChowPhaser (jatinchowdhury18/ChowPhaser): JUCE/CMake phaser. Upstream emits a
    # Mono and a Stereo plugin; OpenRig chains are always stereo, so ship only the
    # Stereo variant. The bundle name carries a space — the merge job handles it.
    local src="$DEPS_DIR/ChowPhaser"
    do_cmake "$src" ChowPhaserStereo_VST3
    collect_bundle "$LAST_BUILD_DIR" "ChowPhaser Stereo.vst3"
}

build_chowmatrix() {
    # ChowMatrix (Chowdhury-DSP/ChowMatrix): JUCE/CMake growable multitap delay.
    local src="$DEPS_DIR/ChowMatrix"
    do_cmake "$src" ChowMatrix_VST3
    collect_bundle "$LAST_BUILD_DIR" "ChowMatrix.vst3"
}

build_chowmultitool() {
    # ChowMultiTool (Chowdhury-DSP/ChowMultiTool): JUCE/CMake multi-effect (EQ,
    # waveshaper, signal gen…). CLAP disabled (CMake 3.21 + clap submodule); only
    # the VST3 target is built.
    local src="$DEPS_DIR/ChowMultiTool"
    CMAKE_EXTRA="${CMAKE_EXTRA:-} -DCHOWMULTITOOL_BUILD_CLAP=OFF" \
        do_cmake "$src" ChowMultiTool_VST3
    collect_bundle "$LAST_BUILD_DIR" "ChowMultiTool.vst3"
}

build_byod() {
    # BYOD (Chowdhury-DSP/BYOD): JUCE/CMake modular "build-your-own-drive". CLAP and
    # the preset server are disabled; only the VST3 target is built.
    local src="$DEPS_DIR/BYOD"
    CMAKE_EXTRA="${CMAKE_EXTRA:-} -DBYOD_BUILD_CLAP=OFF -DBYOD_BUILD_PRESET_SERVER=OFF" \
        do_cmake "$src" BYOD_VST3
    collect_bundle "$LAST_BUILD_DIR" "BYOD.vst3"
}
