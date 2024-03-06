/* ========================================================================== *
 *
 * @file fetchers/wrapped-nixpkgs-input.cc
 *
 * @brief Fetches a `nix` input and applies a patch before evaluation.
 *
 *
 * -------------------------------------------------------------------------- */

#include <algorithm>
#include <cctype>
#include <cstdint>
#include <filesystem>
#include <fstream>
#include <map>
#include <memory>
#include <optional>
#include <regex>
#include <string>
#include <string_view>
#include <utility>
#include <variant>
#include <vector>

#include <nix/attrs.hh>
#include <nix/cache.hh>
#include <nix/error.hh>
#include <nix/fetchers.hh>
#include <nix/flake/flake.hh>
#include <nix/flake/flakeref.hh>
#include <nix/flake/lockfile.hh>
#include <nix/hash.hh>
#include <nix/logging.hh>
#include <nix/path.hh>
#include <nix/ref.hh>
#include <nix/store-api.hh>
#include <nix/types.hh>
#include <nix/url-parts.hh>
#include <nix/url.hh>
#include <nix/util.hh>

#include "flox/core/nix-state.hh"
#include "flox/core/util.hh"
#include "flox/fetchers/wrapped-nixpkgs-input.hh"
#include "flox/flox-flake.hh"


/* -------------------------------------------------------------------------- */

/* Forward declaration */
namespace nix {
class EvalState;
}


/* -------------------------------------------------------------------------- */

namespace flox {

/* -------------------------------------------------------------------------- */

/**
 * @brief Create a temporary directory containing a `flake.nix` which wraps
 *        @a nixpkgsRef configuring it to allow unfree and broken packages.
 */
static std::filesystem::path
createWrappedFlakeDirV0( const nix::FlakeRef & nixpkgsRef )
{
  /* Create a temporary directory to put the filled out template file in it. */
  std::filesystem::path tmpDir = nix::createTempDir();
  debugLog( "created temp dir for flake template: " + tmpDir.string() );

  /* Fill out the template with the flake references. */
  std::ofstream            flakeOut( tmpDir / "flake.nix" );
  static const std::string flakeTemplate =
#include "./flake-v0.nix.in.hh"
    ;
  std::istringstream flakeIn( flakeTemplate );
  std::string        line;
  while ( std::getline( flakeIn, line ) )
    {
      /* Inject URL */
      if ( line.find( "@NIXPKGS_URL@" ) != std::string::npos )
        {
          line.replace( line.find( "@NIXPKGS_URL@" ),
                        std::string( "@NIXPKGS_URL@" ).length(),
                        nixpkgsRef.to_string() );
        }
      flakeOut << line << '\n';
    }
  flakeOut.close();
  debugLog( "filled out flake template with flake reference: "
            + nixpkgsRef.to_string() );

  /* Lock the filled out template to avoid spurious re-locking and silence the
   * "Added input ..." message. */
  flox::NixState           nixState;
  nix::ref<nix::EvalState> state = nixState.getState();
  nix::FlakeRef wrappedRef = nix::parseFlakeRef( "path:" + tmpDir.string() );
  /* Push verbosity level to suppress "warning: creating lock file ..." */
  auto oldVerbosity = nix::verbosity;
  nix::verbosity    = nix::lvlError;
  auto _locked      = nix::flake::lockFlake( *state, wrappedRef, {} );
  /* Pop verbosity */
  nix::verbosity = oldVerbosity;
  debugLog( "locked flake template" );

  return tmpDir;
}


/* -------------------------------------------------------------------------- */

/** The latest `flox-nixpkgs` version available. Used by default. */
static const uint64_t latestWrapperVersion = 0;

/**
 * @brief Create a temporary directory containing a `flake.nix` which wraps
 *        @a nixpkgsRef applying evaluated changes.
 *
 * This alias should always refer to the routine associated
 * with `latestWrapperVersion`.
 */
static inline std::filesystem::path
createWrappedFlakeDir( const nix::FlakeRef & nixpkgsRef, uint64_t version = 0 )
{
  // NOLINTNEXTLINE(hicpp-multiway-paths-covered)
  switch ( version )
    {
      case 0: return flox::createWrappedFlakeDirV0( nixpkgsRef ); break;

      default:
        throw nix::Error( "unsupported 'version' '%d' in input '%s'",
                          version,
                          nixpkgsRef.to_string() );
        break;
    }
}


/* -------------------------------------------------------------------------- */

/** @brief Fetches a `nixpkgs` input and wraps it with a few modifications. */
struct WrappedNixpkgsInputScheme : nix::fetchers::InputScheme
{

  [[nodiscard]] virtual std::string
  type() const
  {
    return "flox-nixpkgs";
  }

  /** @brief Convert raw attributes into an input. */
  [[nodiscard]] std::optional<nix::fetchers::Input>
  inputFromAttrs( const nix::fetchers::Attrs & attrs ) const override;

  /** @brief Convert a URL string into an input. */
  [[nodiscard]] std::optional<nix::fetchers::Input>
  inputFromURL( const nix::ParsedURL & url ) const override;

  /** @brief Convert input to a URL representation. */
  [[nodiscard]] nix::ParsedURL
  toURL( const nix::fetchers::Input & input ) const override;

  /**
   * @brief Check to see if the input has all information necessary for use
   *        with SQLite caches.
   *
   * We require `rev` and `version` fields to be present.
   */
  [[nodiscard]] bool
  hasAllInfo( const nix::fetchers::Input & input ) const override;

  /**
   * @brief Override an input with a different `ref` or `rev`.
   *
   * This is unlikely to be used for our purposes; but because it's a part of
   * the `nix` fetcher interface, we implement it.
   */
  [[nodiscard]] nix::fetchers::Input
  applyOverrides( const nix::fetchers::Input & _input,
                  std::optional<std::string>   ref,
                  std::optional<nix::Hash>     rev ) const override;

  /**
   * @brief Clone the `nixpkgs` repository to prime the cache.
   *
   * This function is used by `nix flake archive` to pre-fetch sources.
   */
  void
  clone( const nix::fetchers::Input & input,
         const nix::Path &            destDir ) const override;

  /** @brief Generate a flake with wraps `nixpkgs`. */
  [[nodiscard]] std::pair<nix::StorePath, nix::fetchers::Input>
  fetch( nix::ref<nix::Store>         store,
         const nix::fetchers::Input & _input ) override;


}; /* End class `WrappedNixpkgsInputScheme' */


/* -------------------------------------------------------------------------- */

/**
 * @brief Helper used to convert a `flox-nixpkgs` attribute set representation,
 *        to a `github` attribute set representation.
 */
static nix::fetchers::Attrs
floxNixpkgsAttrsToGithubAttrs( const nix::fetchers::Attrs & attrs )
{
  nix::fetchers::Attrs _attrs;
  _attrs["type"]  = "github";
  _attrs["owner"] = "NixOS";
  _attrs["repo"]  = "nixpkgs";

  /* Inherit `rev' and `ref' fields */
  if ( auto rev = nix::fetchers::maybeGetStrAttr( attrs, "rev" ) )
    {
      _attrs["rev"] = *rev;
    }
  else if ( auto ref = nix::fetchers::maybeGetStrAttr( attrs, "ref" ) )
    {
      _attrs["ref"] = *ref;
    }
  else
    {
      throw nix::Error(
        "missing 'rev' or 'ref' field in 'flox-nixpkgs' input" );
    }

  return _attrs;
}

/* -------------------------------------------------------------------------- */

/**
 * @brief Helper used to convert a `github` attribute set representation,
 *        to a `flox-nixpkgs` attribute set representation.
 * @note This is the inverse of `floxNixpkgsAttrsToGithubAttrs`.
 * @param attrs The attribute set representation of an (assumed) `github` input.
 * @return The attribute set representation of a `flox-nixpkgs` input.
 * @throws nix::Error If the input type is not `github`.
 * @throws nix::Error If the input type is `github` but the `rev` or `ref`
 * fields are missing.
 * @throws nix::Error If the input owner/repo is not `NixOS/nixpkgs` (case
 * insensitive).
 *
 * @todo throw a flox exception instead of a nix exception for easier handling?
 * @todo support wrapping of other inputs than `github:nixos/nixpkgs`.
 * This would also require changes to the `WrappedNixpkgsInputScheme` class,
 * as well as existing conversion methods imlemented for it.
 */
nix::fetchers::Attrs
githubAttrsToFloxNixpkgsAttrs( const nix::fetchers::Attrs & attrs )
{
  auto type = nix::fetchers::getStrAttr( attrs, "type" );

  if ( type != "github" )
    {
      throw nix::Error( "unsupported input type '%s' expected 'github'", type );
    }

  auto owner = nix::fetchers::getStrAttr( attrs, "owner" );
  auto repo  = nix::fetchers::getStrAttr( attrs, "repo" );

  if ( nix::toLower( owner ) != "nixos" || nix::toLower( repo ) != "nixpkgs" )
    {
      throw nix::Error(
        "unsupported input owner/repo '%s/%s' expected 'NixOS/nixpkgs'",
        owner,
        repo );
    }


  nix::fetchers::Attrs _attrs;
  _attrs["type"]    = "flox-nixpkgs";
  _attrs["version"] = latestWrapperVersion;

  /* Inherit `rev' and `ref' fields */
  if ( auto rev = nix::fetchers::maybeGetStrAttr( attrs, "rev" ) )
    {
      _attrs["rev"] = *rev;
    }
  else if ( auto ref = nix::fetchers::maybeGetStrAttr( attrs, "ref" ) )
    {
      _attrs["ref"] = *ref;
    }
  else
    {
      throw nix::Error(
        "missing 'rev' or 'ref' field in 'flox-nixpkgs' input" );
    }

  return _attrs;
}


/* -------------------------------------------------------------------------- */

std::optional<nix::fetchers::Input>
WrappedNixpkgsInputScheme::inputFromAttrs(
  const nix::fetchers::Attrs & attrs ) const
{
  if ( nix::fetchers::maybeGetStrAttr( attrs, "type" ) != "flox-nixpkgs" )
    {
      return std::nullopt;
    }

  for ( const auto & [name, value] : attrs )
    {
      if ( ( name != "type" ) && ( name != "ref" ) && ( name != "rev" )
           && ( name != "narHash" ) && ( name != "version" ) )
        {
          throw nix::Error( "unsupported flox-nixpkgs input attribute '%s'",
                            name );
        }
    }

  /* Type check the following fields if they exist. */
  nix::fetchers::maybeGetStrAttr( attrs, "narHash" );
  nix::fetchers::maybeGetIntAttr( attrs, "version" );

  /* Check the rev field if present */
  if ( auto rev = nix::fetchers::maybeGetStrAttr( attrs, "rev" ) )
    {
      if ( ! std::regex_match( *rev, nix::revRegex ) )
        {
          throw nix::BadURL( "invalid Git commit hash '%s'", *rev );
        }
    }

  /* Check the ref field if present */
  if ( auto ref = nix::fetchers::maybeGetStrAttr( attrs, "ref" ) )
    {
      if ( std::regex_search( *ref, nix::badGitRefRegex ) )
        {
          throw nix::BadURL( "invalid Git branch/tag name '%s'", *ref );
        }
    }

  nix::fetchers::Input input;
  input.attrs = attrs;
  return input;
}


/* -------------------------------------------------------------------------- */

/**
 * @brief Parses an input from a URL with the schema
 *        `flox-nixpkgs:v<RULES-VERSION>/<REV-OR-REF>`.
 */
std::optional<nix::fetchers::Input>
WrappedNixpkgsInputScheme::inputFromURL( const nix::ParsedURL & url ) const
{
  if ( url.scheme != this->type() ) { return std::nullopt; }

  nix::fetchers::Input input;
  input.attrs.insert_or_assign( "type", this->type() );

  auto path = nix::tokenizeString<std::vector<std::string>>( url.path, "/" );

  if ( path.size() != 2 )
    {
      throw nix::BadURL( "URL '%s' is invalid", url.url );
    }

  if ( ( path[0].front() == 'v' )
       && ( std::find_if( path[0].begin() + 1,
                          path[0].end(),
                          []( unsigned char chr )
                          { return std::isdigit( chr ) == 0; } )
            == path[0].end() ) )
    {
      input.attrs.insert_or_assign(
        "version",
        nix::string2Int<uint64_t>(
          std::string_view( path[0].begin() + 1, path[0].end() ) )
          .value() );
    }
  else
    {
      throw nix::BadURL(
        "in URL '%s', '%s' is not a rules version tag like 'v<NUMBER>'",
        url.url,
        path[0] );
    }

  if ( std::regex_match( path[1], nix::revRegex ) )
    {
      input.attrs.insert_or_assign( "rev", path[1] );
    }
  else if ( std::regex_match( path[1], nix::refRegex ) )
    {
      if ( std::regex_match( path[1], nix::badGitRefRegex ) )
        {
          throw nix::BadURL(
            "in URL '%s', '%s' is not a valid Git branch/tag name",
            url.url,
            path[1] );
        }
      input.attrs.insert_or_assign( "ref", path[1] );
    }
  else
    {
      throw nix::BadURL(
        "in URL '%s', '%s' is not a Git commit hash or branch/tag name",
        url.url,
        path[1] );
    }

  return input;
}


/* -------------------------------------------------------------------------- */

nix::ParsedURL
WrappedNixpkgsInputScheme::toURL( const nix::fetchers::Input & input ) const
{
  nix::ParsedURL url;
  url.scheme = type();

  if ( auto version = nix::fetchers::maybeGetIntAttr( input.attrs, "version" ) )
    {
      url.path = "v" + std::to_string( *version );
    }
  else { throw nix::Error( "missing 'version' attribute in input" ); }

  if ( auto rev = nix::fetchers::maybeGetStrAttr( input.attrs, "rev" ) )
    {
      url.path += "/" + *rev;
    }
  else if ( auto ref = nix::fetchers::maybeGetStrAttr( input.attrs, "ref" ) )
    {
      url.path += "/" + *ref;
    }
  else { throw nix::Error( "missing 'rev' or 'ref' attribute in input" ); }

  return url;
}


/* -------------------------------------------------------------------------- */

bool
WrappedNixpkgsInputScheme::hasAllInfo(
  const nix::fetchers::Input & input ) const
{
  return nix::fetchers::maybeGetStrAttr( input.attrs, "rev" ).has_value()
         && nix::fetchers::maybeGetIntAttr( input.attrs, "version" )
              .has_value();
}


/* -------------------------------------------------------------------------- */

nix::fetchers::Input
WrappedNixpkgsInputScheme::applyOverrides( const nix::fetchers::Input & _input,
                                           std::optional<std::string>   ref,
                                           std::optional<nix::Hash> rev ) const
{
  auto input = _input;

  if ( rev.has_value() && ref.has_value() )
    {
      throw nix::BadURL(
        "cannot apply both a commit hash (%s) and a branch/tag "
        "name ('%s') to input '%s'",
        rev->gitRev(),
        *ref,
        input.to_string() );
    }
  if ( rev.has_value() )
    {
      input.attrs.insert_or_assign( "rev", rev->gitRev() );
      input.attrs.erase( "ref" );
    }
  else if ( ref.has_value() )
    {
      input.attrs.insert_or_assign( "ref", *ref );
      input.attrs.erase( "rev" );
    }

  return input;
}


/* -------------------------------------------------------------------------- */

/** @brief Clones the repository for analysis, but does not modify/patch it. */
void
WrappedNixpkgsInputScheme::clone( const nix::fetchers::Input & input,
                                  const nix::Path &            destDir ) const
{
  auto githubInput = nix::fetchers::Input::fromAttrs(
    floxNixpkgsAttrsToGithubAttrs( input.attrs ) );
  githubInput.clone( destDir );
}


/* -------------------------------------------------------------------------- */

std::pair<nix::StorePath, nix::fetchers::Input>
WrappedNixpkgsInputScheme::fetch( nix::ref<nix::Store>         store,
                                  const nix::fetchers::Input & _input )
{
  nix::fetchers::Input input( _input );

  /* Fill a fallback version is one wasn't given. */
  if ( ! nix::fetchers::maybeGetIntAttr( input.attrs, "version" ).has_value() )
    {
      input.attrs.insert_or_assign( "version", latestWrapperVersion );
    }

  /* Fill a fallback `ref' if one wasn't given.
   * This will get clobbered by `rev` if one was given. */
  if ( ! nix::fetchers::maybeGetStrAttr( input.attrs, "ref" ).has_value() )
    {
      input.attrs.insert_or_assign( "ref", "HEAD" );
    }

  /* If we don't have a `rev', get the revision hash from `ref'. */
  auto rev = input.getRev();
  if ( ! rev.has_value() )
    {
      /* Use existing GitHub fetcher in `nix' to lookup `rev'. */
      auto githubInput = nix::fetchers::Input::fromAttrs(
        floxNixpkgsAttrsToGithubAttrs( input.attrs ) );
      rev = githubInput.fetch( store ).second.getRev();
    }
  /* Now that we have a `rev' we can drop the `ref' field. */
  input.attrs.erase( "ref" );
  input.attrs.insert_or_assign( "rev", rev->gitRev() );

  /* Stash our locked attributes to be used as a SQL table key. */
  nix::fetchers::Attrs lockedAttrs(
    { { "type", "flox-nixpkgs" },
      { "version", nix::fetchers::getIntAttr( input.attrs, "version" ) },
      { "rev", rev->gitRev() } } );

  /* If we're already cached then we're done. */
  if ( auto res = nix::fetchers::getCache()->lookup( store, lockedAttrs ) )
    {
      return { std::move( res->second ), input };
    }

  /* Otherwise create our flake and add it the `nix' store. */
  auto flakeDir = createWrappedFlakeDir(
    nix::FlakeRef::fromAttrs( floxNixpkgsAttrsToGithubAttrs( input.attrs ) ),
    nix::fetchers::getIntAttr( input.attrs, "version" ) );

  nix::StorePath storePath = store->addToStore( input.getName(), flakeDir );

  /* If we had to lookup a `rev' from a `ref', add a cache entry associated with
   * the `ref'.
   * The final boolean argument to `add( ... )' tells the cache to only respect
   * this entry for a short period of time
   * ( according to _tarball TTL_ setting ). */
  if ( ! _input.getRev().has_value() )
    {
      nix::fetchers::getCache()->add( store,
                                      _input.attrs,
                                      { { "rev", rev->gitRev() } },
                                      storePath,
                                      false );
    }

  /* Add a cache entry for our locked reference. */
  nix::fetchers::getCache()->add( store,
                                  lockedAttrs,
                                  { { "rev", rev->gitRev() } },
                                  storePath,
                                  true );

  /* Return the store path for the generated flake, and it's
   * _locked_ input representation. */
  return { storePath, input };
}


/* -------------------------------------------------------------------------- */

/** Register this fetcher with `nix` on start-up. */
// Ignore clang-tidy warning about static constructor potentially throwing.
// NOLINTNEXTLINE(cert-err58-cpp)
static const auto rWrappedNixpkgsInputScheme = nix::OnStartup(
  []
  {
    nix::fetchers::registerInputScheme(
      std::make_unique<WrappedNixpkgsInputScheme>() );
  } );


/* -------------------------------------------------------------------------- */

}  // namespace flox

/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
