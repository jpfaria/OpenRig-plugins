# Copyright 2021-2022 David Robillard <d@drobilla.net>
# SPDX-License-Identifier: 0BSD OR GPL-2.0-or-later

module_names = [
  'autowah',
  'blvco',
  'cs_chorus',
  'cs_phaser',
  'filters',
  'mvchpf24',
  'mvclpf24',
]

plugin_names = [
  'autowah',
  'cs_chorus1',
  'cs_chorus2',
  'cs_phaser1_lfo',
  'cs_phaser1',
  'mvchpf1',
  'mvclpf1',
  'mvclpf2',
  'mvclpf3',
  'mvclpf4',
  'parametric1',
  'pulse_vco',
  'rec_vco',
  'reverb_amb',
  'reverb',
  'saw_vco',
  'triple_chorus',
]

# Copy data files to bundle
foreach plugin_name : plugin_names
  configure_file(
    copy: true,
    input: files(plugin_name + '.ttl'),
    install: true,
    install_dir: lv2dir / 'fomp.lv2',
    output: '@PLAINNAME@',
  )
endforeach

# Build reverbs module (different because it has many sources)
reverbs_module = shared_module(
  'reverbs',
  files(
    '../src/pareq.cc',
    '../src/reverbs.cc',
    '../src/reverbs_lv2.cc',
    '../src/zreverb.cc',
  ),
  cpp_args: cpp_suppressions,
  dependencies: [lv2_dep, m_dep],
  gnu_symbol_visibility: 'hidden',
  install: true,
  install_dir: lv2dir / 'fomp.lv2',
  name_prefix: '',
)

module_ext = reverbs_module.full_path().split('.')[-1]

# Build all other modules
modules = [reverbs_module]
foreach module_name : module_names
  module = shared_module(
    module_name,
    files(
      '../src/@0@.cc'.format(module_name),
      '../src/@0@_lv2.cc'.format(module_name),
    ),
    cpp_args: cpp_suppressions,
    dependencies: [lv2_dep, m_dep],
    gnu_symbol_visibility: 'hidden',
    install: true,
    install_dir: lv2dir / 'fomp.lv2',
    name_prefix: '',
  )

  modules += [module]
endforeach

# Configure bundle manifest
configure_file(
  configuration: configuration_data({'LIB_EXT': '.' + module_ext}),
  input: files('manifest.ttl.in'),
  install_dir: lv2dir / 'fomp.lv2',
  output: 'manifest.ttl',
)

# Run lv2lint test on all plugins
if get_option('lint') and not meson.is_subproject()
  lv2lint = find_program('lv2lint', required: get_option('tests'))
  if lv2lint.found()
    plugin_uris = []
    foreach plugin_name : plugin_names
      plugin_uris += ['http://drobilla.net/plugins/fomp/' + plugin_name]
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
