/* ========================================================================== *
 *
 * @file lock-flake-installable.hh
 *
 * @brief Executable command helper and `flox::lockFlakeInstallable`.
 *
 *
 * -------------------------------------------------------------------------- */

#include <fstream>

#include <nix/attr-path.hh>
#include <nix/eval.hh>
#include <nix/installable-flake.hh>
#include <nix/value-to-json.hh>

#include "flox/lock-flake-installable.hh"


/* -------------------------------------------------------------------------- */

namespace flox {

/* -------------------------------------------------------------------------- */

LockCommand::LockCommand() : parser( "lock-flake-installable" )
{
  this->parser.add_description(
    "Lock a flake installable and return its lock data as json" );

  this->parser.add_argument( "url" )
    .help( "The flake installable to lock" )
    .action( [&]( const std::string & value ) { this->installable = value; } );

  auto arg
    = this->parser.add_argument( "--system" )
        .metavar( "SYSTEM" )
        .help( "The system to lock the flake installable for" )
        .default_value( nix::settings.thisSystem.get() )
        .nargs( 1 )
        .action( [&]( const std::string & value ) { this->system = value; } );
}


/* -------------------------------------------------------------------------- */

int
LockCommand::run()
{
  auto state = this->getState();

  auto lockedInstallable
    = lockFlakeInstallable( this->getState(), this->system, this->installable );

  printf( "%s\n", nlohmann::json( lockedInstallable ).dump( 2 ).c_str() );

  return EXIT_SUCCESS;
};

/**
 * @brief Parse the installable string into a flake reference, fragment and
 * extended outputs spec.
 * @param state The nix evaluation state
 * @param installableStr The installable string
 * @return A tuple containing the flake reference, fragment and extended outputs
 * @throws LockFlakeInstallableException if the installable string could not be
 * parsed
 */
static std::tuple<nix::FlakeRef, std::string, nix::ExtendedOutputsSpec>
parseInstallable( const std::string & installableStr )
{
  try
    {
      return nix::parseFlakeRefWithFragmentAndExtendedOutputsSpec(
        installableStr );
    }
  catch ( const nix::Error & e )
    {
      throw LockFlakeInstallableException( "could not parse flake installable",
                                           e.info().msg.str() );
    }
}

/**
 * @brief Locate the installable in the flake and return a locked installable.
 * Locks the referenced flake if necessary, but does not apply updates
 * or writes any local state.
 * @param state The nix evaluation state
 * @param flakeRef The flake reference
 * @param fragment The attrpath fragment e.g. everything right of the `#` in a
 * flake installable (excluding output specifiers)
 * @param extendedOutputsSpec The outputs specified with `^<outputs>` in a flake
 * installable
 * @return A locked @a nix::InstallableFlake
 * @throws @a LockFlakeInstallableException if the installable could not be
 * located or the flakeref could not be locked
 */
static nix::InstallableFlake
locateInstallable( const nix::ref<nix::EvalState> & state,
                   const std::string &              system,
                   nix::FlakeRef &                  flakeRef,
                   const std::string &              fragment,
                   const nix::ExtendedOutputsSpec & extendedOutputsSpec,
                   const nix::flake::LockFlags &    lockFlags )
{
  try
    {
      nix::InstallableFlake installable = nix::InstallableFlake(
        static_cast<nix::SourceExprCommand *>( nullptr ),
        state,
        std::move( flakeRef ),
        fragment,
        extendedOutputsSpec,
        // Defaults from nix:
        // <https://github.com/NixOS/nix/blob/142e566adbce587a5ed97d1648a26352f0608ec5/src/libcmd/installables.cc#L231>
        nix::Strings {
          "packages." + system + ".default",
          "defaultPackage." + system,
        },
        // Defaults from nix:
        // <https://github.com/NixOS/nix/blob/142e566adbce587a5ed97d1648a26352f0608ec5/src/libcmd/installables.cc#L236>
        nix::Strings {
          "packages." + system + ".",
          "legacyPackages." + system + ".",
        },
        lockFlags );

      return installable;
    }
  catch ( const nix::Error & e )
    {
      throw LockFlakeInstallableException( "could not locate flake installable",
                                           e.info().msg.str() );
    }
}

LockedInstallable
lockFlakeInstallable( const nix::ref<nix::EvalState> & state,
                      const std::string &              system,
                      const std::string &              installableStr )
{
  debugLog( nix::fmt( "original installable: %s", installableStr ) );

  auto parsed = parseInstallable( installableStr );

  nix::FlakeRef            flakeRef            = std::get<0>( parsed );
  std::string              fragment            = std::get<1>( parsed );
  nix::ExtendedOutputsSpec extendedOutputsSpec = std::get<2>( parsed );

  debugLog(
    nix::fmt( "original flakeRef: '%s'", flakeRef.to_string().c_str() ) );
  debugLog( nix::fmt( "original fragment: '%s'", fragment ) );
  debugLog( nix::fmt( "original extendedOutputsSpec: '%s'",
                      extendedOutputsSpec.to_string() ) );

  auto lockFlags = nix::flake::LockFlags {
    .recreateLockFile      = false,
    .updateLockFile        = false,
    .writeLockFile         = false,
    .useRegistries         = false,
    .applyNixConfig        = false,
    .allowUnlocked         = true,
    .commitLockFile        = false,
    .referenceLockFilePath = std::nullopt,
    .outputLockFilePath    = std::nullopt,
    .inputOverrides        = std::map<nix::flake::InputPath, nix::FlakeRef> {},
    .inputUpdates          = std::set<nix::flake::InputPath> {}
  };

  auto installable = locateInstallable( state,
                                        system,
                                        flakeRef,
                                        fragment,
                                        extendedOutputsSpec,
                                        lockFlags );

  debugLog(
    nix::fmt( "locked installable: '%s'", installable.what().c_str() ) );


  auto lockedUrl = installable.getLockedFlake()->flake.lockedRef.to_string();
  debugLog( nix::fmt( "locked url: '%s'", lockedUrl ) );

  auto flakeDescription = installable.getLockedFlake()->flake.description;

  auto cursor = installable.getCursor( *state );

  auto lockedAttrPath = cursor->getAttrPathStr();
  debugLog( nix::fmt( "locked attr path: '%s'", lockedAttrPath ) );

  debugLog( nix::fmt( "locked outputs: '%s'",
                      installable.extendedOutputsSpec.to_string() ) );

  // check if the output is a derivation (not a just a store path)
  if ( ! cursor->isDerivation() )
    {
      auto v = cursor->forceValue();
      throw LockFlakeInstallableException( nix::fmt(
        "expected flake output attribute '%s' to be a derivation but found %s",
        lockedAttrPath,
        nix::showType( v ) ) );
    }

  // read the drv path
  std::string derivation;
  {
    auto derivationCursor
      = cursor->findAlongAttrPath( nix::parseAttrPath( *state, "drvPath" ) );
    if ( ! derivationCursor )
      {
        throw LockFlakeInstallableException(
          nix::fmt( "could not find '%s.%s' in derivation",
                    lockedAttrPath,
                    "drvPath" ) );
      }
    derivation = ( *derivationCursor )->getStringWithContext().first;
  }

  // map output names to their store paths
  std::map<std::string, std::string> outputs;
  std::vector<std::string>           outputNames;
  {
    auto maybe_outputs_cursor
      = cursor->findAlongAttrPath( nix::parseAttrPath( *state, "outputs" ) );
    if ( ! maybe_outputs_cursor )
      {
        throw LockFlakeInstallableException(
          nix::fmt( "could not find '%s.%s' in derivation",
                    lockedAttrPath,
                    "outputs" ) );
      }
    outputNames = ( *maybe_outputs_cursor )->getListOfStrings();

    for ( auto output : outputNames )
      {
        auto outputCursor = cursor->findAlongAttrPath(
          nix::parseAttrPath( *state, output + ".outPath" ) );
        if ( ! outputCursor )
          {
            throw LockFlakeInstallableException(
              nix::fmt( "could not find '%s.%s' in derivation",
                        lockedAttrPath,
                        output + ".outPath" ) );
          }
        auto outputValue = ( *outputCursor )->getStringWithContext();
        outputs[output]  = outputValue.first;
      }
  }

  // determine outputs to install in the following order:
  // 1. extendedOutputsSpec (`<installable>^out,man`, `<installable>^*`))
  // 2. `meta.outputsToInstall` field
  // 3. first output in the `outputs` field
  std::set<std::string> outputsToInstall;
  {
    auto outputSpec = std::visit(
      overloaded {
        [&]( const nix::ExtendedOutputsSpec::Default & ) -> nix::OutputsSpec
        {
          std::set<std::string> outputsToInstall;
          auto metaOutputsToInstallCursor = cursor->findAlongAttrPath(
            nix::parseAttrPath( *state, "meta.outputsToInstall" ) );
          if ( metaOutputsToInstallCursor )
            {
              for ( auto output :
                    ( *metaOutputsToInstallCursor )->getListOfStrings() )
                {
                  outputsToInstall.insert( output );
                }
            }
          else if ( ! outputNames.empty() )
            {
              outputsToInstall.insert( outputNames.front() );
            }

          // this seems to be the default in a few nix places
          // could also dropt this, since reaching this point means
          // that the package has no outputs?!
          if ( outputsToInstall.empty() ) { outputsToInstall.insert( "out" ); }
          return nix::OutputsSpec::Names { std::move( outputsToInstall ) };
        },
        [&]( const nix::ExtendedOutputsSpec::Explicit & e ) -> nix::OutputsSpec
        { return e; },
      },
      extendedOutputsSpec.raw() );

    outputsToInstall = std::visit(
      overloaded {
        [&]( const nix::OutputsSpec::Names & n ) -> std::set<std::string>
        { return n; },
        [&]( const nix::OutputsSpec::All & ) -> std::set<std::string>
        {
          std::set<std::string> outputNamesSet;
          for ( auto output : outputNames ) { outputNamesSet.insert( output ); }
          return outputNamesSet;
        } },
      outputSpec.raw() );
  }

  std::string systemAttribute;
  {
    auto systemCursor
      = cursor->findAlongAttrPath( nix::parseAttrPath( *state, "system" ) );

    if ( ! systemCursor )
      {
        throw LockFlakeInstallableException(
          nix::fmt( "could not find '%s.%s' in derivation",
                    lockedAttrPath,
                    "system" ) );
      }
    systemAttribute = ( *systemCursor )->getString();
  }

  // Read `name` field - field is implied by the derivation
  std::string name;
  {
    auto nameCursor
      = cursor->findAlongAttrPath( nix::parseAttrPath( *state, "name" ) );

    if ( ! nameCursor )
      {
        throw LockFlakeInstallableException(
          nix::fmt( "could not find '%s.%s' in derivation",
                    lockedAttrPath,
                    "name" ) );
      }
    name = ( *nameCursor )->getString();
  }

  // Read `pname` field
  std::optional<std::string> pname;
  {
    auto pnameCursor
      = cursor->findAlongAttrPath( nix::parseAttrPath( *state, "pname" ) );

    if ( pnameCursor ) { pname = ( *pnameCursor )->getString(); }
  }

  // Read `version` field
  std::optional<std::string> version;
  {
    auto versionCursor
      = cursor->findAlongAttrPath( nix::parseAttrPath( *state, "version" ) );

    if ( versionCursor ) { version = ( *versionCursor )->getString(); }
  }

  // Read `meta.description` field
  std::optional<std::string> description;
  {
    auto descriptionCursor
      = cursor->findAlongAttrPath( { state->sMeta, state->sDescription } );

    if ( descriptionCursor )
      {
        description = ( *descriptionCursor )->getString();
      }
  }

  std::optional<std::string> license;
  {
    // todo
  }

  std::optional<bool> broken;
  {
    auto brokenCursor
      = cursor->findAlongAttrPath( nix::parseAttrPath( *state, "broken" ) );

    if ( brokenCursor ) { broken = ( *brokenCursor )->getBool(); }
  }

  std::optional<bool> unfree;
  {
    auto unfreeCursor
      = cursor->findAlongAttrPath( nix::parseAttrPath( *state, "unfree" ) );

    if ( unfreeCursor ) { unfree = ( *unfreeCursor )->getBool(); }
  }


  LockedInstallable lockedInstallable = {
    .lockedUrl        = lockedUrl,
    .flakeDescription = flakeDescription,
    .lockedAttrPath   = lockedAttrPath,
    .derivation       = derivation,
    .outputs          = outputs,
    .outputsToInstall = outputsToInstall,
    .packageSystem    = systemAttribute,
    .lockedSystem     = system,
    .name             = name,
    .pname            = pname,
    .version          = version,
    .description      = description,
    .license          = license,
    .broken           = broken,
    .unfree           = unfree,
  };

  return lockedInstallable;
}


void
to_json( nlohmann::json & jto, const LockedInstallable & from )
{
  jto = nlohmann::json {
    { "locked-url", from.lockedUrl },
    { "flake-description", from.flakeDescription },
    { "locked-attr-path", from.lockedAttrPath },
    { "derivation", from.derivation },
    { "outputs", from.outputs },
    { "outputs-to-install", from.outputsToInstall },
    { "package-system", from.packageSystem },
    { "locked-system", from.lockedSystem },
    { "name", from.name },
    { "pname", from.pname },
    { "version", from.version },
    { "description", from.description },
    { "license", from.license },
    { "broken", from.broken },
    { "unfree", from.unfree },
  };
}


/* -------------------------------------------------------------------------- */

}  // namespace flox


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
