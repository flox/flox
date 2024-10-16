/* ========================================================================== *
 *
 * @file realisepkgs/realise.cc
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
#include <nix/eval-settings.hh>
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

#include "flox/fetchers/wrapped-nixpkgs-input.hh"
#include "flox/realisepkgs/realise.hh"
#include "flox/resolver/lockfile.hh"


/* -------------------------------------------------------------------------- */

namespace flox::realisepkgs {

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

std::vector<std::pair<realisepkgs::RealisedPackage, nix::StorePath>>
collectRealisedOutputs(
  nix::ref<nix::EvalState> &                     state,
  const RealisepkgsLockedPackage &               lockedPackage,
  std::unordered_map<std::string, std::string> & outputsToOutpaths )
{
  std::vector<std::pair<realisepkgs::RealisedPackage, nix::StorePath>> pkgs;
  for ( const auto & [name, outpathStr] : outputsToOutpaths )
    {
      debugLog( nix::fmt( "processing output '%s' of '%s'",
                          name,
                          lockedPackage.installId ) );
      auto outpathForOutput = state->store->parseStorePath( outpathStr );
      realisepkgs::RealisedPackage pkg(
        state->store->printStorePath( outpathForOutput ),
        true );
      pkgs.emplace_back( pkg, outpathForOutput );
    }
  return pkgs;
}


/* -------------------------------------------------------------------------- */

std::vector<std::pair<realisepkgs::RealisedPackage, nix::StorePath>>
getRealisedOutputs( nix::ref<nix::EvalState> &       state,
                    const RealisepkgsLockedPackage & lockedPackage )
{
  debugLog( nix::fmt( "getting cursor for %s", lockedPackage.attrPath[0] ) );
  auto timeEvalStart = std::chrono::high_resolution_clock::now();
  auto cursor        = evalCacheCursorForInput( state,
                                         lockedPackage.input,
                                         lockedPackage.attrPath );

  /**
   * Collect the store paths for each output of the package.
   * Note that the "out" output is the same as the package's outPath.
   */
  auto outputsToOutpaths
    = outpathsForPackageOutputs( state, lockedPackage.installId, cursor );


  auto pkgs = collectRealisedOutputs( state, lockedPackage, outputsToOutpaths );
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

/**
 * @brief Create a nix package for an environment definition.
 * @param state Nix state.
 * @param lockfile Lockfile to extract environment definition from.
 * @param system System to create the environment for.
 * @return The store path of the environment.
 */
std::vector<RealisedPackage>
realiseFloxEnvPackages( nix::ref<nix::EvalState> & state,
                        const nlohmann::json &     lockfileContent,
                        const System &             system )
{
  RealisepkgsLockfile lockfile = RealisepkgsLockfile();
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
          auto realised = getRealisedOutputs( state, package );
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

  return pkgs;
}

/* -------------------------------------------------------------------------- */

}  // namespace flox::realisepkgs


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
