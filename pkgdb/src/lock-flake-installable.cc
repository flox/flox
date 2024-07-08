/* ========================================================================== *
 *
 * @file eval.hh
 *
 * @brief Executable command helpers, argument parsers, etc.
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

  debugLog( nix::fmt( "original installable: %s", this->installable ) );


  std::tuple<nix::FlakeRef, std::string_view, nix::ExtendedOutputsSpec> parsed
    = nix::parseFlakeRefWithFragmentAndExtendedOutputsSpec( this->installable );

  nix::FlakeRef            flakeRef            = std::get<0>( parsed );
  std::string_view         fragment            = std::get<1>( parsed );
  nix::ExtendedOutputsSpec extendedOutputsSpec = std::get<2>( parsed );

  debugLog(
    nix::fmt( "original flakeRef: '%s'", flakeRef.to_string().c_str() ) );
  debugLog( nix::fmt( "original fragment: '%s'", fragment ) );
  debugLog( nix::fmt( "original extendedOutputsSpec: '%s'",
                      extendedOutputsSpec.to_string() ) );

  auto lockFlags = nix::flake::LockFlags {
    .recreateLockFile = false,
    .updateLockFile   = false,
    .writeLockFile    = false,
    .allowUnlocked    = true,
    .commitLockFile   = false,
  };


  auto installable = nix::make_ref<nix::InstallableFlake>(
    static_cast<nix::SourceExprCommand *>( nullptr ),
    state,
    std::move( flakeRef ),
    fragment,
    extendedOutputsSpec,
    nix::Strings {
      "packages." + this->system + ".default",
      "legacyPackages." + this->system + ".default",
    },
    nix::Strings {
      "packages." + this->system + ".",
      "legacyPackages." + this->system + ".",
    },
    lockFlags );
  debugLog(
    nix::fmt( "locked installable: '%s'", installable->what().c_str() ) );


  auto lockedUrl = installable->getLockedFlake()->flake.lockedRef.to_string();
  debugLog( nix::fmt( "locked url: '%s'", lockedUrl ) );

  auto flakeDescription = installable->getLockedFlake()->flake.description;

  auto cursor = installable->getCursor( *state );

  auto lockedAttrPath = cursor->getAttrPathStr();
  debugLog( nix::fmt( "locked attr path: '%s'", lockedAttrPath ) );

  debugLog( nix::fmt( "locked outputs: '%s'",
                      installable->extendedOutputsSpec.to_string() ) );

  // check if the output is a derivation (not a just a store path)
  if ( ! cursor->isDerivation() )
    {
      auto v = cursor->forceValue();
      throw nix::Error(
        "expected flake output attribute '%s' to be a derivation but found %s",
        lockedAttrPath,
        nix::showType( v ) );
    }

  // read the drv path
  std::string derivation;
  {
    auto derivationCursor
      = cursor->findAlongAttrPath( nix::parseAttrPath( *state, "drvPath" ) );
    if ( ! derivationCursor ) { return EXIT_FAILURE; }
    derivation = ( *derivationCursor )->getStringWithContext().first;
  }

  // map output names to their store paths
  std::map<std::string, std::string> outputs;
  std::vector<std::string>           outputNames;
  {
    auto maybe_outputs_cursor
      = cursor->findAlongAttrPath( nix::parseAttrPath( *state, "outputs" ) );
    if ( ! maybe_outputs_cursor ) { return EXIT_FAILURE; }
    outputNames = ( *maybe_outputs_cursor )->getListOfStrings();

    for ( auto output : outputNames )
      {
        auto outputCursor = cursor->findAlongAttrPath(
          nix::parseAttrPath( *state, output + ".outPath" ) );
        if ( ! outputCursor ) { return EXIT_FAILURE; }
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

  // Store the current system
  std::string system;
  {
    system = this->system;
  }


  // Read `name` field - field is impliend by the derivation
  std::string name;
  {
    auto nameCursor
      = cursor->findAlongAttrPath( nix::parseAttrPath( *state, "name" ) );

    if ( ! nameCursor ) { return EXIT_FAILURE; }
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
    .system           = system,
    .name             = name,
    .pname            = pname,
    .version          = version,
    .description      = description,
    .license          = license,
    .broken           = broken,
    .unfree           = unfree,
  };


  printf( "%s\n", nlohmann::json( lockedInstallable ).dump( 2 ).c_str() );

  return EXIT_SUCCESS;
};

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
    { "system", from.system },
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
