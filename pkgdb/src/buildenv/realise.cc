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

#ifndef ACTIVATION_SCRIPTS_PACKAGE_DIR
#  error "ACTIVATION_SCRIPTS_PACKAGE_DIR must be set"
#endif

#ifndef CONTAINER_BUILDER_PATH
#  error \
    "CONTAINER_BUILDER_PATH must be set to a store path of 'mkContainer.nix'"
#endif

#ifndef COMMON_NIXPKGS_URL
#  error "COMMON_NIXPKGS_URL must be set to a locked flakeref of nixpkgs to use"
#endif

#ifndef FLOX_BASH_PKG
#  error "FLOX_BASH_PKG must be set to the path of the nix bash package"
#endif

#ifndef FLOX_CACERT_PKG
#  error "FLOX_CACERT_PKG must be set to the path of the nixpkgs cacert package"
#endif

#ifdef linux
#  ifndef FLOX_LOCALE_ARCHIVE
#    error "FLOX_LOCALE_ARCHIVE_PKG must be set to the LOCALE_ARCHIVE variable"
#  endif
#else  // darwin
#  ifndef FLOX_PATH_LOCALE
#    error "FLOX_PATH_LOCALE_PKG must be set to the PATH_LOCALE variable"
#  endif
#  ifndef FLOX_NIX_COREFOUNDATION_RPATH
#    error \
      "FLOX_NIX_COREFOUNDATION_RPATH must be set to the NIX_COREFOUNDATION_RPATH variable"
#  endif
#endif

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
  nix::EvalState &                        state,
  std::vector<RealisedPackage> &          pkgs,
  nix::StorePathSet &                     references,
  std::map<nix::StorePath, std::string> & storePathsToInstallIds,
  const std::optional<std::string> &      serviceConfigPath )
{
  /* build the profile into a tempdir */
  auto tempDir = nix::createTempDir();
  if ( serviceConfigPath.has_value() )
    {
      try
        {
          std::filesystem::copy_file( *serviceConfigPath,
                                      std::filesystem::path( tempDir )
                                        / "service-config.yaml",
                                      std::filesystem::copy_options::none );
        }
      catch ( const std::filesystem::filesystem_error & e )
        {
          throw PackageBuildFailure(
            "Failed to add service config to environment",
            nix::filterANSIEscapes( e.what(), true ) );
        }
    }
  try
    {
      buildenv::buildEnvironment( tempDir, pkgs );
    }
  catch ( buildenv::FileConflict & err )
    {
      auto [storePathA, filePath] = state.store->toStorePath( err.fileA );
      auto [storePathB, _]        = state.store->toStorePath( err.fileB );

      auto nameA = storePathsToInstallIds.at( storePathA );
      auto nameB = storePathsToInstallIds.at( storePathB );


      throw PackageConflictException( nix::fmt(
        "'%s' conflicts with '%s'. Both packages provide the file '%s'"
        "\n\nResolve by uninstalling one of the conflicting packages "
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
  for ( const auto & attrName : attrpath )
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
  auto boolAttr = ( *maybeCursor )->getBool();
  return boolAttr;
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
          auto * aOutPath
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
  auto packageInputRef = nix::FlakeRef::fromAttrs( input.attrs );

  auto packageFlake = nix::flake::lockFlake( *state,
                                             packageInputRef,
                                             nix::flake::LockFlags {} );

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
collectRealisedOutputs(
  nix::ref<nix::EvalState> &                     state,
  const BuildenvLockedPackage &                  lockedPackage,
  const std::string &                            parentOutpath,
  std::unordered_map<std::string, std::string> & outputsToOutpaths )
{
  std::vector<std::pair<buildenv::RealisedPackage, nix::StorePath>> pkgs;
  auto internalPriority = 0;
  for ( const auto & [name, outpathStr] : outputsToOutpaths )
    {
      debugLog( nix::fmt( "processing output '%s' of '%s'",
                          name,
                          lockedPackage.installId ) );
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
getRealisedOutputs( nix::ref<nix::EvalState> &    state,
                    const BuildenvLockedPackage & lockedPackage,
                    const System &                system )
{
  debugLog( nix::fmt( "getting cursor for %s", lockedPackage.attrPath[0] ) );
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
  auto parentOutpath = tryEvaluatePackageOutPath( state,
                                                  lockedPackage.installId,
                                                  system,
                                                  cursor );

  // auto parentOutpath
  // = tryEvalPath( state, packageName, system, cursor, isUnfree, "outPath" );

  /**
   * Collect the store paths for each output of the package.
   * Note that the "out" output is the same as the package's outPath.
   */
  auto outputsToOutpaths
    = outpathsForPackageOutputs( state, lockedPackage.installId, cursor );


  auto pkgs        = collectRealisedOutputs( state,
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
          throw PackageBuildFailure( "Failed to build package '"
                                       + lockedPackage.installId + "'",
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
                      lockedPackage.installId,
                      timeEval.count(),
                      timeBuild.count(),
                      timeTotal.count() ) );
  return pkgs;
}


/* -------------------------------------------------------------------------- */

void
addScriptToScriptsDir( const std::string &           scriptContents,
                       const std::filesystem::path & scriptsDir,
                       const std::string &           scriptName )
{
  /* Ensure that the `scriptsDir` subdirectory exists. */
  std::filesystem::create_directories( scriptsDir );

  /* Write the script to a temporary file. */
  std::filesystem::path scriptTempPath( nix::createTempFile().second );
  debugLog(
    nix::fmt( "created tempfile for activation script: script=%s, path=%s",
              scriptName,
              scriptTempPath ) );
  std::ofstream scriptTmpFile( scriptTempPath );
  if ( ! scriptTmpFile.is_open() )
    {
      throw ActivationScriptBuildFailure( std::string( strerror( errno ) ) );
    }
  scriptTmpFile << scriptContents;
  if ( scriptTmpFile.fail() )
    {
      throw ActivationScriptBuildFailure( std::string( strerror( errno ) ) );
    }
  scriptTmpFile.close();

  /* Copy the script to the scripts directory. */
  auto scriptPath = scriptsDir / scriptName;
  debugLog( nix::fmt( "copying script to scripts dir: src=%s, dest=%s",
                      scriptTempPath,
                      scriptPath ) );
  std::filesystem::copy_file( scriptTempPath, scriptPath );
}

std::string
activationScriptEnvironmentPath( const std::string & scriptName )
{
  return nix::fmt( "\"$FLOX_ENV/%s/%s\"", ACTIVATION_SUBDIR_NAME, scriptName );
}

/* -------------------------------------------------------------------------- */

std::pair<buildenv::RealisedPackage, nix::StorePathSet>
makeActivationScripts( nix::EvalState &         state,
                       const BuildenvLockfile & lockfile )
{
  std::vector<nix::StorePath> activationScripts;
  auto tempDir            = std::filesystem::path( nix::createTempDir() );
  auto activateScriptsDir = tempDir / ACTIVATION_SUBDIR_NAME;
  std::filesystem::create_directories( activateScriptsDir );

  /* Create the shell-specific activation scripts */
  std::stringstream envrcScript;

  auto manifest = lockfile.manifest;

  /* Add environment variables. */
  if ( auto vars = manifest.vars )
    {
      // XXX Really need to find better way to master these variables.
      envrcScript << "# Default environment variables\n"
                  << defaultValue( "SSL_CERT_FILE",
                                   FLOX_CACERT_PKG
                                     << "/etc/ssl/certs/ca-bundle.crt" )
                  << defaultValue( "NIX_SSL_CERT_FILE", "${SSL_CERT_FILE}" )
#ifdef __linux__
                  << defaultValue( "LOCALE_ARCHIVE", FLOX_LOCALE_ARCHIVE )
#else
                  << defaultValue( "NIX_COREFOUNDATION_RPATH",
                                   FLOX_NIX_COREFOUNDATION_RPATH )
                  << defaultValue( "PATH_LOCALE", FLOX_PATH_LOCALE )
#endif
                  << "# Static environment variables" << std::endl;

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
          envrcScript << nix::fmt( "export %s='%s'\n", name, value );
        }
    }

  /* Add envrc script */
  if ( envrcScript.str().size() > 0 )
    {
      debugLog( "adding 'envrc' to activation scripts" );
      addScriptToScriptsDir( envrcScript.str(), activateScriptsDir, "envrc" );
    }

  /* Append profile script invocations in the middle */
  auto profile = manifest.profile;
  if ( profile.has_value() )
    {
      if ( profile->common.has_value() )
        {
          debugLog( "adding 'profile.common' to activation scripts" );
          addScriptToScriptsDir( *profile->common,
                                 activateScriptsDir,
                                 "profile-common" );
        }
      if ( profile->bash.has_value() )
        {
          debugLog( "adding 'profile.bash' to activation scripts" );
          addScriptToScriptsDir( *profile->bash,
                                 activateScriptsDir,
                                 "profile-bash" );
        }
      if ( profile->fish.has_value() )
        {
          debugLog( "adding 'profile.fish' to activation scripts" );
          addScriptToScriptsDir( *profile->fish,
                                 activateScriptsDir,
                                 "profile-fish" );
        }
      if ( profile->tcsh.has_value() )
        {
          debugLog( "adding 'profile.tcsh' to activation scripts" );
          addScriptToScriptsDir( *profile->tcsh,
                                 activateScriptsDir,
                                 "profile-tcsh" );
        }
      if ( profile->zsh.has_value() )
        {
          debugLog( "adding 'profile.zsh' to activation scripts" );
          addScriptToScriptsDir( *profile->zsh,
                                 activateScriptsDir,
                                 "profile-zsh" );
        }
    }

  /* Add 'hook-on-activate' script. */
  auto hook = manifest.hook;
  if ( hook.has_value() )
    {
      // [hook.script] is deprecated, in favor of [profile.*].  For now we will
      // allow it.
      // TODO: print a warning??
      if ( hook->script.has_value() )
        {
          debugLog( "adding 'hook.script' to activation scripts" );
          addScriptToScriptsDir( *hook->script,
                                 activateScriptsDir,
                                 "hook-script" );
        }

      if ( hook->onActivate.has_value() )
        {
          debugLog( "adding 'hook.on-activate' to activation scripts" );
          addScriptToScriptsDir( *hook->onActivate,
                                 activateScriptsDir,
                                 "hook-on-activate" );
        }
    }

  debugLog( "adding activation scripts to store" );
  auto activationStorePath
    = state.store->addToStore( "activation-scripts", tempDir );

  RealisedPackage realised( state.store->printStorePath( activationStorePath ),
                            true,
                            buildenv::Priority() );
  auto            references = nix::StorePathSet();
  references.insert( activationStorePath );
  references.insert(
    state.store->parseStorePath( ACTIVATION_SCRIPTS_PACKAGE_DIR ) );
  references.insert( state.store->parseStorePath( FLOX_BASH_PKG ) );
  references.insert( state.store->parseStorePath( FLOX_CACERT_PKG ) );

  return { realised, references };
}

std::pair<buildenv::RealisedPackage, nix::StorePathSet>
makePackageBuildScripts( nix::EvalState &         state,
                         const BuildenvLockfile & lockfile )
{
  auto tempDir          = std::filesystem::path( nix::createTempDir() );
  auto packageBuildsDir = tempDir / PACKAGE_BUILDS_SUBDIR_NAME;
  std::filesystem::create_directories( packageBuildsDir );


  if ( lockfile.manifest.build.has_value() )
    {
      for ( auto [name, descriptor] : *lockfile.manifest.build )
        {
          addScriptToScriptsDir( descriptor.command, packageBuildsDir, name );
        }
    }

  debugLog( "adding package build scripts to store" );
  auto packageBuildsStorePath
    = state.store->addToStore( "package-build-scripts", tempDir );

  RealisedPackage realised(
    state.store->printStorePath( packageBuildsStorePath ),
    true,
    buildenv::Priority() );
  auto references = nix::StorePathSet();
  references.insert( packageBuildsStorePath );
  return { realised, references };
}


/* -------------------------------------------------------------------------- */

/**
 * @brief Make a @a RealisedPackage and store path for the activate package.
 * @param state Nix state.
 * @return A pair of the realised package and the store path of the activate
 * package.
 */
static std::pair<buildenv::RealisedPackage, nix::StorePath>
makeActivationScriptsPackageDir( nix::EvalState & state )
{
  /* Insert activation scripts.
   * The store path is provided at compile time via the
   * `ACTIVATION_SCRIPTS_PACKAGE_DIR' environment variable. */
  debugLog( nix::fmt( "adding activation scripts to store, path=%s",
                      ACTIVATION_SCRIPTS_PACKAGE_DIR ) );
  auto profileScriptsPath
    = state.store->parseStorePath( ACTIVATION_SCRIPTS_PACKAGE_DIR );
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
createFloxEnv( nix::ref<nix::EvalState> &         state,
               const nlohmann::json &             lockfileContent,
               const std::optional<std::string> & serviceConfigPath,
               const System &                     system )
{
  BuildenvLockfile lockfile = BuildenvLockfile();
  lockfile.load_from_content( lockfileContent );

  // Check this system is supported
  auto systems = lockfile.manifest.getSystems();
  if ( std::find( systems.begin(), systems.end(), system ) == systems.end() )
    {
      throw SystemNotSupportedByLockfile(
        "'" + system + "' not supported by this environment" );
    }


  /* Extract derivations */
  nix::StorePathSet                     references;
  std::vector<RealisedPackage>          pkgs;
  std::map<nix::StorePath, std::string> storePathsToInstallIds;

  for ( auto const & package : lockfile.packages )
    {
      // Skip any packages not for this system
      if ( package.system == system )
        {
          auto realised = getRealisedOutputs( state, package, system );
          for ( auto [realisedPackage, storePath] : realised )
            {
              pkgs.push_back( realisedPackage );
              references.insert( storePath );
              storePathsToInstallIds.insert( {
                storePath,
                package.installId,
              } );
            }
        }
    }

  // Add the environment's activation scripts to the environment
  auto [activationScriptPackage, activationScriptReferences]
    = makeActivationScripts( *state, lockfile );

  pkgs.push_back( activationScriptPackage );
  references.insert( activationScriptReferences.begin(),
                     activationScriptReferences.end() );

  // Add the scripts with our activation logic to the environment
  auto [profileScriptsPath, profileScriptsReference]
    = makeActivationScriptsPackageDir( *state );

  pkgs.push_back( profileScriptsPath );
  references.insert( profileScriptsReference );

  auto [packageBuildsPackage, packageBuildsReferences]
    = makePackageBuildScripts( *state, lockfile );

  pkgs.push_back( packageBuildsPackage );
  references.insert( packageBuildsReferences.begin(),
                     packageBuildsReferences.end() );

  return createEnvironmentStorePath( *state,
                                     pkgs,
                                     references,
                                     storePathsToInstallIds,
                                     serviceConfigPath );
}


nix::StorePath
createContainerBuilder( nix::EvalState &       state,
                        const nix::StorePath & environmentStorePath,
                        const System &         system )
{
  static const nix::FlakeRef nixpkgsRef
    = nix::parseFlakeRef( COMMON_NIXPKGS_URL );

  auto lockedNixpkgs
    = nix::flake::lockFlake( state, nixpkgsRef, nix::flake::LockFlags() );

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
