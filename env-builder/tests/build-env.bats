#! /usr/bin/env bats
#
# --------------------------------------------------------------------------- #
#
# `flox-env-builder build-env` tests.
#
# Test building environments from lockfiles.
#
# Relies on lockfiles generated by `pkgdb` from flox manifests.
#
# These tests only check the build segment,
# they do not check the resolution of manifests,
# nor the activation of the resulting environments.
# Such tests are found in `pkgdb` and `flox` respectively.
#
# --------------------------------------------------------------------------- #

# bats file_tags=build-env

load setup_suite.bash

# bats test_tags=single,smoke
@test "Simple environment builds successfully" {
    cat $LOCKFILES/single-package/manifest.toml >&3

    run "$ENV_BUILDER_BIN" build-env \
        --lockfile "$(cat $LOCKFILES/single-package/manifest.lock)"

    assert_success
}

# bats test_tags=single,binaries
@test "Built environment contains binaries" {
    cat $LOCKFILES/single-package/manifest.toml >&3

    run "$ENV_BUILDER_BIN" build-env \
        --lockfile "$(cat $LOCKFILES/single-package/manifest.lock)" \
        --out-link "$BATS_TEST_TMPDIR/env"

    assert_success
    assert [ -x "$BATS_TEST_TMPDIR/env/bin/vim" ]
}

# bats test_tags=single,activate-files
@test "Built environment contains activate files" {
    cat $LOCKFILES/single-package/manifest.toml >&3

    run "$ENV_BUILDER_BIN" build-env \
        --lockfile "$(cat $LOCKFILES/single-package/manifest.lock)" \
        --out-link "$BATS_TEST_TMPDIR/env"

    assert_success
    assert [ -f "$BATS_TEST_TMPDIR/env/activate/bash" ]
    assert [ -f "$BATS_TEST_TMPDIR/env/activate/zsh" ]
    assert [ -d "$BATS_TEST_TMPDIR/env/etc/profile.d" ]
}

# --------------------------------------------------------------------------- #

# bats test_tags=hook,script
@test "Built environment includes hook script" {
    cat $LOCKFILES/hook-script/manifest.toml >&3

    run "$ENV_BUILDER_BIN" build-env \
        --lockfile "$(cat $LOCKFILES/hook-script/manifest.lock)" \
        --out-link "$BATS_TEST_TMPDIR/env"

    assert_success
    assert [ -f "$BATS_TEST_TMPDIR/env/activate/hook.sh" ]

    run cat "$BATS_TEST_TMPDIR/env/activate/hook.sh"
    assert_output "script"
}


# bats test_tags=hook,file
@test "Built environment includes hook file" {
    skip "Hook files require path"

    cat $LOCKFILES/hook-file/manifest.toml >&3

    run "$ENV_BUILDER_BIN" build-env \
        --lockfile "$(cat $LOCKFILES/hook-file/manifest.lock)" \
        --out-link "$BATS_TEST_TMPDIR/env"

    assert_success
    assert [ -f "$BATS_TEST_TMPDIR/env/activate/hook.sh" ]

    run cat "$BATS_TEST_TMPDIR/env/activate/hook.sh"
    assert_output "file"
}

# --------------------------------------------------------------------------- #

# bats test_tags=conflict,detect
@test "Detects conflicting packages" {

    cat $LOCKFILES/conflict/manifest.toml >&3

    run "$ENV_BUILDER_BIN" build-env \
        --lockfile "$(cat $LOCKFILES/conflict/manifest.lock)" \
        --out-link "$BATS_TEST_TMPDIR/env"

    assert_failure
    assert_output --partial "file conflict between packages"
}

# bats test_tags=conflict,resolve
@test "Allows to resolve conflicting with priority" {

    cat $LOCKFILES/conflict-resolved/manifest.toml >&3

    run "$ENV_BUILDER_BIN" build-env \
        --lockfile "$(cat $LOCKFILES/conflict-resolved/manifest.lock)" \
        --out-link "$BATS_TEST_TMPDIR/env"

    assert_success
}

# --------------------------------------------------------------------------- #

# bats test_tags=propagated
@test "Environment includes propagated packages" {
    skip "ansi does not work on all systems"

    cat $LOCKFILES/propagated/manifest.toml >&3

    run "$ENV_BUILDER_BIN" build-env \
        --lockfile "$(cat $LOCKFILES/propagated/manifest.lock)" \
        --out-link "$BATS_TEST_TMPDIR/env"
    assert_success

    # environment contains anki
    # -> which propagates beautifulsoup4
    assert [ -f "$BATS_TEST_TMPDIR/env/lib/python3.10/site-packages/bs4/__init__.py" ]
    # -> which propagates chardet
    assert [ -f "$BATS_TEST_TMPDIR/env/lib/python3.10/site-packages/chardet/__init__.py" ]
}
