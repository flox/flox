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
#include "flox/fetchers/wrapped-nixpkgs-input.hh"
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
 * @brief Extract locked packages from the lockfile for the given system.
 * @throws @a SystemNotSupportedByLockfile exception if the lockfile does not
 *         specify packages for the given system.
 * @param lockfile Lockfile to extract packages from.
 * @param system System to extract packages for.
 * @return List of locked packages for the given system paired with their id.
 */
static std::vector<std::pair<std::string, resolver::LockedPackageRaw>>
getLockedPackages( resolver::Lockfile & lockfile, const System & system )
{
  traceLog( "creating FloxEnv" );
  auto packages = lockfile.getLockfileRaw().packages.find( system );
  if ( packages == lockfile.getLockfileRaw().packages.end() )
    {
      // Custom exception for non supported system
      throw SystemNotSupportedByLockfile(
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

  return locked_packages;
}

/* -------------------------------------------------------------------------- */

std::optional<nix::ref<nix::eval_cache::AttrCursor>>
maybeGetCursor( nix::ref<nix::EvalState> &              state,
                nix::ref<nix::eval_cache::AttrCursor> & cursor,
                const std::string &                     attr )
{
  debugLog(
    nix::fmt( "getting attr cursor '%s.%s", cursor->getAttrPathStr(), attr ) );
  auto symbol      = state->symbols.create( attr );
  auto maybeCursor = cursor->maybeGetAttr( symbol, true );
  if ( maybeCursor == nullptr ) { return std::nullopt; }
  auto newCursor
    = static_cast<nix::ref<nix::eval_cache::AttrCursor>>( maybeCursor );
  return newCursor;
}


/* -------------------------------------------------------------------------- */

nix::ref<nix::eval_cache::AttrCursor>
getPackageCursor( nix::ref<nix::EvalState> &      state,
                  const nix::flake::LockedFlake & flake,
                  const flox::AttrPath &          attrpath )
{
  auto evalCache
    = nix::openEvalCache( *state,
                          std::make_shared<nix::flake::LockedFlake>( flake ) );
  auto                     cursor = evalCache->getRoot();
  std::vector<std::string> seen;
  for ( auto attrName : attrpath )
    {

      if ( auto maybeCursor = maybeGetCursor( state, cursor, attrName );
           maybeCursor.has_value() )
        {
          cursor = *maybeCursor;
        }
      else
        {
          debugLog( "failed to get package cursor" );
          throw PackageEvalFailure(
            nix::fmt( "failed to evaluate attribute '%s.%s'",
                      cursor->getAttrPathStr(),
                      attrName ) );
        }
    }
  return cursor;
}


/* -------------------------------------------------------------------------- */

std::optional<std::string>
maybeGetStringAttr( nix::ref<nix::EvalState> &              state,
                    nix::ref<nix::eval_cache::AttrCursor> & cursor,
                    const std::string &                     attr )
{
  debugLog(
    nix::fmt( "getting string attr '%s.%s", cursor->getAttrPathStr(), attr ) );
  auto maybeCursor = maybeGetCursor( state, cursor, attr );
  if ( ! maybeCursor.has_value() ) { return std::nullopt; }
  auto str = ( *maybeCursor )->getString();
  return str;
}


/* -------------------------------------------------------------------------- */

std::optional<std::vector<std::string>>
maybeGetStringListAttr( nix::ref<nix::EvalState> &              state,
                        nix::ref<nix::eval_cache::AttrCursor> & cursor,
                        const std::string &                     attr )
{
  debugLog( nix::fmt( "getting string list attr '%s.%s",
                      cursor->getAttrPathStr(),
                      attr ) );
  auto maybeCursor = maybeGetCursor( state, cursor, attr );
  if ( ! maybeCursor.has_value() ) { return std::nullopt; }
  auto strs = ( *maybeCursor )->getListOfStrings();
  return strs;
}


/* -------------------------------------------------------------------------- */

std::optional<bool>
maybeGetBoolAttr( nix::ref<nix::EvalState> &              state,
                  nix::ref<nix::eval_cache::AttrCursor> & cursor,
                  const std::string &                     attr )
{
  debugLog(
    nix::fmt( "getting bool attr '%s.%s", cursor->getAttrPathStr(), attr ) );
  auto maybeCursor = maybeGetCursor( state, cursor, attr );
  if ( ! maybeCursor.has_value() ) { return std::nullopt; }
  auto b = ( *maybeCursor )->getBool();
  return b;
}


/* -------------------------------------------------------------------------- */

OutputsOrMissingOutput
getOutputsOutpaths( nix::ref<nix::EvalState> &              state,
                    nix::ref<nix::eval_cache::AttrCursor> & pkgCursor,
                    const std::vector<std::string> &        names )
{
  std::unordered_map<std::string, std::string> outpaths;
  for ( const auto & outputName : names )
    {
      debugLog( nix::fmt( "getting output attr '%s.%s",
                          pkgCursor->getAttrPathStr(),
                          outputName ) );


      // cursor to `<pkg>.${outputName}`
      auto maybeCursor = maybeGetCursor( state, pkgCursor, outputName );
      if ( ! maybeCursor.has_value() )
        {
          OutputsOrMissingOutput missing = outputName;
          return missing;
        }

      // cursor to `<pkg>.${outputName}.outPath`
      auto maybeStorePath
        = maybeGetStringAttr( state, *maybeCursor, "outPath" );

      if ( maybeStorePath == std::nullopt )
        {
          OutputsOrMissingOutput missing = outputName + ".outPath";
          return missing;
        }

      outpaths[outputName] = *maybeStorePath;
    }
  return outpaths;
}


/* -------------------------------------------------------------------------- */

std::string
tryEvaluatePackageOutPath( nix::ref<nix::EvalState> &              state,
                           const std::string &                     packageName,
                           const std::string &                     system,
                           nix::ref<nix::eval_cache::AttrCursor> & cursor )
{
  try
    {
      debugLog( nix::fmt( "trying to get outPath for '%s.outPath'",
                          cursor->getAttrPathStr() ) );

      auto result = maybeGetStringAttr( state, cursor, "outPath" );
      if ( result.has_value() ) { return *result; }
      throw PackageEvalFailure( "package '" + packageName
                                + "' had no outPath" );
    }
  catch ( const nix::Error & e )
    {
      /**
       * "not available on the requested hostPlatform"
       *   -> package isn't supported on this system
       */
      debugLog( "failed to get outPath: " + std::string( e.what() ) );
      if ( e.info().msg.str().find(
             "is not available on the requested hostPlatform:" )
           != std::string::npos )
        {
          debugLog( "'" + packageName + "' is not available on this system" );
          throw PackageUnsupportedSystem(
            nix::fmt( "package '%s' is not available for this system ('%s')",
                      packageName,
                      system ),

            nix::filterANSIEscapes( e.what(), true ) );
        }

      /**
       * eval errors are cached without the eror trace
       * force an impure eval to get the full error message
       */
      try
        {
          debugLog(
            "evaluating outPath uncached to get full error message" ) auto
            vPackage
            = cursor->forceValue();
          state->forceAttrs( vPackage, nix::noPos, "while evaluating package" );
          // expected to fail
          auto aOutPath
            = vPackage.attrs->get( state->symbols.create( "outPath" ) );
          state->forceString( *aOutPath->value,
                              aOutPath->pos,
                              "while evaluating outPath" );
          /**
           * this should only be reachable if we have a cached eval failure,
           * that evaluates successfully at a later time.
           * Since eval checks for nixpkgs are disabled through the
           * `flox-nixpkgs` fetcher which upon change will observe a different
           * fingerprint, i.e. fresh cache, this is rather unlikely.
           */
          debugLog( "evaluation was expected to fail, but was successful" );
          return aOutPath->value->string.s;
        }
      catch ( const nix::Error & e )
        {
          throw PackageEvalFailure(
            nix::fmt( "package '%s' failed to evaluate", packageName ),
            e.info().msg.str() );
        }
    }
}


/* -------------------------------------------------------------------------- */

nix::ref<nix::eval_cache::AttrCursor>
evalCacheCursorForInput( nix::ref<nix::EvalState> &             state,
                         const flox::resolver::LockedInputRaw & input,
                         const flox::AttrPath &                 attrPath )
{

  /**
   * Ensure the input is fetched with `flox-nixpkgs`.
   * Currently, the 'flox-nixpkgs' fetcher requires the original input to be
   * a rev or ref of `github:nixos/nixpkgs`.
   */
  auto floxNixpkgsAttrs = flox::githubAttrsToFloxNixpkgsAttrs( input.attrs );
  auto packageInputRef  = nix::FlakeRef::fromAttrs( floxNixpkgsAttrs );

  auto packageFlake
    = flox::lockFlake( *state, packageInputRef, nix::flake::LockFlags {} );

  auto cursor = getPackageCursor( state, packageFlake, attrPath );
  return cursor;
}


/* -------------------------------------------------------------------------- */

std::unordered_map<std::string, std::string>
outpathsForPackageOutputs( nix::ref<nix::EvalState> &              state,
                           const std::string &                     packageName,
                           nix::ref<nix::eval_cache::AttrCursor> & pkgCursor )
{
  debugLog( "getting outputs for " + packageName );

  // get `<pkg>.outputs`
  auto outputNames = maybeGetStringListAttr( state, pkgCursor, "outputs" );
  if ( ! ( outputNames.has_value() ) )
    {
      throw PackageEvalFailure(
        nix::fmt( "package '%s' had no outputs", packageName ) );
    }
  debugLog( nix::fmt( "found outputs [%s] for '%s'",
                      flox::concatStringsSep( ",", *outputNames ),
                      packageName ) );

  debugLog( "getting outPaths for outputs of " + packageName );

  auto maybeOutputsToOutpaths
    = getOutputsOutpaths( state, pkgCursor, *outputNames );

  if ( std::holds_alternative<std::string>( maybeOutputsToOutpaths ) )
    {
      auto missingOutput = std::get<std::string>( maybeOutputsToOutpaths );
      throw PackageEvalFailure( nix::fmt( "package '%s' had no output '%s'",
                                          packageName,
                                          missingOutput ) );
    }
  auto outputsToOutpaths
    = std::get<std::unordered_map<std::string, std::string>>(
      maybeOutputsToOutpaths );
  return outputsToOutpaths;
}


/* -------------------------------------------------------------------------- */

std::vector<std::pair<buildenv::RealisedPackage, nix::StorePath>>
collectRealisedPackages(
  nix::ref<nix::EvalState> &                     state,
  const std::string &                            packageName,
  const flox::resolver::LockedPackageRaw &       lockedPackage,
  const std::string &                            parentOutpath,
  std::unordered_map<std::string, std::string> & outputsToOutpaths )
{
  std::vector<std::pair<buildenv::RealisedPackage, nix::StorePath>> pkgs;
  auto internalPriority = 0;
  for ( const auto & [name, outpathStr] : outputsToOutpaths )
    {
      debugLog( "processing output '" + name + "' of '" + packageName + "'" );
      auto outpathForOutput = state->store->parseStorePath( outpathStr );
      buildenv::RealisedPackage pkg(
        state->store->printStorePath( outpathForOutput ),
        true,
        buildenv::Priority( lockedPackage.priority,
                            parentOutpath,
                            internalPriority++ ) );
      pkgs.emplace_back( pkg, outpathForOutput );
    }
  return pkgs;
}


/* -------------------------------------------------------------------------- */

std::vector<std::pair<buildenv::RealisedPackage, nix::StorePath>>
getRealisedPackages( nix::ref<nix::EvalState> &         state,
                     const std::string &                packageName,
                     const resolver::LockedPackageRaw & lockedPackage,
                     const System &                     system )
{
  auto timeEvalStart = std::chrono::high_resolution_clock::now();
  auto cursor        = evalCacheCursorForInput( state,
                                         lockedPackage.input,
                                         lockedPackage.attrPath );

  /* Try to eval the outPath. Trying this eval tells us whether the package is
   * unsupported. This eval will fail in a number of cases:
   * - The package doesn't work on this system
   * - The package is marked "insecure" i.e. it's old (e.g. Python 2)
   * - Possibly other cases as well
   * */

  // uses the cached value
  auto parentOutpath
    = tryEvaluatePackageOutPath( state, packageName, system, cursor );

  // auto parentOutpath
  // = tryEvalPath( state, packageName, system, cursor, isUnfree, "outPath" );

  /**
   * Collect the store paths for each output of the package.
   * Note that the "out" output is the same as the package's outPath.
   */
  auto outputsToOutpaths
    = outpathsForPackageOutputs( state, packageName, cursor );


  auto pkgs        = collectRealisedPackages( state,
                                       packageName,
                                       lockedPackage,
                                       parentOutpath,
                                       outputsToOutpaths );
  auto timeEvalEnd = std::chrono::high_resolution_clock::now();

  bool allValid = true;
  for ( const auto & [pkg, outPath] : pkgs )
    {
      try
        {
          state->store->ensurePath( outPath );
        }
      catch ( const nix::Error & e )
        {
          debugLog( "failed to ensure path: " + std::string( e.what() ) );
          allValid = false;
          break;  // no need to check the rest if any output is not
                  // substitutable
        }
    }

  // one or more outputs are not substitutable
  // we need to build the derivation to get all outputs
  if ( ! allValid )
    {
      auto drvPath = cursor->forceDerivation();
      try
        {
          auto storePathWithOutputs = nix::StorePathWithOutputs { drvPath, {} };
          state->store->buildPaths(
            nix::toDerivedPaths( { storePathWithOutputs } ) );
        }
      catch ( const nix::Error & e )
        {
          throw PackageBuildFailure( "Failed to build package '" + packageName
                                       + "'",
                                     nix::filterANSIEscapes( e.what(), true ) );
        }
    }


  auto timeBuildEnd = std::chrono::high_resolution_clock::now();

  /* Report some timings for diagnostics */
  auto timeEval = std::chrono::duration_cast<std::chrono::microseconds>(
    timeEvalEnd - timeEvalStart );
  auto timeBuild = std::chrono::duration_cast<std::chrono::microseconds>(
    timeBuildEnd - timeEvalEnd );
  auto timeTotal = timeEval + timeBuild;
  debugLog( nix::fmt( "times for package %s: eval=%dus, build=%dus, total=%dus",
                      packageName,
                      timeEval.count(),
                      timeBuild.count(),
                      timeTotal.count() ) );
  return pkgs;
}


/* -------------------------------------------------------------------------- */

/**
 * @brief Make a @a RealisedPackage and store path for the activation scripts.
 * The package contains the activation scripts for *bash* and *zsh*.
 * @param state Nix state.
 * @param lockfile Lockfile to extract environment variables and hook script
 * from.
 * @return A pair of the realised package and the store path of the activation
 * scripts.
 */
static std::pair<buildenv::RealisedPackage, nix::StorePathSet>
makeActivationScripts( nix::EvalState & state, resolver::Lockfile & lockfile )
{
  std::vector<nix::StorePath> activationScripts;
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

  RealisedPackage realised( state.store->printStorePath( activationStorePath ),
                            true,
                            buildenv::Priority() );
  auto            references = nix::StorePathSet();
  references.insert( activationStorePath );
  references.insert( state.store->parseStorePath( SET_PROMPT_BASH_SH ) );
  references.insert( state.store->parseStorePath( SET_PROMPT_ZSH_SH ) );


  return { realised, references };
}

/* -------------------------------------------------------------------------- */

/**
 * @brief Make a @a RealisedPackage and store path for the profile.d scripts.
 * @param state Nix state.
 * @return A pair of the realised package and the store path of the profile.d
 * scripts.
 */
static std::pair<buildenv::RealisedPackage, nix::StorePath>
makeProfileDScripts( nix::EvalState & state )
{
  /* Insert profile.d scripts.
   * The store path is provided at compile time via the
   * `PROFILE_D_SCRIPTS_DIR' environment variable. */
  auto profileScriptsPath
    = state.store->parseStorePath( PROFILE_D_SCRIPTS_DIR );
  state.store->ensurePath( profileScriptsPath );
  RealisedPackage realised( state.store->printStorePath( profileScriptsPath ),
                            true,
                            buildenv::Priority() );

  return { realised, profileScriptsPath };
}


/* -------------------------------------------------------------------------- */

/**
 * @brief Create a nix package for an environment definition.
 * @param state Nix state.
 * @param lockfile Lockfile to extract environment definition from.
 * @param system System to create the environment for.
 * @return The store path of the environment.
 */
nix::StorePath
createFloxEnv( nix::ref<nix::EvalState> & state,
               resolver::Lockfile &       lockfile,
               const System &             system )
{
  auto locked_packages = getLockedPackages( lockfile, system );

  /* Extract derivations */
  nix::StorePathSet            references;
  std::vector<RealisedPackage> pkgs;
  std::map<nix::StorePath, std::pair<std::string, resolver::LockedPackageRaw>>
    originalPackage;

  for ( auto const & [pId, package] : locked_packages )
    {
      auto realised = getRealisedPackages( state, pId, package, system );
      for ( auto [realisedPackage, output] : realised )
        {
          pkgs.push_back( realisedPackage );
          references.insert( output );
          originalPackage.insert( { output, { pId, package } } );
        }
    }

  // Add activation scripts to the environment
  auto [activationScriptPackage, activationScriptReferences]
    = makeActivationScripts( *state, lockfile );

  pkgs.push_back( activationScriptPackage );
  references.insert( activationScriptReferences.begin(),
                     activationScriptReferences.end() );


  auto [profileScriptsPath, profileScriptsReference]
    = makeProfileDScripts( *state );

  pkgs.push_back( profileScriptsPath );
  references.insert( profileScriptsReference );

  return createEnvironmentStorePath( *state,
                                     pkgs,
                                     references,
                                     originalPackage );
}


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
