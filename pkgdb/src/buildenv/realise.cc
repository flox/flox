/* ========================================================================== *
 *
 * @file buildenv/realise.cc
 *
 * @brief Evaluate an environment definition and realise it.
 *
 *
 * -------------------------------------------------------------------------- */

#include <filesystem>
#include <fstream>

#include <nix/command.hh>
#include <nix/derivations.hh>
#include <nix/derived-path.hh>
#include <nix/eval-cache.hh>
#include <nix/eval-inline.hh>
#include <nix/eval.hh>
#include <nix/flake/flake.hh>
#include <nix/get-drvs.hh>
#include <nix/globals.hh>
#include <nix/installable-flake.hh>
#include <nix/local-fs-store.hh>
#include <nix/path-with-outputs.hh>
#include <nix/profiles.hh>
#include <nix/shared.hh>
#include <nix/store-api.hh>
#include <nix/util.hh>
#include <nlohmann/json.hpp>

#include "flox/buildenv/realise.hh"
#include "flox/resolver/lockfile.hh"


/* -------------------------------------------------------------------------- */

namespace flox::buildenv {

/* -------------------------------------------------------------------------- */

#ifndef PROFILE_D_SCRIPTS_DIR
#  error "PROFILE_D_SCRIPTS_DIR must be set to the path of `etc/profile.d/'"
#endif

#ifndef SET_PROMPT_BASH_SH
#  error "SET_PROMPT_BASH_SH must be set to the path of `set-prompt.bash.sh'"
#endif

#ifndef SET_PROMPT_ZSH_SH
#  error "SET_PROMPT_ZSH_SH must be set to the path of `set-prompt.zsh.sh'"
#endif

#ifndef CONTAINER_BUILDER_PATH
#  error \
    "CONTAINER_BUILDER_PATH must be set to a store path of 'mkContainer.nix'"
#endif

#ifndef COMMON_NIXPKGS_URL
#  error "COMMON_NIXPKGS_URL must be set to a locked flakeref of nixpkgs to use"
#endif

/* -------------------------------------------------------------------------- */

const char * const BASH_ACTIVATE_SCRIPT = R"(
# We use --rcfile to activate using bash which skips sourcing ~/.bashrc,
# so source that here.
if [ -f ~/.bashrc -a "${FLOX_SOURCED_FROM_SHELL_RC:-}" != 1 ]
then
    source ~/.bashrc
fi

if [ -d "$FLOX_ENV/etc/profile.d" ]; then
  declare -a _prof_scripts;
  _prof_scripts=( $(
    shopt -s nullglob;
    echo "$FLOX_ENV/etc/profile.d"/*.sh;
  ) );
  for p in "${_prof_scripts[@]}"; do . "$p"; done
  unset _prof_scripts;
fi
)";


// unlike bash, zsh activation calls this script from the user's shell rcfile
const char * const ZSH_ACTIVATE_SCRIPT = R"(
if [ -d "$FLOX_ENV/etc/profile.d" ]; then
  declare -a _prof_scripts;
  _prof_scripts=( $(
    echo "$FLOX_ENV/etc/profile.d"/*.sh;
  ) );
  for p in "${_prof_scripts[@]}"; do . "$p"; done
  unset _prof_scripts;
fi
)";


/* -------------------------------------------------------------------------- */

static nix::StorePath
addDirToStore( nix::EvalState &    state,
               std::string const & dir,
               nix::StorePathSet & references )
{

  /* Add the symlink tree to the store. */
  nix::StringSink sink;
  dumpPath( dir, sink );

  auto narHash = hashString( nix::htSHA256, sink.s );
  nix::ValidPathInfo info {
            *state.store,
            "environment",
            nix::FixedOutputInfo {
                .method = nix::FileIngestionMethod::Recursive,
                .hash = narHash,
                .references = {
                    .others = std::move(references),
                    // profiles never refer to themselves
                    .self = false,
                },
            },
            narHash,
        };
  info.narSize = sink.s.size();

  nix::StringSource source( sink.s );
  state.store->addToStore( info, source );
  return std::move( info.path );
}


/* -------------------------------------------------------------------------- */

nix::StorePath
createEnvironmentStorePath(
  nix::EvalState &               state,
  std::vector<RealisedPackage> & pkgs,
  nix::StorePathSet &            references,
  std::map<nix::StorePath, std::pair<std::string, resolver::LockedPackageRaw>> &
    originalPackage )
{
  /* build the profile into a tempdir */
  auto tempDir = nix::createTempDir();
  try
    {
      buildenv::buildEnvironment( tempDir, pkgs );
    }
  catch ( buildenv::FileConflict & err )
    {
      auto [storePathA, filePath] = state.store->toStorePath( err.fileA );
      auto [storePathB, _]        = state.store->toStorePath( err.fileB );

      auto [nameA, packageA] = originalPackage.at( storePathA );
      auto [nameB, packageB] = originalPackage.at( storePathB );


      throw PackageConflictException( nix::fmt(
        "'%s' conflicts with '%s'. Both packages provide the file '%s'"
        "\n\nResolve by uninstalling one of the conflicting packages"
        "or setting the priority of the preferred package to a value lower "
        "than '%d'",
        nameA,
        nameB,
        filePath,
        err.priority ) );
    }
  return addDirToStore( state, tempDir, references );
}

/* -------------------------------------------------------------------------- */

/**
 * @brief Extract a nix value from a (flake) curor.
 * @param cursor Cursor to extract the value from.
 * @param attrPath an attribute path to a package in the flake.
 * @return The value referenced by @a attrPath.
 * @throws PackageEvalFailure if the attribute is not found.
 */
static nix::Value
extractAttrPath( nix::EvalState &              state,
                 nix::eval_cache::AttrCursor & cFlake,
                 const flox::AttrPath &        attrPath )
{
  std::vector<nix::Symbol> attrPathNix {};
  for ( auto attrName : attrPath )
    {
      attrPathNix.push_back( state.symbols.create( attrName ) );
    }

  auto maybeValue = cFlake.findAlongAttrPath( attrPathNix );
  if ( ! maybeValue )
    {
      throw PackageEvalFailure(
        nix::fmt( "Failed to find package '%s'", cFlake.getAttrPathStr() ) );
    }

  auto output = ( *maybeValue )->forceValue();

  return output;
}


/* -------------------------------------------------------------------------- */
// TODO: this function is too long, break it up
// NOLINTBEGIN(readability-function-cognitive-complexity)
nix::StorePath
createFloxEnv( nix::EvalState &     state,
               resolver::Lockfile & lockfile,
               const System &       system )
{
  auto packages = lockfile.getLockfileRaw().packages.find( system );
  if ( packages == lockfile.getLockfileRaw().packages.end() )
    {
      // TODO: throw structured exception
      throw SystenNotSupportedByLockfile(
        "'" + system + "' not supported by this environment" );
    }

  /* Extract all packages */
  std::vector<std::pair<std::string, resolver::LockedPackageRaw>>
    locked_packages;

  for ( auto const & package : packages->second )
    {
      if ( ! package.second.has_value() ) { continue; }
      auto const & locked_package = package.second.value();
      locked_packages.emplace_back( package.first, locked_package );
    }

  /* Extract derivations */
  nix::StorePathSet                      references;
  std::vector<nix::StorePathWithOutputs> drvsToBuild;
  std::vector<RealisedPackage>           pkgs;
  std::map<nix::StorePath, std::pair<std::string, resolver::LockedPackageRaw>>
    originalPackage;

  for ( auto const & [pId, package] : locked_packages )
    {
      // TODO: use `FloxFlake'
      auto packageInputRef = nix::FlakeRef( package.input );
      auto packageFlake    = nix::flake::lockFlake( state,
                                                 packageInputRef,
                                                 nix::flake::LockFlags {} );

      auto evalCache = nix::openEvalCache(
        state,
        std::make_shared<nix::flake::LockedFlake>( packageFlake ) );

      nix::ref<nix::eval_cache::AttrCursor> cFlake = evalCache->getRoot();

      /* Get referenced output. */
      auto vFlake = extractAttrPath( state, *cFlake, package.attrPath );

      /* Interpret output as derivation. */
      auto package_drv = getDerivation( state, vFlake, false );

      if ( ! package_drv.has_value() )
        {
          throw PackageEvalFailure( "Failed to get derivation for package '"
                                    + nlohmann::json( package ).dump() + "'" );
        }

      std::string packagePath;
      try
        {
          packagePath
            = state.store->printStorePath( package_drv->queryOutPath() );
        }
      catch ( const nix::Error & e )
        {

          if ( e.info().msg.str().find(
                 "is not available on the requested hostPlatform:" )
               != std::string::npos )
            {
              throw PackageUnsupportedSystem(
                nix::fmt(
                  "package '%s' is not available for this system ('%s')",
                  pId,
                  system ),

                nix::filterANSIEscapes( e.what(), true ) );
            }

          // rethrow the original root cause without the nix trace
          throw PackageEvalFailure(
            nix::fmt( "package '%s' failed to evaluate", pId ),
            e.info().msg.str() );
        };


      /* Collect all outputs to include in the environment.
       *
       * Set the priority of the outputs to the priority of the package
       * and the internal priority to the index of the output.
       * This way `buildenv::buildEnvironment` can resolve conflicts between
       * outputs of the same derivation. */
      for ( auto [idx, output] : enumerate( package_drv->queryOutputs() ) )
        {
          /* Skip outputs without path */
          if ( ! output.second.has_value() ) { continue; }
          pkgs.emplace_back(
            state.store->printStorePath( output.second.value() ),
            true,
            buildenv::Priority( package.priority,
                                packagePath,
                                /* idx should always fit in uint its unlikely a
                                 * package has more than 4 billion outputs. */
                                static_cast<unsigned>( idx ) ) );
          references.insert( output.second.value() );
          originalPackage.insert( { output.second.value(), { pId, package } } );
        }

      /* Collect drvs that may yet need to be built. */
      if ( auto drvPath = package_drv->queryDrvPath() )
        {
          /* Build derivation of pacakge in environment,
           * rethrow errors as PackageBuildFailure. */
          try
            {
              auto storePathWithOutputs
                = nix::StorePathWithOutputs { *drvPath, {} };
              state.store->buildPaths(
                nix::toDerivedPaths( { storePathWithOutputs } ) );
            }
          catch ( const nix::Error & e )
            {
              throw PackageBuildFailure(
                "Failed to build package '" + pId + "'",
                nix::filterANSIEscapes( e.what(), true ) );
            }
        }
    }

  /* verbatim content of the activate script common to all shells */
  std::stringstream commonActivate;

  auto tempDir = std::filesystem::path( nix::createTempDir() );
  std::filesystem::create_directories( tempDir / "activate" );

  /* Add environment variables.
   * Read environment variables from the manifest and add them as exports to the
   * activate script. */
  if ( auto vars = lockfile.getManifest().getManifestRaw().vars )
    {

      for ( auto [name, value] : vars.value() )
        {
          /* Single quote value and replace ' with '\''.
           *
           * This is the same as what nixpkgs.lib.escapeShellArg does.
           * to disable these variables dynamically expanding at runtime.
           *
           * 'foo''\\''bar' is evaluated as  foo'bar  in bash/zsh*/
          size_t indexOfQuoteChar = 0;
          while ( ( indexOfQuoteChar = value.find( '\'', indexOfQuoteChar ) )
                  != std::string::npos )
            {
              value.replace( indexOfQuoteChar, 1, "'\\''" );
              indexOfQuoteChar += 4;
            }

          commonActivate << nix::fmt( "export %s='%s'\n", name, value );
        }
    }

  /* Add hook script.
   * Write hook script to a temporary file and copy it to the environment.
   * Add source command to the activate script. */
  // TODO: is it script _xor_ file?
  // Currently it is assumed that `hook.script` and `hook.file` are
  // mutually exclusive.
  // If both are set, `hook.file` takes precedence.
  if ( auto hook = lockfile.getManifest().getManifestRaw().hook )
    {
      nix::Path script_path;

      /* Either set script path to a temporary file. */
      if ( auto script = hook->script )
        {
          script_path = nix::createTempFile().second;
          std::ofstream file( script_path );
          file << script.value();
          file.close();
        }

      /* ...Or to the file specified in the manifest. */
      if ( auto file = hook->file ) { script_path = file.value(); }

      if ( ! script_path.empty() )
        {

          std::filesystem::copy_file( script_path,
                                      tempDir / "activate" / "hook.sh" );
          std::filesystem::permissions( tempDir / "activate" / "hook.sh",
                                        std::filesystem::perms::owner_exec,
                                        std::filesystem::perm_options::add );
          commonActivate << "source \"$FLOX_ENV/activate/hook.sh\"" << '\n';
        }
    }

  /* Add bash activation script. */
  std::ofstream bashActivate( tempDir / "activate" / "bash" );
  /* If this gets bigger, we could factor this out into a file that gets
   * sourced, like we do for zsh. */
  bashActivate << BASH_ACTIVATE_SCRIPT << "\n";
  bashActivate << "source " << SET_PROMPT_BASH_SH << "\n";
  bashActivate << commonActivate.str();
  bashActivate.close();

  /* Add zsh activation script.
   * Functionality shared between all environments is
   * in `flox.zdotdir/.zshrc'. */
  std::ofstream zshActivate( tempDir / "activate" / "zsh" );
  zshActivate << ZSH_ACTIVATE_SCRIPT << "\n";
  zshActivate << "source " << SET_PROMPT_ZSH_SH << "\n";
  zshActivate << commonActivate.str();
  zshActivate.close();

  auto activationStorePath
    = state.store->addToStore( "activation-scripts", tempDir );
  references.insert( activationStorePath );
  pkgs.emplace_back( state.store->printStorePath( activationStorePath ),
                     true,
                     buildenv::Priority() );

  references.insert( state.store->parseStorePath( SET_PROMPT_BASH_SH ) );
  references.insert( state.store->parseStorePath( SET_PROMPT_ZSH_SH ) );

  /* Insert profile.d scripts.
   * The store path is provided at compile time via the `PROFILE_D_SCRIPTS_DIR'
   * environment variable. */
  auto profileScriptsPath
    = state.store->parseStorePath( PROFILE_D_SCRIPTS_DIR );
  state.store->ensurePath( profileScriptsPath );
  references.insert( profileScriptsPath );
  pkgs.emplace_back( state.store->printStorePath( profileScriptsPath ),
                     true,
                     buildenv::Priority() );

  return createEnvironmentStorePath( state, pkgs, references, originalPackage );
}
// NOLINTEND(readability-function-cognitive-complexity)


nix::StorePath
createContainerBuilder( nix::EvalState &       state,
                        const nix::StorePath & environmentStorePath,
                        const System &         system )
{
  static const nix::FlakeRef nixpkgsRef
    = nix::parseFlakeRef( COMMON_NIXPKGS_URL );

  auto lockedNixpkgs
    = flox::lockFlake( state, nixpkgsRef, nix::flake::LockFlags() );

  nix::Value vNixpkgsFlake {};
  nix::flake::callFlake( state, lockedNixpkgs, vNixpkgsFlake );

  state.store->ensurePath(
    state.store->parseStorePath( CONTAINER_BUILDER_PATH ) );

  nix::Value vContainerBuilder {};
  state.eval(
    state.parseExprFromFile( nix::CanonPath( CONTAINER_BUILDER_PATH ) ),
    vContainerBuilder );

  nix::Value vEnvironmentStorePath {};
  auto       sStorePath = state.store->printStorePath( environmentStorePath );
  vEnvironmentStorePath.mkPath( sStorePath.c_str() );

  nix::Value vSystem {};
  vSystem.mkString( nix::nativeSystem );

  nix::Value vContainerSystem {};
  vContainerSystem.mkString( system );

  nix::Value vBindings {};
  auto       bindings = state.buildBindings( 4 );
  bindings.push_back(
    { state.symbols.create( "nixpkgsFlake" ), &vNixpkgsFlake } );
  bindings.push_back(
    { state.symbols.create( "environmentOutPath" ), &vEnvironmentStorePath } );
  bindings.push_back( { state.symbols.create( "system" ), &vSystem } );
  bindings.push_back(
    { state.symbols.create( "containerSystem" ), &vContainerSystem } );

  vBindings.mkAttrs( bindings );

  nix::Value vContainerBuilderDrv {};
  state.callFunction( vContainerBuilder,
                      vBindings,
                      vContainerBuilderDrv,
                      nix::PosIdx() );

  // force the derivation value to be evaluated
  // this enforces that the nix expression in pure up to the derivation
  // (see below)
  state.forceValue( vContainerBuilderDrv, nix::noPos );

  auto containerBuilderDrv
    = nix::getDerivation( state, vContainerBuilderDrv, false ).value();


  // building of the container builder derivation requires impure evaluation


  // Access to absolute paths is restricted by default.
  // Instead of disabling restricted evaluation,
  // we allow access to the bundled store path explictly.
  state.allowPath( environmentStorePath );

  // the derivation uses `builtins.storePath`
  // to ensure that all store references of the enfironment
  // are included in the derivation/container.
  //
  // `builtins.storePath` however requires impure evaluation
  // since input addressed store paths are not guaranteed to be pure or
  // present in the store in the first place.
  // In this case, we know that the environment is already built.
  //
  //
  auto pureEvalState = nix::evalSettings.pureEval.get();
  nix::evalSettings.pureEval.override( false );

  state.store->buildPaths( nix::toDerivedPaths(
    { nix::StorePathWithOutputs { *containerBuilderDrv.queryDrvPath(),
                                  {} } } ) );


  auto outPath = containerBuilderDrv.queryOutPath();

  // be nice, reset the original pure eval state
  nix::evalSettings.pureEval = pureEvalState;

  return outPath;
}

/* -------------------------------------------------------------------------- */

}  // namespace flox::buildenv


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
