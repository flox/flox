/* ========================================================================== *
 *
 * @file resolver/primops.cc
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
#include "flox/registry.hh"
#include "flox/resolver/descriptor.hh"
#include "flox/resolver/environment.hh"
#include "flox/resolver/manifest-raw.hh"
#include "flox/resolver/manifest.hh"
#include "flox/resolver/primops.hh"


/* -------------------------------------------------------------------------- */

namespace flox::resolver {

/* -------------------------------------------------------------------------- */

void
prim_resolve( nix::EvalState &  state,
              const nix::PosIdx pos,
              nix::Value **     args,
              nix::Value &      value )
{
  state.forceAttrs( *args[0],
                    pos,
                    "while processing options argument to 'builtins.resolve'" );

  nix::NixStringContext context;
  Options               options
    = nix::printValueAsJSON( state, true, *args[0], pos, context, false );

  RegistryInput input( valueToFlakeRef(
    state,
    *args[1],
    pos,
    "while processing 'input' argument to 'builtins.resolve'" ) );
  RegistryRaw   registry;
  registry.inputs.emplace( std::make_pair( "input", std::move( input ) ) );


  if ( args[2]->isThunk() && args[2]->isTrivial() )
    {
      state.forceValue( *args[2], pos );
    }
  ManifestDescriptorRaw descriptor;
  auto                  type = args[2]->type();
  if ( type == nix::nAttrs )
    {
      state.forceAttrs(
        *args[2],
        pos,
        "while processing 'descriptor' argument to 'builtins.resolve'" );
      descriptor
        = nix::printValueAsJSON( state, true, *args[2], pos, context, false );
      if ( descriptor.systems.has_value() && ( ! options.systems.has_value() ) )
        {
          options.systems    = std::move( descriptor.systems );
          descriptor.systems = std::nullopt;
        }
    }
  else if ( type == nix::nString )
    {
      state.forceStringNoCtx(
        *args[2],
        pos,
        "while processing 'descriptor' argument to 'builtins.resolve'" );
      descriptor.name = args[2]->str();
    }
  else
    {
      state
        .error( "descriptor was expected to be a set or a string, but "
                "got '%s'",
                nix::showType( type ) )
        .debugThrow<nix::EvalError>();
    }

  std::unordered_map<std::string, std::optional<ManifestDescriptorRaw>> install;
  install.emplace( "__package", std::move( descriptor ) );

  ManifestRaw manifest;
  manifest.options  = std::move( options );
  manifest.registry = std::move( registry );
  manifest.install  = std::move( install );

  EnvironmentManifest envManifest( manifest );
  Environment         environment( std::nullopt, envManifest, std::nullopt );
  LockfileRaw         lock = environment.createLockfile().getLockfileRaw();

  nlohmann::json bySystem = nlohmann::json::object();
  for ( auto & [system, pkgs] : lock.packages )
    {
      bySystem.emplace( system, pkgs.at( "__package" ) );
    }

  nix::parseJSON( state, bySystem.dump(), value );
}


/* -------------------------------------------------------------------------- */

static nix::RegisterPrimOp
  primop_resolve( { .name                = "__resolve",
                    .args                = { "options", "input", "descriptor" },
                    .arity               = 0,
                    .doc                 = R"(
    Resolve a descriptor to an installable.
    Takes the following arguments:

    - `options`: An attribute set of `flox::Options`.

    - `input`: Either an attribute set or string flake-ref.

    - `descriptor`: Either a string or attribute set representing a descriptor.
                    The fields `name`, `version`, `path`, `absPath`, and
                    `systems` are respected.
  )",
                    .fun                 = prim_resolve,
                    .experimentalFeature = nix::Xp::Flakes } );


/* -------------------------------------------------------------------------- */

}  // namespace flox::resolver


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
