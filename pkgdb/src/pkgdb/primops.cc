/* ========================================================================== *
 *
 * @file pkgdb/primops.cc
 *
 * @brief Extensions to `nix` primitive operations.
 *
 *
 * -------------------------------------------------------------------------- */

#include <nix/json-to-value.hh>
#include <nix/primops.hh>
#include <nix/value-to-json.hh>
#include <nlohmann/json.hpp>

#include "flox/core/expr.hh"
#include "flox/core/nix-state.hh"
#include "flox/pkgdb/pkg-query.hh"
#include "flox/pkgdb/primops.hh"
#include "flox/registry.hh"


/* -------------------------------------------------------------------------- */

namespace flox::pkgdb {

/* -------------------------------------------------------------------------- */

void
prim_getFingerprint( nix::EvalState &  state,
                     const nix::PosIdx pos,
                     nix::Value **     args,
                     nix::Value &      value )
{
  nix::NixStringContext context;

  if ( args[0]->isThunk() && args[0]->isTrivial() )
    {
      state.forceValue( *args[0], pos );
    }
  RegistryInput input( valueToFlakeRef(
    state,
    *args[0],
    pos,
    "while processing 'flakeRef' argument to 'builtins.getFingerprint'" ) );

  FloxFlakeInput flake( state.store, input );
  value.mkString(
    flake.getFlake()->lockedFlake.getFingerprint().to_string( nix::Base16,
                                                              false ) );
}


/* -------------------------------------------------------------------------- */

static nix::RegisterPrimOp primop_getFingerprint( { .name  = "__getFingerprint",
                                                    .args  = { "flakeRef" },
                                                    .arity = 0,
                                                    .doc   = R"(
    This hash uniquely identifies a revision of a locked flake.
    Takes a single argument:

    - `flakeRef`: Either an attribute set or string flake-ref.
    )",
                                                    .fun = prim_getFingerprint,
                                                    .experimentalFeature
                                                    = nix::Xp::Flakes } );


/* -------------------------------------------------------------------------- */

}  // namespace flox::pkgdb


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
