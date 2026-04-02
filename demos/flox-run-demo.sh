#!/usr/bin/env bash
#
# flox run - On-Demand Package Execution Demo
#
# Run any package binary without installing it into an environment.
# Similar to: npx (Node), uvx (Python), nix run (Nix), mise x (mise)
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
    echo -e "${BOLD}${CYAN}  flox run - On-Demand Package Execution${RESET}"
    echo -e "${BOLD}${CYAN}========================================${RESET}"
    echo ""
    echo -e "Run any package binary without installing it."
    echo -e "No environment setup needed. No cleanup required."
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
echo -e "  ${DIM}npx cowsay 'hello'${RESET}              # Node"
echo -e "  ${DIM}uvx ruff check .${RESET}                 # Python"
echo -e "  ${DIM}nix run nixpkgs#cowsay -- 'hello'${RESET} # Nix"
echo -e "  ${DIM}mise x node -- node --version${RESET}     # mise"
echo ""
pause

# ============================================================

demo_step "Basic usage - run a package you don't have installed"

echo "Let's use cowsay without installing it into any environment:"
echo ""
run_cmd flox run cowsay -- "Hello from Flox!"

pause

# ============================================================

demo_step "Version pinning - run a specific version"

echo "Pin to a specific Python version:"
echo ""
run_cmd flox run python3@3.13.12 -- --version

pause

# ============================================================

demo_step "Binary override with --bin"

echo "Some packages have binaries with different names than the package."
echo "Use --bin to specify which binary to run:"
echo ""
run_cmd flox run --bin npm nodejs -- --version

pause

# ============================================================

echo ""
echo -e "${BOLD}${CYAN}========================================${RESET}"
echo -e "${BOLD}${CYAN}  Demo Complete!${RESET}"
echo -e "${BOLD}${CYAN}========================================${RESET}"
echo ""
echo -e "Key takeaways:"
echo -e "  1. ${BOLD}No environment needed${RESET} - just flox run <package> -- <args>"
echo -e "  2. ${BOLD}Version pinning${RESET}      - use package@version syntax"
echo -e "  3. ${BOLD}Binary override${RESET}      - use --bin when binary name differs"
echo -e "  4. ${BOLD}Zero cleanup${RESET}          - temporary environment is removed automatically"
echo ""
