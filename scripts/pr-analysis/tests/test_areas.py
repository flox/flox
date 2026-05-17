from scripts.pr_analysis.lib.areas import area_for_path, is_rust, HOT_AREAS


def test_commands_services_routes_to_services_bucket():
    assert area_for_path("cli/flox/src/commands/services/start.rs") == "commands/services"


def test_commands_init_routes_to_init_bucket():
    assert area_for_path("cli/flox/src/commands/init/handler.rs") == "commands/init"


def test_generic_commands_file_routes_to_commands_bucket():
    assert area_for_path("cli/flox/src/commands/install.rs") == "commands"


def test_environment_model_routes_to_models_environment():
    assert area_for_path(
        "cli/flox-rust-sdk/src/models/environment/managed_environment.rs"
    ) == "models/environment"


def test_non_environment_model_routes_to_models_other():
    assert area_for_path("cli/flox-rust-sdk/src/models/lockfile.rs") == "models/other"


def test_providers_routes_to_providers():
    assert area_for_path("cli/flox-rust-sdk/src/providers/catalog.rs") == "providers"


def test_unknown_path_routes_to_other():
    assert area_for_path("README.md") == "other"


def test_hot_areas_are_the_three_we_locked_in():
    assert HOT_AREAS == ("commands", "models/environment", "providers")


def test_rust_detector_is_extension_based():
    assert is_rust("foo/bar.rs")
    assert not is_rust("foo/bar.bats")


def test_prefix_map_is_ordered_longest_first():
    """A later prefix in _PREFIX_MAP must not be a strict prefix of an earlier one.

    If it were, the longer (later) entry would never match — area_for_path
    short-circuits on the first hit.
    """
    from scripts.pr_analysis.lib.areas import _PREFIX_MAP
    for i, (earlier, _) in enumerate(_PREFIX_MAP):
        for later, _ in _PREFIX_MAP[i + 1:]:
            assert not later.startswith(earlier), (
                f"{later!r} listed after its prefix {earlier!r} — would never match"
            )


def test_hot_areas_are_all_producible_by_area_for_path():
    """Every HOT_AREAS string must appear as a value in _PREFIX_MAP.

    Otherwise a rename in _PREFIX_MAP silently desynchronizes the constant
    used by per-area analysis from what area_for_path actually emits.
    """
    from scripts.pr_analysis.lib.areas import _PREFIX_MAP, HOT_AREAS
    produced = {area for _, area in _PREFIX_MAP}
    assert set(HOT_AREAS) <= produced, (
        f"HOT_AREAS contains values not produced by _PREFIX_MAP: "
        f"{set(HOT_AREAS) - produced}"
    )


def test_environment_interpreter_assets_route_to_activations():
    assert area_for_path(
        "assets/environment-interpreter/etc/profile.d/foo.sh"
    ) == "activations"
