# Copyright 2021-2022 David Robillard <d@drobilla.net>
# SPDX-License-Identifier: 0BSD OR MIT

plugins = [
  'Ambience',
  'Bandisto',
  'BeatBox',
  'Combo',
  'DX10',
  'DeEss',
  'Degrade',
  'Delay',
  'Detune',
  'Dither',
  'DubDelay',
  'Dynamics',
  'EPiano',
  'Image',
  'JX10',
  'Leslie',
  'Limiter',
  'Loudness',
  'MultiBand',
  'Overdrive',
  'Piano',
  'RePsycho',
  'RezFilter',
  'RingMod',
  'RoundPan',
  'Shepard',
  'Splitter',
  'Stereo',
  'SubSynth',
  'TalkBox',
  'TestTone',
  'ThruZero',
  'Tracker',
  'Transient',
  'VocInput',
  'Vocoder',
]

preset_files = [
  'DX10-presets.ttl',
  'Detune-presets.ttl',
  'EPiano-presets.ttl',
  'JX10-presets.ttl',
  'Leslie-presets.ttl',
  'Piano-presets.ttl',
  'Splitter-presets.ttl',
  'ThruZero-presets.ttl',
  'Vocoder-presets.ttl',
]

modules = []
foreach plugin : plugins
  plugin_sources = files(
    '../lvz/wrapper.cpp',
    '../src/mda@0@.cpp'.format(plugin),
  )

  # Build loadable plugin module
  module = shared_library(
    plugin,
    plugin_sources,
    cpp_args: cpp_suppressions + [
      '-DPLUGIN_CLASS=mda@0@'.format(plugin),
      '-DURI_PREFIX="http://drobilla.net/plugins/mda/"',
      '-DPLUGIN_URI_SUFFIX="@0@"'.format(plugin),
      '-DPLUGIN_HEADER="../src/mda@0@.h"'.format(plugin),
    ],
    dependencies: [lv2_dep, m_dep],
    gnu_symbol_visibility: 'hidden',
    include_directories: include_directories('../lvz'),
    install: true,
    install_dir: lv2dir / 'mda.lv2',
    name_prefix: '',
  )

  # Configure plugin data file
  module_ext = module.full_path().split('.')[-1]
  data_filename = '@0@.ttl'.format(plugin)
  configure_file(
    copy: true,
    input: files(data_filename),
    install: true,
    install_dir: lv2dir / 'mda.lv2',
    output: data_filename,
  )

  modules += [module]
endforeach

# Configure bundle manifest
configure_file(
  configuration: configuration_data({'LIB_EXT': '.' + module_ext}),
  input: files('manifest.ttl.in'),
  install_dir: lv2dir / 'mda.lv2',
  output: 'manifest.ttl',
)

# Install additional data files
install_data(preset_files, install_dir: lv2dir / 'mda.lv2')

# Run lv2lint test on all plugins
if get_option('lint') and not meson.is_subproject()
  lv2lint = find_program('lv2lint', required: get_option('tests'))
  if lv2lint.found()
    plugin_uris = []
    foreach plugin : plugins
      plugin_uris += ['http://drobilla.net/plugins/mda/' + plugin]
    endforeach

    test(
      'lv2lint',
      lv2lint,
      args: ['-I', meson.current_build_dir(), '-E', 'warn'] + plugin_uris,
      depends: modules,
      suite: 'plugins',
    )
  endif
endif
