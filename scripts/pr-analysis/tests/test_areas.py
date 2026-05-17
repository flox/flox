from scripts.pr_analysis.lib.areas import area_for_path, is_rust, HOT_AREAS


def commands_services_routes_to_services_bucket():
    assert area_for_path("cli/flox/src/commands/services/start.rs") == "commands/services"


def commands_init_routes_to_init_bucket():
    assert area_for_path("cli/flox/src/commands/init/handler.rs") == "commands/init"


def generic_commands_file_routes_to_commands_bucket():
    assert area_for_path("cli/flox/src/commands/install.rs") == "commands"


def environment_model_routes_to_models_environment():
    assert area_for_path(
        "cli/flox-rust-sdk/src/models/environment/managed_environment.rs"
    ) == "models/environment"


def non_environment_model_routes_to_models_other():
    assert area_for_path("cli/flox-rust-sdk/src/models/lockfile.rs") == "models/other"


def providers_routes_to_providers():
    assert area_for_path("cli/flox-rust-sdk/src/providers/catalog.rs") == "providers"


def unknown_path_routes_to_other():
    assert area_for_path("README.md") == "other"


def hot_areas_are_the_three_we_locked_in():
    assert HOT_AREAS == ("commands", "models/environment", "providers")


def rust_detector_is_extension_based():
    assert is_rust("foo/bar.rs")
    assert not is_rust("foo/bar.bats")
