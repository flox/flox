# shellcheck shell=bash

# ============================================================================ #
#
# Setup language-specific environment variables (Rust, Jupyter, Java)
#
# ---------------------------------------------------------------------------- #

# Rust: Set RUST_SRC_PATH if rustLibSrc is in the environment
if [[ -d "$FLOX_ENV/rustc-std-workspace-std" ]]; then
  export RUST_SRC_PATH="$FLOX_ENV"
fi

# Jupyter: Set JUPYTER_PATH if Jupyter is in the environment
_jupyter_env="${FLOX_ENV}/share/jupyter"
if [[ -d "$_jupyter_env" ]]; then
  export JUPYTER_PATH="${_jupyter_env}${JUPYTER_PATH:+:$JUPYTER_PATH}"
fi

# Java: Set JAVA_HOME if Java is in the environment
if [[ -x "$FLOX_ENV/bin/java" ]]; then
  export JAVA_HOME="$FLOX_ENV"
fi

# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
