/* ========================================================================== *
 *
 * @file flox/resolver/primops.hh
 *
 * @brief Extensions to `nix` primitive operations.
 *
 *
 * -------------------------------------------------------------------------- */

#pragma once

#include "flox/core/nix-state.hh"


/* -------------------------------------------------------------------------- */

namespace flox::resolver {

/* -------------------------------------------------------------------------- */

/**
 * @brief Resolve a descriptor to an installable.
 *
 * Takes the following arguments:
 * - `options`: An attribute set of `flox::Options`.
 * - `flake-ref`: Either an attribute set or string.
 * - `query`: Either a string or attribute set representing a descriptor.
 *
 * @param state The `nix` evaluator's state.
 * @param pos The position ( file name and line/column numbers ) of the call.
 *            This is generally used for error reporting.
 * @param args The arguments to the primitive.
 * @param value An allocated `nix::Value` to store the result in.
 */
void
prim_resolve( nix::EvalState &  state,
              const nix::PosIdx pos,
              nix::Value **     args,
              nix::Value &      value );


/* -------------------------------------------------------------------------- */

}  // namespace flox::resolver


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
