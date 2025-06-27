#!/usr/bin/env bash

set -euo pipefail

# rm -rf ../../test_data/generated2
cargo build
RUST_LOG=debug ../target/debug/mk_data -i ../../test_data/input_data -o ../../test_data/generated2 ../../test_data/config2.toml
