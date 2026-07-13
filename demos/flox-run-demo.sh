#!/usr/bin/env bash
#
# flox run - Command-First Package Execution Demo (Phase 2)
#
# Run any command without installing it — and without knowing which
# package provides it. Flox looks the command up in the FloxHub
# command-to-package index.
#
# Phase 1 (shipped): flox run -p <package> <command>
# Phase 2 (this demo): the -p flag is optional.
#
# NOTE: the command-to-package index requires a FloxHub with the
# `packages/by-binary` endpoint (e.g. a local floxhub stack with the
# feat/binary-to-package-index branch). Against an older FloxHub the
# CLI falls back to a search-based heuristic.
#
# Usage: bash demos/flox-run-demo.sh

set -euo pipefail

# Colors
BOLD='\033[1m'
CYAN='\033[36m'
GREEN='\033[32m'
YELLOW='\033[33m'
DIM='\033[2m'
RESET='\033[0m'

step=0

header() {
    echo ""
    echo -e "${BOLD}${CYAN}========================================${RESET}"
    echo -e "${BOLD}${CYAN}  flox run - Command-First Execution${RESET}"
    echo -e "${BOLD}${CYAN}========================================${RESET}"
    echo ""
    echo -e "Run any command without installing it."
    echo -e "No package name needed. No environment. No cleanup."
    echo ""
}

demo_step() {
    step=$((step + 1))
    echo ""
    echo -e "${BOLD}${GREEN}--- Step ${step}: $1 ---${RESET}"
    echo ""
}

run_cmd() {
    echo -e "${YELLOW}\$ $*${RESET}"
    echo ""
    eval "$@"
    echo ""
}

pause() {
    echo -e "${DIM}Press Enter to continue...${RESET}"
    read -r
}

# ============================================================

header

echo -e "Equivalent commands in other tools:"
echo -e "  ${DIM}npx cowsay 'hello'${RESET}               # Node"
echo -e "  ${DIM}uvx ruff check .${RESET}                  # Python"
echo -e "  ${DIM}nix run nixpkgs#cowsay -- 'hello'${RESET} # Nix"
echo -e "  ${DIM}mise x node -- node --version${RESET}     # mise"
echo ""
echo -e "None of them can answer: ${BOLD}which package provides 'readelf'?${RESET}"
echo ""
pause

# ============================================================

demo_step "Just run a command - no package name needed"

echo "The command name is looked up in the FloxHub command-to-package index:"
echo ""
run_cmd flox run hello

pause

# ============================================================

demo_step "Command name != package name"

echo "readelf is provided by binutils. You don't need to know that:"
echo ""
run_cmd flox run readelf -- -a "\$(command -v ls)"

pause

# ============================================================

demo_step "Disambiguation - several packages provide 'vi'"

echo "When several packages provide a command and none matches the"
echo "command name exactly, Flox prompts you to choose."
echo "Your choice is saved as a preference. (Quit vi with :q)"
echo ""
run_cmd flox run vi

pause

# ============================================================

demo_step "Saved preference - no prompt the second time"

echo "The same command now runs silently with the saved choice:"
echo ""
run_cmd flox run vi

echo "The preference lives in the user config:"
echo ""
run_cmd flox config \| grep -A2 command_preferences

pause

# ============================================================

demo_step "Change your mind with --reselect"

echo "Clear the saved preference and choose again:"
echo ""
run_cmd flox run --reselect vi

pause

# ============================================================

demo_step "Explicit package and version pinning"

echo "--package bypasses the lookup (and saves the mapping)."
echo "Version constraints are allowed on the package spec:"
echo ""
run_cmd flox run -p hello@2.12 hello

pause

# ============================================================

demo_step "Which packages provide a command? flox search --command"

echo "The same index powers search:"
echo ""
run_cmd flox search --command rg

pause

# ============================================================

demo_step "Non-interactive: never prompts, never hangs"

echo "Without a terminal and without a saved preference, ambiguity"
echo "fails fast with the candidates listed inline:"
echo ""
run_cmd flox config --delete command_preferences.vi \|\| true
run_cmd "echo '' | flox run vi || true"

pause

# ============================================================

echo ""
echo -e "${BOLD}${CYAN}========================================${RESET}"
echo -e "${BOLD}${CYAN}  Demo Complete!${RESET}"
echo -e "${BOLD}${CYAN}========================================${RESET}"
echo ""
echo -e "Key takeaways:"
echo -e "  1. ${BOLD}No package name needed${RESET} - flox run <command> just works"
echo -e "  2. ${BOLD}Accurate lookup${RESET}        - backed by the command-to-package index"
echo -e "  3. ${BOLD}Exact match wins${RESET}       - package named like the command runs silently"
echo -e "  4. ${BOLD}Choices are saved${RESET}      - prompt once, reuse silently, --reselect to change"
echo -e "  5. ${BOLD}CI-safe${RESET}                - non-interactive runs never prompt or hang"
echo ""
