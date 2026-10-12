#! /usr/bin/env bats

load test_support.bash

# bats file_tags=defaults

setup_file() { common_file_setup; }
teardown_file() { common_file_teardown; }

setup() {
  common_test_setup
  home_setup test
  setup_isolated_flox
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/empty.yaml"
  export DEFAULTS_PROBE="$BATS_TEST_TMPDIR/probe"
  mkdir -p "$DEFAULTS_PROBE"
  export DEFAULTS_SH="$(command -v sh)"
  unset _FLOX_ACTIVE_ENVIRONMENTS _FLOX_SOURCED_PROFILE_SCRIPTS
}

teardown() {
  if [ -n "${DEFAULTS_TMUX:-}" ]; then
    for socket in "$DEFAULTS_PROBE"/*.sock; do
      test ! -S "$socket" || "$DEFAULTS_TMUX" -S "$socket" kill-server || true
    done
  fi
  if [ -n "${PROJECT_DIR:-}" ] && [ -d "$PROJECT_DIR/.flox" ]; then
    wait_for_activations "$PROJECT_DIR" || return 1
  fi
  if [ -n "${OTHER_DIR:-}" ] && [ -d "$OTHER_DIR/.flox" ]; then
    wait_for_activations "$OTHER_DIR" || return 1
  fi
  common_test_teardown
}

startup_setup() {
  export PROJECT_DIR="$DEFAULTS_PROBE/fred"
  export OTHER_DIR="$DEFAULTS_PROBE/bob"
  mkdir -p "$PROJECT_DIR" "$OTHER_DIR" "$HOME/.config/fish"
  "$FLOX_BIN" init -d "$PROJECT_DIR"
  "$FLOX_BIN" init -d "$OTHER_DIR"
  cat > "$DEFAULTS_PROBE/record.sh" <<'EOF'
printf '%s\n' "${_FLOX_ACTIVE_ENVIRONMENTS-[]}" > "$DEFAULTS_PROBE/$1.state"
EOF
  cat > "$DEFAULTS_PROBE/pty.exp" <<'EOF'
set timeout 30
spawn -noecho {*}$argv
if {[info exists env(DEFAULTS_INPUT)]} { send -- "$env(DEFAULTS_INPUT)\nexit\n" }
expect {
  timeout { exit 124 }
  eof {}
}
set result [wait]
exit [lindex $result 3]
EOF
  for shell_name in bash zsh fish tcsh; do
    case "$shell_name" in
      bash) rc="$HOME/.bashrc" ;;
      zsh) rc="$HOME/.zshrc" ;;
      fish) rc="$HOME/.config/fish/config.fish" ;;
      tcsh) rc="$HOME/.tcshrc" ;;
    esac
    "$FLOX_BIN" defaults setup --shell "$shell_name" --print > "$rc"
  done
}

assert_saved_path() {
  jq -e --arg path "$2/.flox" \
    'length == 1 and .[0].environment.path == $path' "$DEFAULTS_PROBE/$1.state"
}

run_startup() {
  # tcsh -ic has no prompt variable during RC evaluation. Drive a real
  # interactive terminal so this exercises the installed startup path.
  run env DEFAULTS_INPUT="$2" expect "$DEFAULTS_PROBE/pty.exp" "$(command -v "$1")" -i
}

@test "defaults: remote mutations are offline, idempotent, and do not activate" {
  run --separate-stderr "$FLOX_BIN" defaults set acme/fred --json
  assert_success
  assert_equal "$output" '{"changed":true,"integration":{"status":"unverified"},"selection":{"kind":"remote","reference":"acme/fred"}}'
  cp "$FLOX_CONFIG_DIR/defaults.toml" "$DEFAULTS_PROBE/saved"
  run --separate-stderr "$FLOX_BIN" defaults set acme/fred --json
  assert_success
  assert_equal "$(jq .changed <<< "$output")" false
  cmp "$FLOX_CONFIG_DIR/defaults.toml" "$DEFAULTS_PROBE/saved"
  test ! -e "$FLOX_DATA_DIR"
  test -z "${_FLOX_ACTIVE_ENVIRONMENTS:-}"
  run --separate-stderr "$FLOX_BIN" defaults clear --json
  assert_success
  assert_equal "$output" '{"changed":true,"integration":{"status":"unverified"},"selection":null}'
  run --separate-stderr "$FLOX_BIN" defaults clear --json
  assert_success
  assert_equal "$(jq .changed <<< "$output")" false
}

@test "defaults: canonical local identity and unavailable source inventory" {
  local first="$DEFAULTS_PROBE/a same'\"\$;name" second="$DEFAULTS_PROBE/b"
  mkdir -p "$first/.flox" "$second/.flox"
  # Metadata only: saving must not need a manifest or a built environment.
  printf '%s\n' '{"name":"same","version":1}' > "$first/.flox/env.json"
  cp "$first/.flox/env.json" "$second/.flox/env.json"
  ln -s "$first" "$DEFAULTS_PROBE/link"
  run --separate-stderr "$FLOX_BIN" defaults set --dir "$DEFAULTS_PROBE/link" --json
  assert_success
  assert_equal "$(jq -r .selection.path <<< "$output")" "$first"
  run --separate-stderr "$FLOX_BIN" defaults set --dir "$first" --json
  assert_success
  assert_equal "$(jq .changed <<< "$output")" false
  run --separate-stderr "$FLOX_BIN" defaults set --dir "$second" --json
  assert_success
  assert_equal "$(jq .changed <<< "$output")" true
  mv "$second" "$second-missing"
  run --separate-stderr "$FLOX_BIN" defaults list --json
  assert_success
  assert_equal "$(jq -r .selection.path <<< "$output")" "$second"
  run --separate-stderr "$FLOX_BIN" defaults activate --print-script
  assert_failure
  assert_equal "$output" ''
  run --separate-stderr "$FLOX_BIN" defaults clear --json
  assert_success
  test -d "$second-missing/.flox"
}

@test "defaults: concurrent writers serialize idempotent mutations" {
  local pids=() i
  for i in {1..8}; do
    "$FLOX_BIN" defaults set acme/fred --json > "$DEFAULTS_PROBE/$i.json" &
    pids+=("$!")
  done
  for pid in "${pids[@]}"; do wait "$pid"; done
  assert_equal "$(jq -s '[.[] | select(.changed)] | length' "$DEFAULTS_PROBE/"*.json)" 1
  run --separate-stderr "$FLOX_BIN" defaults list --json
  assert_success
  assert_equal "$(jq -r .selection.reference <<< "$output")" acme/fred
}

@test "defaults: failed atomic write retains the previous selector" {
  "$FLOX_BIN" defaults set acme/fred
  cp "$FLOX_CONFIG_DIR/defaults.toml" "$DEFAULTS_PROBE/saved"
  # Exercise a real failed write even when tests run as root. Ignore SIGXFSZ
  # so the checked write returns an I/O error instead of terminating abruptly.
  run --separate-stderr bash -c 'trap "" XFSZ; ulimit -f 0; "$FLOX_BIN" defaults set acme/bob --json'
  assert_failure
  assert_equal "$(jq -r .error.code <<< "$output")" config_io
  [[ "$(jq -r .error.message <<< "$output")" == *'File too large'* ]]
  cmp "$FLOX_CONFIG_DIR/defaults.toml" "$DEFAULTS_PROBE/saved"
}

@test "defaults: setup is instruction-only and inherited state skips configuration" {
  touch "$DEFAULTS_PROBE/not-a-directory"
  export FLOX_CONFIG_DIR="$DEFAULTS_PROBE/not-a-directory/config"
  for shell_name in bash zsh fish tcsh; do
    run "$FLOX_BIN" defaults setup --shell "$shell_name" --print
    assert_success
    assert_output --partial 'flox defaults _activate-if-saved'
  done
  test ! -e "$FLOX_DATA_DIR"
  export _FLOX_ACTIVE_ENVIRONMENTS='[{"environment":{"type":"dot-flox","path":"/missing/.flox","pointer":{"name":"fred","version":1}},"mode":"dev"}]'
  run --separate-stderr "$FLOX_BIN" defaults _activate-if-saved
  assert_success
  assert_equal "$output" ''
  assert_equal "$stderr" ''
  export _FLOX_ACTIVE_ENVIRONMENTS='broken inherited state'
  run --separate-stderr "$FLOX_BIN" defaults _activate-if-saved
  assert_success
  assert_equal "$output" ''
  [[ "$stderr" == *'skipping startup selection'* ]]
}

@test "defaults: absent selection is quiet at startup and explicit activation errors" {
  run --separate-stderr "$FLOX_BIN" defaults _activate-if-saved
  assert_success
  assert_equal "$output" ''
  assert_equal "$stderr" ''
  run --separate-stderr "$FLOX_BIN" defaults activate --print-script
  assert_failure
  assert_equal "$output" ''
  [[ "$stderr" == *'No startup environment is saved.'* ]]
}

@test "defaults: printed loaders dynamically select Fred and then Bob in all shells" {
  startup_setup
  for shell_name in bash zsh fish tcsh; do
    "$FLOX_BIN" defaults set --dir "$PROJECT_DIR"
    run_startup "$shell_name" '"$DEFAULTS_SH" "$DEFAULTS_PROBE/record.sh" fred'
    assert_success
    assert_saved_path fred "$PROJECT_DIR"
    "$FLOX_BIN" defaults set --dir "$OTHER_DIR"
    run_startup "$shell_name" '"$DEFAULTS_SH" "$DEFAULTS_PROBE/record.sh" bob'
    assert_success
    assert_saved_path bob "$OTHER_DIR"
    "$FLOX_BIN" defaults clear
    run_startup "$shell_name" '"$DEFAULTS_SH" "$DEFAULTS_PROBE/record.sh" cleared'
    assert_success
    assert_equal "$(cat "$DEFAULTS_PROBE/cleared.state")" '[]'
  done
}

@test "defaults: nested shells preserve inherited Fred after saving Bob" {
  startup_setup
  cat > "$DEFAULTS_PROBE/nested" <<'EOF'
"$DEFAULTS_SH" "$DEFAULTS_PROBE/record.sh" parent
"$FLOX_BIN" defaults set --dir "$OTHER_DIR" --json > "$DEFAULTS_PROBE/mutation.json"
"$DEFAULTS_CHILD" -ic '"$DEFAULTS_SH" "$DEFAULTS_PROBE/record.sh" child'
"$DEFAULTS_SH" "$DEFAULTS_PROBE/record.sh" after
EOF
  for shell_name in bash zsh fish tcsh; do
    "$FLOX_BIN" defaults set --dir "$PROJECT_DIR"
    export DEFAULTS_CHILD="$(command -v "$shell_name")"
    run_startup "$shell_name" 'source "$DEFAULTS_PROBE/nested"'
    assert_success
    assert_saved_path parent "$PROJECT_DIR"
    assert_saved_path child "$PROJECT_DIR"
    cmp "$DEFAULTS_PROBE/parent.state" "$DEFAULTS_PROBE/after.state"
  done
}

@test "defaults: printed loaders discard failed producer output and report evaluation failures" {
  startup_setup
  mkdir "$DEFAULTS_PROBE/bin"
  cat > "$DEFAULTS_PROBE/bin/flox" <<'EOF'
#!/bin/sh
printf '%s\n' '"$DEFAULTS_SH" "$DEFAULTS_PROBE/record.sh" partial'
if [ "$DEFAULTS_FAILURE" = producer ]; then exit 37; fi
printf '%s\n' false
EOF
  chmod +x "$DEFAULTS_PROBE/bin/flox"
  export PATH="$DEFAULTS_PROBE/bin:$PATH"
  for shell_name in bash zsh fish tcsh; do
    export DEFAULTS_FAILURE=producer
    run_startup "$shell_name" '"$DEFAULTS_SH" "$DEFAULTS_PROBE/record.sh" usable'
    assert_success
    assert_output --partial 'generated output was not evaluated'
    test -f "$DEFAULTS_PROBE/usable.state"
    test ! -e "$DEFAULTS_PROBE/partial.state"
    export DEFAULTS_FAILURE=evaluation
    run_startup "$shell_name" '"$DEFAULTS_SH" "$DEFAULTS_PROBE/record.sh" usable'
    assert_success
    assert_output --partial 'may have partial activation effects'
    test -f "$DEFAULTS_PROBE/partial.state"
    rm "$DEFAULTS_PROBE/partial.state" "$DEFAULTS_PROBE/usable.state"
  done
}

@test "defaults: explicit activation preserves commands, exit status, options, and layering" {
  startup_setup
  "$FLOX_BIN" defaults set --dir "$PROJECT_DIR"
  export FLOX_SHELL="$(command -v bash)"
  run --separate-stderr "$FLOX_BIN" defaults activate --mode run --no-start-services --add-sbin -- \
    "$DEFAULTS_SH" -c 'printf "%s\n" "$1" "$2"; printf "%s\n" "$_FLOX_ACTIVE_ENVIRONMENTS" > "$DEFAULTS_PROBE/direct.state"; exit 37' sh 'two words' 'literal $() ; "'
  assert_equal "$status" 37
  assert_equal "$output" $'two words\nliteral $() ; "'
  assert_saved_path direct "$PROJECT_DIR"
  assert_equal "$(jq -r '.[0].mode' "$DEFAULTS_PROBE/direct.state")" run
  run "$FLOX_BIN" defaults activate -c 'exit 38'
  assert_equal "$status" 38

  # The selector follows ordinary layering, with no implicit replacement.
  run "$FLOX_BIN" activate -d "$OTHER_DIR" -- "$FLOX_BIN" defaults activate -- \
    "$DEFAULTS_SH" "$DEFAULTS_PROBE/record.sh" layered
  assert_success
  jq -e --arg fred "$PROJECT_DIR/.flox" --arg bob "$OTHER_DIR/.flox" \
    'map(.environment.path) == [$fred, $bob]' "$DEFAULTS_PROBE/layered.state"
  run --separate-stderr "$FLOX_BIN" defaults list --json
  assert_success
  assert_equal "$(jq -r .selection.path <<< "$output")" "$PROJECT_DIR"
}

@test "defaults: an actually deactivated parent allows saved Bob in new interactive children" {
  startup_setup
  enable_prompt_hook
  "$FLOX_BIN" defaults set --dir "$PROJECT_DIR"
  export DEFAULTS_EXPECT="$(command -v expect)"
  cat > "$DEFAULTS_PROBE/deactivate.bash" <<'EOF'
"$DEFAULTS_SH" "$DEFAULTS_PROBE/record.sh" before-deactivate
"$FLOX_BIN" defaults set --dir "$OTHER_DIR" --json > "$DEFAULTS_PROBE/mutation.json"
eval "$("$FLOX_BIN" deactivate --print-script "$_FLOX_INVOCATION_TYPES")"
"$DEFAULTS_SH" "$DEFAULTS_PROBE/record.sh" inactive-parent
for shell_name in bash zsh fish tcsh; do
  export DEFAULTS_CHILD_STATE="after-deactivate-$shell_name"
  env DEFAULTS_INPUT='"$DEFAULTS_SH" "$DEFAULTS_PROBE/record.sh" "$DEFAULTS_CHILD_STATE"' \
    "$DEFAULTS_EXPECT" "$DEFAULTS_PROBE/pty.exp" "$(command -v "$shell_name")" -i
done
"$DEFAULTS_SH" "$DEFAULTS_PROBE/record.sh" unchanged-parent
EOF
  run_startup bash 'source "$DEFAULTS_PROBE/deactivate.bash"'
  assert_success
  assert_saved_path before-deactivate "$PROJECT_DIR"
  assert_equal "$(cat "$DEFAULTS_PROBE/inactive-parent.state")" '[]'
  cmp "$DEFAULTS_PROBE/inactive-parent.state" "$DEFAULTS_PROBE/unchanged-parent.state"
  for shell_name in bash zsh fish tcsh; do
    assert_saved_path "after-deactivate-$shell_name" "$OTHER_DIR"
  done
}

@test "defaults: explicit interactive children retain the selected stack through startup files" {
  startup_setup
  "$FLOX_BIN" defaults set --dir "$PROJECT_DIR"
  for shell_name in bash zsh fish tcsh; do
    local shell_path
    shell_path="$(command -v "$shell_name")"
    run env FLOX_SHELL="$shell_path" _FLOX_SHELL_FORCE="$shell_path" \
      DEFAULTS_INPUT='"$DEFAULTS_SH" "$DEFAULTS_PROBE/record.sh" explicit-child' \
      expect "$DEFAULTS_PROBE/pty.exp" "$FLOX_BIN" defaults activate
    assert_success
    assert_saved_path explicit-child "$PROJECT_DIR"
  done
}

@test "defaults: saved published references use ordinary remote activation" {
  startup_setup
  floxhub_setup test
  "$FLOX_BIN" push -d "$PROJECT_DIR" --owner test
  "$FLOX_BIN" defaults set test/fred
  export FLOX_SHELL="$(command -v bash)"
  run "$FLOX_BIN" defaults activate --trust --no-start-services -- "$DEFAULTS_SH" -c 'printf "remote-selected\n"'
  assert_success
  assert_output --partial remote-selected
  run --separate-stderr "$FLOX_BIN" defaults list --json
  assert_success
  assert_equal "$(jq -r .selection.reference <<< "$output")" test/fred
}

@test "defaults: noninteractive sourcing does not invoke the startup producer" {
  startup_setup
  mkdir "$DEFAULTS_PROBE/bin"
  cat > "$DEFAULTS_PROBE/bin/flox" <<'EOF'
#!/bin/sh
touch "$DEFAULTS_PROBE/producer-called"
exit 1
EOF
  chmod +x "$DEFAULTS_PROBE/bin/flox"
  export PATH="$DEFAULTS_PROBE/bin:$PATH"
  for shell_name in bash zsh fish tcsh; do
    export DEFAULTS_LOADER="$DEFAULTS_PROBE/loader"
    "$FLOX_BIN" defaults setup --shell "$shell_name" --print > "$DEFAULTS_LOADER"
    run "$(command -v "$shell_name")" -c 'source "$DEFAULTS_LOADER"; "$DEFAULTS_SH" "$DEFAULTS_PROBE/record.sh" noninteractive'
    assert_success
    test ! -e "$DEFAULTS_PROBE/producer-called"
    assert_equal "$(cat "$DEFAULTS_PROBE/noninteractive.state")" '[]'
  done
}

@test "defaults: missing source and corrupt configuration leave all startup shells usable" {
  startup_setup
  "$FLOX_BIN" defaults set --dir "$PROJECT_DIR"
  cp "$FLOX_CONFIG_DIR/defaults.toml" "$DEFAULTS_PROBE/saved"
  mv "$PROJECT_DIR" "$PROJECT_DIR-missing"
  for shell_name in bash zsh fish tcsh; do
    run_startup "$shell_name" '"$DEFAULTS_SH" "$DEFAULTS_PROBE/record.sh" missing'
    assert_success
    assert_output --partial 'startup activation failed'
    assert_equal "$(cat "$DEFAULTS_PROBE/missing.state")" '[]'
    cmp "$FLOX_CONFIG_DIR/defaults.toml" "$DEFAULTS_PROBE/saved"
  done
  printf '%s\n' 'version = 999' > "$FLOX_CONFIG_DIR/defaults.toml"
  for shell_name in bash zsh fish tcsh; do
    run_startup "$shell_name" '"$DEFAULTS_SH" "$DEFAULTS_PROBE/record.sh" corrupt'
    assert_success
    assert_output --partial 'Invalid startup configuration'
    assert_equal "$(cat "$DEFAULTS_PROBE/corrupt.state")" '[]'
    assert_equal "$(cat "$FLOX_CONFIG_DIR/defaults.toml")" 'version = 999'
  done
}

@test "defaults: tcsh startup respects a user's noclobber setting" {
  startup_setup
  "$FLOX_BIN" defaults set --dir "$PROJECT_DIR"
  { printf '%s\n' 'set noclobber'; "$FLOX_BIN" defaults setup --shell tcsh --print; } > "$HOME/.tcshrc"
  run_startup tcsh '"$DEFAULTS_SH" "$DEFAULTS_PROBE/record.sh" noclobber'
  assert_success
  assert_saved_path noclobber "$PROJECT_DIR"
}

@test "defaults: customized zsh and fish startup locations use the dynamic loader" {
  startup_setup
  "$FLOX_BIN" defaults set --dir "$PROJECT_DIR"
  export ZDOTDIR="$DEFAULTS_PROBE/zsh config"
  mkdir -p "$ZDOTDIR"
  mv "$HOME/.zshrc" "$ZDOTDIR/.zshrc"
  run_startup zsh '"$DEFAULTS_SH" "$DEFAULTS_PROBE/record.sh" customized-zsh'
  assert_success
  assert_saved_path customized-zsh "$PROJECT_DIR"
  export XDG_CONFIG_HOME="$DEFAULTS_PROBE/xdg config"
  mkdir -p "$XDG_CONFIG_HOME/fish"
  mv "$HOME/.config/fish/config.fish" "$XDG_CONFIG_HOME/fish/config.fish"
  run_startup fish '"$DEFAULTS_SH" "$DEFAULTS_PROBE/record.sh" customized-fish'
  assert_success
  assert_saved_path customized-fish "$PROJECT_DIR"
}

assert_path_activation_parity() {
  local tested_shell="$1" startup_file variant route next_path input result
  startup_setup
  case "$tested_shell" in
    bash) startup_file="$HOME/.bashrc" ;;
    zsh) startup_file="$HOME/.zshrc" ;;
    fish) startup_file="$HOME/.config/fish/config.fish" ;;
    tcsh) startup_file="$HOME/.tcshrc" ;;
  esac
  cp "$startup_file" "$DEFAULTS_PROBE/loader"
  export FLOX_SHELL="$tested_shell"

  # CLI-256 tracks desired success for quote-containing paths as an optional
  # existing activation defect. Compare actual behavior, never require failure:
  # https://linear.app/floxdotdev/issue/CLI-256/optional-handle-quotation-marks-in-activation-directory-names
  for variant in simple spaces quotes; do
    case "$variant" in
      simple) next_path="$DEFAULTS_PROBE/fred" ;;
      spaces) next_path="$DEFAULTS_PROBE/fred space" ;;
      quotes) next_path="$DEFAULTS_PROBE/fred space'\"\$;[x]" ;;
    esac
    if [ "$PROJECT_DIR" != "$next_path" ]; then mv "$PROJECT_DIR" "$next_path"; fi
    export PROJECT_DIR="$next_path"
    "$FLOX_BIN" defaults set --dir "$PROJECT_DIR"
    run "$FLOX_BIN" defaults list --json
    assert_success
    jq -e --arg path "$PROJECT_DIR" '.selection == {kind: "local", path: $path}' <<< "$output"
    cp "$FLOX_CONFIG_DIR/defaults.toml" "$DEFAULTS_PROBE/saved"

    # Explicit activation must not also run the saved startup loader.
    : > "$startup_file"
    for route in ordinary defaults; do
      result="$DEFAULTS_PROBE/$variant-$route"
      export DEFAULTS_PARITY_RESULT="$variant-$route"
      local -a args=(activate --print-script --dir "$PROJECT_DIR")
      if [ "$route" = defaults ]; then args=(defaults activate --print-script); fi
      run --separate-stderr "$FLOX_BIN" "${args[@]}"
      printf '%s\n' "$status" > "$result.producer"
      input='"$DEFAULTS_SH" "$DEFAULTS_PROBE/record.sh" "$DEFAULTS_PARITY_RESULT"'
      if [ "$status" -eq 0 ]; then
        printf '%s\n' "$output" > "$DEFAULTS_PROBE/generated"
        case "$tested_shell" in
          bash|zsh) input=$'source "$DEFAULTS_PROBE/generated"\necho $? > "$DEFAULTS_PROBE/$DEFAULTS_PARITY_RESULT.evaluation"\n'"$input" ;;
          fish|tcsh) input=$'source "$DEFAULTS_PROBE/generated"\necho $status > "$DEFAULTS_PROBE/$DEFAULTS_PARITY_RESULT.evaluation"\n'"$input" ;;
        esac
      fi
      run_startup "$tested_shell" "$input"
      assert_success
      jq -S . "$result.state" > "$result.sorted"
    done
    assert_equal "$(cat "$DEFAULTS_PROBE/$variant-defaults.producer")" "$(cat "$DEFAULTS_PROBE/$variant-ordinary.producer")"
    cmp "$DEFAULTS_PROBE/$variant-ordinary.sorted" "$DEFAULTS_PROBE/$variant-defaults.sorted"
    if [ "$variant" != quotes ]; then
      assert_equal "$(cat "$DEFAULTS_PROBE/$variant-ordinary.producer")" 0
      assert_equal "$(cat "$DEFAULTS_PROBE/$variant-ordinary.evaluation")" 0
    fi
    if [ "$(cat "$DEFAULTS_PROBE/$variant-ordinary.producer")" -eq 0 ]; then
      test -f "$DEFAULTS_PROBE/$variant-ordinary.evaluation"
      test -f "$DEFAULTS_PROBE/$variant-defaults.evaluation"
      assert_equal "$(cat "$DEFAULTS_PROBE/$variant-defaults.evaluation")" "$(cat "$DEFAULTS_PROBE/$variant-ordinary.evaluation")"
      if [ "$(cat "$DEFAULTS_PROBE/$variant-ordinary.evaluation")" -eq 0 ]; then
        assert_saved_path "$variant-ordinary" "$PROJECT_DIR"
      fi
    fi

    cp "$DEFAULTS_PROBE/loader" "$startup_file"
    export DEFAULTS_PARITY_RESULT="$variant-startup"
    run_startup "$tested_shell" '"$DEFAULTS_SH" "$DEFAULTS_PROBE/record.sh" "$DEFAULTS_PARITY_RESULT"'
    assert_success
    jq -S . "$DEFAULTS_PROBE/$variant-startup.state" > "$DEFAULTS_PROBE/$variant-startup.sorted"
    cmp "$DEFAULTS_PROBE/$variant-ordinary.sorted" "$DEFAULTS_PROBE/$variant-startup.sorted"
    cmp "$DEFAULTS_PROBE/saved" "$FLOX_CONFIG_DIR/defaults.toml"
    wait_for_activations "$PROJECT_DIR" || return 1
  done
}

@test "defaults: bash activation paths match ordinary activation" { assert_path_activation_parity bash; }
@test "defaults: zsh activation paths match ordinary activation" { assert_path_activation_parity zsh; }
@test "defaults: fish activation paths match ordinary activation" { assert_path_activation_parity fish; }
@test "defaults: tcsh activation paths match ordinary activation" { assert_path_activation_parity tcsh; }

@test "defaults: temporary script creation failure leaves fish and tcsh usable" {
  startup_setup
  mkdir "$DEFAULTS_PROBE/bin"
  printf '%s\n' '#!/bin/sh' 'exit 1' > "$DEFAULTS_PROBE/bin/mktemp"
  chmod +x "$DEFAULTS_PROBE/bin/mktemp"
  export PATH="$DEFAULTS_PROBE/bin:$PATH"
  for shell_name in fish tcsh; do
    run_startup "$shell_name" '"$DEFAULTS_SH" "$DEFAULTS_PROBE/record.sh" no-temp'
    assert_success
    assert_output --partial 'could not create a temporary startup script'
    assert_equal "$(cat "$DEFAULTS_PROBE/no-temp.state")" '[]'
  done
}

@test "defaults: hook and profile children preserve the activation already starting" {
  startup_setup
  export DEFAULTS_EXPECT="$(command -v expect)"
  cat > "$DEFAULTS_PROBE/children.sh" <<'EOF'
phase="$1"
for shell_name in bash zsh fish tcsh; do
  export DEFAULTS_CHILD_STATE="$phase-$shell_name"
  env DEFAULTS_INPUT='"$DEFAULTS_SH" "$DEFAULTS_PROBE/record.sh" "$DEFAULTS_CHILD_STATE"' \
    "$DEFAULTS_EXPECT" "$DEFAULTS_PROBE/pty.exp" "$(command -v "$shell_name")" -i
done
EOF
  cat > "$DEFAULTS_PROBE/manifest.toml" <<'EOF'
version = 1
[hook]
on-activate = '''
"$FLOX_BIN" defaults set --dir "$OTHER_DIR"
"$DEFAULTS_SH" "$DEFAULTS_PROBE/children.sh" hook
'''
[profile]
bash = '''
"$DEFAULTS_SH" "$DEFAULTS_PROBE/children.sh" profile
'''
EOF
  "$FLOX_BIN" edit -d "$PROJECT_DIR" -f "$DEFAULTS_PROBE/manifest.toml"
  "$FLOX_BIN" defaults set --dir "$PROJECT_DIR"
  run_startup bash '"$DEFAULTS_SH" "$DEFAULTS_PROBE/record.sh" outer'
  assert_success
  assert_saved_path outer "$PROJECT_DIR"
  for phase in hook profile; do
    for shell_name in bash zsh fish tcsh; do
      assert_saved_path "$phase-$shell_name" "$PROJECT_DIR"
    done
  done
}

@test "defaults: lifecycle telemetry keeps matching caller labels and ordinary activation events" {
  startup_setup
  "$FLOX_BIN" defaults set --dir "$PROJECT_DIR"
  export FLOX_DISABLE_METRICS=false
  export _FLOX_METRICS_URL_OVERRIDE="http://127.0.0.1:1/legacy"
  export _FLOX_METRICS_URL_V2_OVERRIDE="http://127.0.0.1:1/v2"
  mkdir -p "$FLOX_DATA_DIR"
  touch "$FLOX_DATA_DIR/events-v2-first-send"
  export FLOX_SHELL="$(command -v bash)"
  run "$FLOX_BIN" defaults activate -- true
  assert_success
  run "$FLOX_BIN" activate -d "$PROJECT_DIR" -- true
  assert_success
  run --separate-stderr "$FLOX_BIN" defaults _activate-if-saved
  assert_success
  for command_name in defaults::activate activate defaults::startup; do
    jq -se --arg command "$command_name" '
      [.[] | select(.payload.subcommand == $command)] as $events |
      ($events | map(.event_type) | sort) == ["cli.command_completed", "cli.command_run"] and
      ($events | map(.invocation_id) | unique | length) == 1
    ' "$FLOX_DATA_DIR/events-v2.json"
  done
  # Ordinary activation emits several domain rows. Compare against that path,
  # rather than assuming that one invocation produces only one domain event.
  jq -se '
    def activation_for($command):
      . as $all |
      (map(select(.event_type == "cli.command_run" and .payload.subcommand == $command))[0].invocation_id) as $id |
      $all | map(select(.event_type == "cli.environment.activate" and .invocation_id == $id) | .payload);
    activation_for("activate") as $ordinary |
    ($ordinary | length) > 0 and activation_for("defaults::activate") == $ordinary and
    (activation_for("defaults::startup") | length) > 0
  ' "$FLOX_DATA_DIR/events-v2.json"
}

@test "defaults: tmux startup follows stored server environment and fresh servers load Bob" {
  startup_setup
  enable_prompt_hook
  export DEFAULTS_TMUX="${DEFAULTS_TMUX:-$(command -v tmux)}"
  export DEFAULTS_BASH="$(command -v bash)"
  "$FLOX_BIN" defaults set --dir "$PROJECT_DIR"
  cat > "$DEFAULTS_PROBE/tmux-pane.bash" <<'EOF'
"$DEFAULTS_SH" "$DEFAULTS_PROBE/record.sh" "$DEFAULTS_CHILD_STATE"
sleep 30
EOF
  cat > "$DEFAULTS_PROBE/tmux.bash" <<'EOF'
set -e
export SHELL="$DEFAULTS_BASH" DEFAULTS_CHILD_STATE=stored-initial
"$FLOX_BIN" defaults set --dir "$OTHER_DIR"
"$DEFAULTS_TMUX" -S "$DEFAULTS_PROBE/stored.sock" -f /dev/null new-session -d -s defaults \
  "$DEFAULTS_BASH" -ic 'source "$DEFAULTS_PROBE/tmux-pane.bash"'
eval "$("$FLOX_BIN" deactivate --print-script "$_FLOX_INVOCATION_TYPES")"
"$DEFAULTS_TMUX" -S "$DEFAULTS_PROBE/stored.sock" new-window -e DEFAULTS_CHILD_STATE=stored-after \
  "$DEFAULTS_BASH" -ic 'source "$DEFAULTS_PROBE/tmux-pane.bash"'
export DEFAULTS_CHILD_STATE=fresh-server
"$DEFAULTS_TMUX" -S "$DEFAULTS_PROBE/fresh.sock" -f /dev/null new-session -d -s defaults \
  "$DEFAULTS_BASH" -ic 'source "$DEFAULTS_PROBE/tmux-pane.bash"'
for attempt in {1..100}; do
  if test -s "$DEFAULTS_PROBE/stored-initial.state" && test -s "$DEFAULTS_PROBE/stored-after.state" && test -s "$DEFAULTS_PROBE/fresh-server.state"; then break; fi
  sleep 0.1
done
"$DEFAULTS_TMUX" -S "$DEFAULTS_PROBE/stored.sock" kill-server
"$DEFAULTS_TMUX" -S "$DEFAULTS_PROBE/fresh.sock" kill-server
"$DEFAULTS_SH" "$DEFAULTS_PROBE/record.sh" inactive-client
EOF
  run_startup bash 'source "$DEFAULTS_PROBE/tmux.bash"'
  assert_success
  assert_saved_path stored-initial "$PROJECT_DIR"
  assert_saved_path stored-after "$PROJECT_DIR"
  assert_saved_path fresh-server "$OTHER_DIR"
  assert_equal "$(cat "$DEFAULTS_PROBE/inactive-client.state")" '[]'
}

@test "defaults: Ctrl-C during startup production returns to a usable prompt" {
  startup_setup
  mkdir "$DEFAULTS_PROBE/bin"
  cat > "$DEFAULTS_PROBE/bin/flox" <<'EOF'
#!/bin/sh
printf '%s\n' producer-ready >&2
sleep 30
EOF
  chmod +x "$DEFAULTS_PROBE/bin/flox"
  export PATH="$DEFAULTS_PROBE/bin:$PATH"
  cat > "$DEFAULTS_PROBE/interrupt.exp" <<'EOF'
set timeout 10
spawn -noecho {*}$argv
expect {
  "producer-ready" {}
  timeout { exit 124 }
  eof { exit 125 }
}
send -- "\003"
after 100
send -- "\"\$DEFAULTS_SH\" \"\$DEFAULTS_PROBE/record.sh\" interrupted\nexit\n"
expect {
  timeout { exit 124 }
  eof {}
}
set result [wait]
exit [lindex $result 3]
EOF
  for shell_name in bash zsh fish tcsh; do
    run expect "$DEFAULTS_PROBE/interrupt.exp" "$(command -v "$shell_name")" -i
    assert_success
    assert_equal "$(cat "$DEFAULTS_PROBE/interrupted.state")" '[]'
    rm "$DEFAULTS_PROBE/interrupted.state"
  done
}
