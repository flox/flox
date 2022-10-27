# Flox Rust SDK / CLI Prototype

This is a prototype abstraction layer that can be used by the Flox UI and an attempt to create a faithful future replacement of the flox bash script in a 
language that can be easier supported, extended, and collaborated on than a collection of bash scripts.

## Design Choices

This code is in its very early stages and should only be thought of as a draft prototype. 

In order to provide a way to extend flox in the future with minimal breaking changes to the API consumers I have seperated the different functions that use external dependnecies into different Provider traits. These traits will provide a common "floxesque" interface but allow us to change out implementation details trivially.

The application will be async so parallelization of certain tasks (e.g. searching) will be possible in future versions.

The rust SDK will then be used to genrate an api via https://github.com/fzyzcjy/flutter_rust_bridge and a UI will be built in flutter / dart. 

## Development (with flox)

1. `flox develop -A rust-env` → sets up an environment with rust, rustfmt, clippy, rust-analyzer and pre-commit-hooks

## Development (without flox)

1. Download rustup - `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`
1. Install rust nightly `rustup default nightly`
1. Set NIX_BIN and FLOX_SH environment variables
1. Hack away

## Providers

Providers allow for dependency injection of different parts.

## References 

Github interaction is provided by octocrab : https://github.com/XAMPPRocky/octocrab
CLI argument parsing is provided by Clap using the Derive feature
Sam proposed that is to at some point integrate cxx binding support so nix can be used via FFI and not the command prompt. He provided me with some examples in go and haskell
    - https://cxx.rs/ 
    - https://github.com/nix-community/go-nix  
    - https://github.com/Profpatsch/libnix-haskell#readme  
    - https://www.haskellforall.com/2022/09/nix-serve-ng-faster-more-reliable-drop.html 
