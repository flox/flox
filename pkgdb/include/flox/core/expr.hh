/* ========================================================================== *
 *
 * @file flox/core/expr.hh
 *
 * @brief Extensions to `libnix-expr`, the `nix` expression language.
 *
 * Adds new `nix` primitive operations, and provides several helper functions.
 *
 *
 * -------------------------------------------------------------------------- */

#pragma once

#include <nix/eval.hh>
#include <nix/flake/flakeref.hh>


/* -------------------------------------------------------------------------- */

namespace flox {

/* -------------------------------------------------------------------------- */

/**
 * @brief Evaluate trivial thunks to values.
 *        This has no effect on non-thunks and non-trivial values.
 *
 * For example, values such as `{ foo = 1; }` may be represented as a thunk, so
 * to process conditional blocks based on `type()` we must evaluate the thunk
 * to find it's _real_ type first.
 */
void
forceTrivialValue( nix::EvalState &  state,
                   nix::Value &      value,
                   const nix::PosIdx pos = nix::noPos );


/* -------------------------------------------------------------------------- */

/**
 * @brief Convert a `nix::Value` attribute set or string into
 *        a `nix::FlakeRef`.
 */
[[nodiscard]] nix::FlakeRef
valueToFlakeRef( nix::EvalState &    state,
                 nix::Value &        value,
                 const nix::PosIdx   pos = nix::noPos,
                 const std::string & errorMsg
                 = "while parsing flake reference" );


/* -------------------------------------------------------------------------- */

}  // namespace flox


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
