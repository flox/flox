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

LockFlakeInstallableCommand::LockFlakeInstallableCommand()
  : parser( "lock-flake-installable" )
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
LockFlakeInstallableCommand::run()
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
static flox::Cursor
getDerivationCursor( const nix::ref<nix::EvalState> & state,
                     nix::InstallableFlake &          installable )
{
  try
    {
      auto cursor = installable.getCursor( *state );
      return cursor;
    }
  catch ( const nix::Error & e )
    {
      throw LockFlakeInstallableException( "could not find installable",
                                           e.info().msg.str() );
    }
}

/**
 * @brief Read a license string or id from a nix value.
 * @note The license can be either a string or an attribute set with a `spdxId`
 * if `<nixpkgs>.lib.licenses.<license>` is used.
 * @param state The nix evaluation state
 * @param licenseValue The value to read the license from
 * @return The license string or id if found or `std::nullopt` otherwise
 */
static std::optional<std::string>
readLicenseStringOrId( const nix::ref<nix::EvalState> & state,
                       nix::Value *                     licenseValue )
{
  if ( licenseValue->type() == nix::ValueType::nString )
    {
      return std::string( licenseValue->str() );
    }
  else if ( licenseValue->type() == nix::ValueType::nAttrs )
    {
      auto licenseIdValue
        = licenseValue->attrs->find( state->symbols.create( "spdxId" ) );

      if ( licenseIdValue != licenseValue->attrs->end()
           && licenseIdValue->value->type() == nix::ValueType::nString )
        {
          return std::string( licenseIdValue->value->str() );
        };
    }

  return std::nullopt;
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

  nix::InstallableFlake installable = nix::InstallableFlake(
    // The `cmd` argument is only used in nix to raise an error
    // if `--arg` was used in the same command.
    // The argument is never stored on the `InstallableFlake` struct
    // or referenced outside of the constructor.
    // We can safely pass a nullptr here, as the constructor performs a null
    // check before dereferencing the arguement:
    // <https://github.com/NixOS/nix/blob/509be0e77aacd8afcf419526620994cbbbe3708a/src/libcmd/installable-flake.cc#L86-L87>
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

  debugLog(
    nix::fmt( "locked installable: '%s'", installable.what().c_str() ) );


  auto lockedUrl = installable.getLockedFlake()->flake.lockedRef.to_string();
  debugLog( nix::fmt( "locked url: '%s'", lockedUrl ) );

  auto flakeDescription = installable.getLockedFlake()->flake.description;

  auto cursor = getDerivationCursor( state, installable );

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

  // try read `meta.outputsToInstall` field
  std::optional<std::set<std::string>> outputsToInstall;
  {
    std::set<std::string> outputsToInstallFound;
    auto metaOutputsToInstallCursor = cursor->findAlongAttrPath(
      nix::parseAttrPath( *state, "meta.outputsToInstall" ) );
    if ( metaOutputsToInstallCursor )
      {
        for ( auto output :
              ( *metaOutputsToInstallCursor )->getListOfStrings() )
          {
            outputsToInstallFound.insert( output );
          }

        outputsToInstall = outputsToInstallFound;
      }
  }

  // the requested outputs to install by means of the extended outputs spec
  // i.e. `#^<outputs>` in the flake installable
  std::optional<std::set<std::string>> requestedOutputs;
  {
    requestedOutputs = std::visit(
      overloaded {
        [&]( const nix::ExtendedOutputsSpec::Default & )
          -> std::optional<std::set<std::string>> { return std::nullopt; },
        [&]( const nix::ExtendedOutputsSpec::Explicit & e )
          -> std::optional<std::set<std::string>>
        {
          return std::visit(
            overloaded {
              [&]( const nix::OutputsSpec::Names & n ) -> std::set<std::string>
              { return n; },
              [&]( const nix::OutputsSpec::All & ) -> std::set<std::string>
              {
                std::set<std::string> outputNamesSet;
                for ( auto output : outputNames )
                  {
                    outputNamesSet.insert( output );
                  }
                return outputNamesSet;
              } },
            e.raw() );
        },
      },
      extendedOutputsSpec.raw() );
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

  std::optional<std::vector<std::string>> licenses;
  {
    auto licenseCursor = cursor->findAlongAttrPath(
      nix::parseAttrPath( *state, "meta.license" ) );

    if ( licenseCursor )
      {
        auto licenseValue = ( *licenseCursor )->forceValue();
        std::vector<std::string> licenseStrings;
        if ( licenseValue.isList() )
          {
            for ( auto licenseValueInner : licenseValue.listItems() )
              {
                state->forceValueDeep( *licenseValueInner );
                if ( auto licenseString = readLicenseStringOrId( state,licenseValueInner ) )
                  {
                    licenseStrings.push_back( *licenseString );
                  }
              }
          }
        else if ( auto licenseString
                  = readLicenseStringOrId( state, &licenseValue ) )
          {
            licenseStrings.push_back( *licenseString );
          }
        if ( ! licenseStrings.empty() ) { licenses = licenseStrings; }
      }
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
    .lockedUrl                 = lockedUrl,
    .flakeDescription          = flakeDescription,
    .lockedFlakeAttrPath       = lockedAttrPath,
    .derivation                = derivation,
    .outputs                   = outputs,
    .outputNames               = outputNames,
    .outputsToInstall          = outputsToInstall,
    .requestedOutputsToInstall = requestedOutputs,
    .packageSystem             = systemAttribute,
    .lockedSystem              = system,
    .name                      = name,
    .pname                     = pname,
    .version                   = version,
    .description               = description,
    .licenses                  = licenses,
    .broken                    = broken,
    .unfree                    = unfree,
  };

  return lockedInstallable;
}


void
to_json( nlohmann::json & jto, const LockedInstallable & from )
{
  jto = nlohmann::json {
    { "locked-url", from.lockedUrl },
    { "flake-description", from.flakeDescription },
    { "locked-flake-attr-path", from.lockedFlakeAttrPath },
    { "derivation", from.derivation },
    { "outputs", from.outputs },
    { "output-names", from.outputNames },
    { "outputs-to-install", from.outputsToInstall },
    { "requested-outputs-to-install", from.requestedOutputsToInstall },
    { "package-system", from.packageSystem },
    { "locked-system", from.lockedSystem },
    { "name", from.name },
    { "pname", from.pname },
    { "version", from.version },
    { "description", from.description },
    { "licenses", from.licenses },
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
