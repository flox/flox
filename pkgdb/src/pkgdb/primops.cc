/* ========================================================================== *
 *
 * @file pkgdb/primops.cc
 *
 * @brief Extensions to `nix` primitive operations.
 *
 *
 * -------------------------------------------------------------------------- */

#include <nix/flake/flake.hh>
#include <nix/json-to-value.hh>
#include <nix/primops.hh>
#include <nix/value-to-json.hh>
#include <nlohmann/json.hpp>

#include "flox/core/expr.hh"
#include "flox/core/nix-state.hh"
#include "flox/pkgdb/primops.hh"


/* -------------------------------------------------------------------------- */

namespace flox::pkgdb {

/* -------------------------------------------------------------------------- */

// NOLINTBEGIN(cppcoreguidelines-pro-bounds-pointer-arithmetic)
// Upstream nix code is using this pattern extensively,
// lets not break the convention
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
  nix::FlakeRef flakeRef = valueToFlakeRef(
    state,
    *args[0],
    pos,
    "while processing 'flakeRef' argument to 'builtins.getFingerprint'" );

  // nix::flake:: locked = nix::flake::lockFlake();
  nix::flake::LockedFlake locked
    = nix::flake::lockFlake( state, flakeRef, defaultLockFlags );
  value.mkString( locked.getFingerprint().to_string( nix::Base16, false ) );
}
// NOLINTEND(cppcoreguidelines-pro-bounds-pointer-arithmetic)


/* -------------------------------------------------------------------------- */

// NOLINTBEGIN(cert-err58-cpp)
// This can throw an exception that cannot be caught.
static const nix::RegisterPrimOp
  primop_getFingerprint( { .name                = "__getFingerprint",
                           .args                = { "flakeRef" },
                           .arity               = 0,
                           .doc                 = R"(
    This hash uniquely identifies a revision of a locked flake.
    Takes a single argument:

    - `flakeRef`: Either an attribute set or string flake-ref.
    )",
                           .fun                 = prim_getFingerprint,
                           .experimentalFeature = nix::Xp::Flakes } );
// NOLINTEND(cppcoreguidelines-pro-bounds-pointer-arithmetic)


/* -------------------------------------------------------------------------- */

}  // namespace flox::pkgdb


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
