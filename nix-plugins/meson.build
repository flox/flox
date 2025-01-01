project(
    'flox-nix-plugins-root',
    'cpp',
    default_options: [
        'cpp_std=c++2a',
        'warning_level=2',
        'debug=true',
        'optimization=2',
        'errorlogs=true',  # Please print logs for tests that fail
    ],
    meson_version: '>=1.1',
)

gen_header = generator(
    find_program('bash', native: true),
    arguments: [
        '-c',
        '{ echo \'R"__NIX_STR(\' && cat @INPUT@ && echo \')__NIX_STR"\'; } > "$1"',
        '_ignored_argv0',
        '@OUTPUT@',
    ],
    output: '@PLAINNAME@.hh',
)

nix_main_dep = dependency('nix-main', required: true)
nix_store_dep = dependency('nix-store', required: true)
nix_expr_dep = dependency('nix-expr', required: true)
nix_cmd_dep = dependency('nix-cmd', required: true)
nix_fetchers_dep = dependency('nix-fetchers', required: true)

dependencies = [
    nix_main_dep,
    nix_store_dep,
    nix_expr_dep,
    nix_cmd_dep,
    nix_fetchers_dep,
]

# nix global constants such as `SYSTEM`
nix_config_h = nix_main_dep.get_variable('includedir') / 'nix/config.h'

# internal headers
include_dirs = [include_directories('include')]

# lock-flake-installable ================================================
lock_flake_installable_src = files('src/lock-flake-installable.cc')

lock_flake_installable = library(
    'lock-flake-installable',
    lock_flake_installable_src,
    dependencies: dependencies,
    include_directories: include_dirs,
    prelink: true,  # For C++ static initializers
    install: true,
    install_dir: 'lib/nix-plugins',
    cpp_args: ['--include', nix_config_h],
)

# wrapped-nixpkgs-input-schema ==========================================
wrapped_nixpkgs_input_src = files('src/fetchers/wrapped-nixpkgs-input.cc')
wrapped_nixpkgs_input_src += gen_header.process('src/fetchers/flake-v0.nix.in')

wrapped_nixpkgs_input = library(
    'wrapped-nixpkgs-input',
    wrapped_nixpkgs_input_src,
    dependencies: dependencies,
    include_directories: include_dirs,
    prelink: true,  # For C++ static initializers
    install: true,
    install_dir: 'lib/nix-plugins',
    cpp_args: ['--include', nix_config_h],
)

# tests ================================================================
# defined in tests/meson.build
#
# todo: defining the tests as a subproject was tricky because
# of the current directory structure.
# We can revisit this later but for now,
# let's just define the tests in the same project.
# 'tests/meson.build' will have access to the variables defined here,
# particularly `wrapped_nixpkgs_input` and `lock_flake_installable`,
# which are dependencies for the respecive tests.
subdir('tests')
