
lock_flake_installable_test = executable(
    'lock-flake-installable',
    'lock-flake-installable.cc',
    dependencies: dependencies,
    include_directories: include_dirs,
    link_with: lock_flake_installable,
    cpp_args: ['--include', nix_config_h],
)

test(
    'lock-flake-installable',
    lock_flake_installable_test,
    protocol: 'exitcode',
    verbose: true,
    timeout: 300,
    workdir: meson.project_source_root(),
)

flox_nixpkgs_test = executable(
    'flox-nixpkgs',
    'flox-nixpkgs.cc',
    dependencies: dependencies,
    include_directories: include_dirs,
    link_with: wrapped_nixpkgs_input,
    cpp_args: ['--include', nix_config_h],
)

test(
    'flox-nixpkgs',
    flox_nixpkgs_test,
    protocol: 'exitcode',
    verbose: true,
    timeout: 300,
    workdir: meson.project_source_root(),
)
