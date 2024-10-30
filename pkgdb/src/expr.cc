/* ========================================================================== *
 *
 * @file flox/nix-state.cc
 *
 * @brief Manages a `nix` runtime state blob with associated helpers.
 *
 *
 * -------------------------------------------------------------------------- */

#include <nix/eval.hh>
#include <nix/globals.hh>
#include <nix/shared.hh>
#include <nix/value-to-json.hh>

#include "flox/core/expr.hh"


/* -------------------------------------------------------------------------- */

namespace flox {

/* -------------------------------------------------------------------------- */

void
forceTrivialValue( nix::EvalState &  state,
                   nix::Value &      value,
                   const nix::PosIdx pos )
{
  if ( value.isThunk() && value.isTrivial() )
    {
      state.forceValue( value, pos );
    }
}


/* -------------------------------------------------------------------------- */

nix::FlakeRef
valueToFlakeRef( nix::EvalState &    state,
                 nix::Value &        value,
                 const nix::PosIdx   pos,
                 const std::string & errorMsg )
{
  nix::NixStringContext context;
  forceTrivialValue( state, value, pos );
  auto type = value.type();
  if ( type == nix::nAttrs )
    {
      state.forceAttrs( value, pos, errorMsg );
      return nix::FlakeRef::fromAttrs(
        state.fetchSettings,
        nix::fetchers::jsonToAttrs(
          nix::printValueAsJSON( state, true, value, pos, context, false ) ) );
    }

  if ( type == nix::nString )
    {
      state.forceStringNoCtx( value, pos, errorMsg );
      return nix::parseFlakeRef( state.fetchSettings,
                                 std::string( value.c_str() ) );
    }

  state
    .error<nix::EvalError>(
      "flake reference was expected to be a set or a string, but "
      "got '%s'",
      nix::showType( value ) )
    .debugThrow();
}


/* -------------------------------------------------------------------------- */

}  // namespace flox


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
