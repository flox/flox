/* ========================================================================== *
 *
 * @file registry/wrapped-nixpkgs-input.cc
 *
 * @brief Fetches a `nix` input and applies a patch before evaluation.
 *
 *
 * -------------------------------------------------------------------------- */

#include <filesystem>
#include <fstream>
#include <string>

#include <nix/cache.hh>
#include <nix/fetchers.hh>
#include <nix/store-api.hh>
#include <nix/url-parts.hh>

#include "flox/core/exceptions.hh"
#include "flox/core/nix-state.hh"
#include "flox/core/util.hh"


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
  /* Create a temporary directory to put the filled out template and rules file
   * in. */
  std::filesystem::path tmpDir = nix::createTempDir();
  debugLog( "created temp dir for flake template: " + tmpDir.string() );

  /* Fill out the template with the flake references and the rules file path. */
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
  debugLog( "filled out flake template with flake-ref:"
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

static const uint64_t latestWrapperVersion = 0;

/**
 * @brief Create a temporary directory containing a `flake.nix` which wraps
 *        @a nixpkgsRef applying evaluated changes.
 *
 * This alias should always refer to the routine associated
 * with `latestWrapperVersion`.
 */
static inline std::filesystem::path
createWrappedFlakeDir( const nix::FlakeRef & nixpkgsRef )
{
  return createWrappedFlakeDirV0( nixpkgsRef );
}


/* -------------------------------------------------------------------------- */

struct WrappedNixpkgsInputScheme : nix::fetchers::InputScheme
{

  virtual std::string
  type() const
  {
    return "flox-nixpkgs";
  }

  std::optional<nix::fetchers::Input>
  inputFromAttrs( const nix::fetchers::Attrs & attrs ) const override;

  std::optional<nix::fetchers::Input>
  inputFromURL( const nix::ParsedURL & url ) const override;

  nix::ParsedURL
  toURL( const nix::fetchers::Input & input ) const override;

  bool
  hasAllInfo( const nix::fetchers::Input & input ) const override;

  nix::fetchers::Input
  applyOverrides( const nix::fetchers::Input & _input,
                  std::optional<std::string>   ref,
                  std::optional<nix::Hash>     rev ) const override;

  void
  clone( const nix::fetchers::Input & input,
         const nix::Path &            destDir ) const override;

  std::pair<nix::StorePath, nix::fetchers::Input>
  fetch( nix::ref<nix::Store>         store,
         const nix::fetchers::Input & _input ) override;

  nix::Hash
  getRevFromRef( nix::ref<nix::Store>         store,
                 const nix::fetchers::Input & input ) const;


}; /* End class `WrappedNixpkgsInputScheme' */


/* -------------------------------------------------------------------------- */

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
        "missing `rev` or `ref` field in `flox-nixpkgs` input" );
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

  for ( auto & [name, value] : attrs )
    {
      if ( ( name != "type" ) && ( name != "ref" ) && ( name != "rev" )
           && ( name != "narHash" ) && ( name != "version" )
           && ( name != "lastModified" ) )
        {
          throw nix::Error( "unsupported flox-nixpkgs input attribute '%s'",
                            name );
        }
    }

  /* Type check the following fields if they exist. */
  nix::fetchers::maybeGetStrAttr( attrs, "narHash" );
  nix::fetchers::maybeGetIntAttr( attrs, "version" );
  nix::fetchers::maybeGetIntAttr( attrs, "lastModified" );

  /*  */
  if ( auto ref = nix::fetchers::maybeGetStrAttr( attrs, "rev" ) )
    {
      if ( std::regex_search( *ref, nix::revRegex ) )
        {
          throw nix::BadURL( "invalid Git commit hash '%s'", *ref );
        }
    }

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

  if ( path.size() != 2 )
    {
      throw nix::BadURL( "URL '%s' is invalid", url.url );
    }

  if ( ( path[0].front() == 'v' )
       && ( std::find_if( path[0].begin() + 1,
                          path[0].end(),
                          []( unsigned char chr )
                          { return ! std::isdigit( chr ); } )
            == path[0].end() ) )
    {
      input.attrs.insert_or_assign(
        "version",
        static_cast<uint64_t>( atoll( path[0].c_str() + 1 ) ) );
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

  if ( rev && ref )
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

  if ( ! nix::fetchers::maybeGetIntAttr( input.attrs, "version" ).has_value() )
    {
      input.attrs.insert_or_assign( "version", latestWrapperVersion );
    }

  if ( ! nix::fetchers::maybeGetStrAttr( input.attrs, "ref" ).has_value() )
    {
      input.attrs.insert_or_assign( "ref", "HEAD" );
    }

  auto rev = input.getRev();
  if ( ! rev.has_value() )
    {
      /* Use existing GitHub fetcher in `nix` to lookup `rev`. */
      auto githubInput = nix::fetchers::Input::fromAttrs(
        floxNixpkgsAttrsToGithubAttrs( input.attrs ) );
      rev = githubInput.fetch( store ).second.getRev();
    }

  input.attrs.erase( "ref" );
  input.attrs.insert_or_assign( "rev", rev->gitRev() );

  nix::fetchers::Attrs lockedAttrs(
    { { "type", "flox-nixpkgs" },
      { "version", nix::fetchers::getIntAttr( input.attrs, "version" ) },
      { "rev", rev->gitRev() } } );

  /* If we're already cached then we're done. */
  if ( auto res = nix::fetchers::getCache()->lookup( store, lockedAttrs ) )
    {
      return { std::move( res->second ), input };
    }

  std::optional<std::filesystem::path> flakeDir;
  switch ( nix::fetchers::getIntAttr( input.attrs, "version" ) )
    {
      case 0:
        flakeDir = flox::createWrappedFlakeDirV0( nix::FlakeRef::fromAttrs(
          floxNixpkgsAttrsToGithubAttrs( lockedAttrs ) ) );
        break;

      default:
        throw nix::Error( "Unsupported 'version' '%d' in input '%s'",
                          nix::fetchers::getIntAttr( input.attrs, "version" ),
                          input.toURLString() );
        break;
    }
  assert( flakeDir.has_value() );

  nix::StorePath storePath = store->addToStore( input.getName(), *flakeDir );

  if ( ! _input.getRev().has_value() )
    {
      nix::fetchers::getCache()->add( store,
                                      _input.attrs,
                                      { { "rev", rev->gitRev() } },
                                      storePath,
                                      false );
    }

  nix::fetchers::getCache()->add( store,
                                  lockedAttrs,
                                  { { "rev", rev->gitRev() } },
                                  storePath,
                                  true );

  return { storePath, input };
}


/* -------------------------------------------------------------------------- */

static auto rWrappedNixpkgsInputScheme = nix::OnStartup(
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
