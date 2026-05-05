# Copyright 2021-2023 David Robillard <d@drobilla.net>
# SPDX-License-Identifier: 0BSD OR MIT

project(
  'mda-lv2',
  ['cpp'],
  default_options: [
    'b_ndebug=if-release',
    'buildtype=release',
    'cpp_std=c++11',
  ],
  license: 'GPLv2+',
  meson_version: '>= 0.56.0',
  version: '1.2.10',
)

#######################
# Compilers and Flags #
#######################

# Load build tools
cpp = meson.get_compiler('cpp')

# Set global warning suppressions
cpp_suppressions = []
warning_level = get_option('warning_level')
if cpp.get_id() in ['clang', 'emscripten']
  if warning_level == 'everything'
    cpp_suppressions += [
      '-Wno-c++98-compat',
      '-Wno-cast-align',
      '-Wno-cast-qual',
      '-Wno-comma',
      '-Wno-deprecated-copy-dtor',
      '-Wno-deprecated-declarations',
      '-Wno-documentation-unknown-command',
      '-Wno-double-promotion',
      '-Wno-float-equal',
      '-Wno-implicit-float-conversion',
      '-Wno-old-style-cast',
      '-Wno-padded',
      '-Wno-reserved-id-macro',
      '-Wno-shorten-64-to-32',
      '-Wno-sign-conversion',
      '-Wno-suggest-destructor-override',
      '-Wno-suggest-override',
      '-Wno-switch-default',
      '-Wno-unsafe-buffer-usage',
      '-Wno-weak-vtables',
      '-Wno-zero-as-null-pointer-constant',
    ]

    if not meson.is_cross_build()
      cpp_suppressions += [
        '-Wno-poison-system-directories',
      ]
    endif

    if host_machine.system() == 'windows'
      cpp_suppressions += [
        '-Wno-deprecated-declarations',
      ]
    endif
  endif

  if warning_level in ['everything', '3']
    cpp_suppressions += [
      '-Wno-nullability-extension',
    ]
  endif

  if warning_level in ['everything', '3', '2']
    cpp_suppressions += [
      '-Wno-unused-parameter',
    ]
  endif

elif cpp.get_id() == 'gcc'
  if warning_level == 'everything'
    cpp_suppressions += [
      '-Wno-cast-align',
      '-Wno-cast-qual',
      '-Wno-conversion',
      '-Wno-deprecated-copy-dtor',
      '-Wno-double-promotion',
      '-Wno-duplicated-branches',
      '-Wno-effc++',
      '-Wno-float-conversion',
      '-Wno-float-equal',
      '-Wno-format-overflow',
      '-Wno-old-style-cast',
      '-Wno-padded',
      '-Wno-strict-overflow',
      '-Wno-suggest-attribute=const',
      '-Wno-suggest-attribute=pure',
      '-Wno-suggest-final-methods',
      '-Wno-suggest-final-types',
      '-Wno-suggest-override',
      '-Wno-switch-default',
      '-Wno-unused-const-variable',
      '-Wno-useless-cast',
    ]
  endif

  if warning_level in ['everything', '3', '2']
    cpp_suppressions += [
      '-Wno-unused-parameter',
    ]
  endif

elif cpp.get_id() == 'msvc'
  if warning_level == 'everything'
    cpp_suppressions += [
      '/wd4365', # signed/unsigned mismatch
      '/wd4464', # relative include path contains '..'
      '/wd4514', # unreferenced inline function has been removed
      '/wd4706', # assignment within conditional expression
      '/wd4710', # function not inlined
      '/wd4711', # function selected for automatic inline expansion
      '/wd4820', # padding added after construct
      '/wd5045', # will insert Spectre mitigation for memory load
      '/wd5219', # implicit conversion to float with possible loss of data
      '/wd5264', # const variable is not used
    ]
  endif

  if warning_level in ['everything', '3']
    cpp_suppressions += [
      '/wd4100', # unreferenced formal parameter
    ]
  endif

  if warning_level in ['everything', '3', '2']
    cpp_suppressions += [
      '/wd4244', # conversion from unsigned long to float
      '/wd4996', # POSIX name for this item is deprecated
    ]
  endif

endif

cpp_suppressions = cpp.get_supported_arguments(cpp_suppressions)

################
# Dependencies #
################

m_dep = cpp.find_library('m', required: false)

lv2_dep = dependency('lv2', version: '>= 1.16.0')

#########
# Paths #
#########

lv2dir = get_option('lv2dir')
if lv2dir == ''
  prefix = get_option('prefix')
  if target_machine.system() == 'darwin' and prefix == '/'
    lv2dir = '/Library/Audio/Plug-Ins/LV2'
  elif target_machine.system() == 'haiku' and prefix == '/'
    lv2dir = '/boot/common/add-ons/lv2'
  elif target_machine.system() == 'windows' and prefix == 'C:/'
    lv2dir = 'C:/Program Files/Common/LV2'
  else
    lv2dir = prefix / get_option('libdir') / 'lv2'
  endif
endif

#################
# Plugin Bundle #
#################

subdir('mda.lv2')

#########
# Tests #
#########

if get_option('lint')
  if not meson.is_subproject()
    # Check release metadata
    autoship = find_program('autoship', required: get_option('tests'))
    if autoship.found()
      test(
        'autoship',
        autoship,
        args: ['test', meson.current_source_dir()],
        suite: 'data',
      )
    endif
  endif

  # Check licensing metadata
  reuse = find_program('reuse', required: get_option('tests'))
  if reuse.found()
    test(
      'REUSE',
      reuse,
      args: ['--root', meson.current_source_dir(), 'lint'],
      suite: 'data',
    )
  endif
endif
