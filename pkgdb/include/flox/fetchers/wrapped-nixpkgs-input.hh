/* ========================================================================== *
 *
 * @file flox/fetchers/wrapped-nixpkgs-input.hh
 *
 * @brief Executable command helpers, argument parsers, etc.
 *
 *
 * -------------------------------------------------------------------------- */

#pragma once
#include <nix/fetchers.hh>

/* -------------------------------------------------------------------------- */

namespace flox {

/**
 * @brief Helper used to convert a `github` attribute set representation,
 *        to a `flox-nixpkgs` attribute set representation.
 */
nix::fetchers::Attrs
githubAttrsToFloxNixpkgsAttrs( const nix::fetchers::Attrs & attrs );

}  // namespace flox
