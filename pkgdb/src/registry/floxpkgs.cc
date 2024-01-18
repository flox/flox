/* ========================================================================== *
 *
 * @file registry/floxpkgs.cc
 *
 * @brief Provides a specialized `FloxFlake' which applies rules/pre-processing
 *        to a `flake' before it is evaluated.
 *        This is used to implement the `floxpkgs' catalog.
 *
 *
 * -------------------------------------------------------------------------- */

#include <fstream>
#include <iostream>
#include <optional>
#include <regex>
#include <string>

#include <nix/archive.hh>
#include <nix/cache.hh>
#include <nix/eval.hh>
#include <nix/fetch-settings.hh>
#include <nix/fetchers.hh>
#include <nix/store-api.hh>
#include <nix/url-parts.hh>

#include "flox/core/util.hh"
#include "flox/flox-flake.hh"
#include "flox/registry/floxpkgs.hh"


/* -------------------------------------------------------------------------- */

namespace flox {

/* -------------------------------------------------------------------------- */

#ifndef RULES_JSON
#  error "RULES_JSON must be defined"
#endif  // ifndef RULES_JSON


/* -------------------------------------------------------------------------- */

[[nodiscard]] static const std::filesystem::path
getRulesFile()
{
  return nix::getEnv( "_PKGDB_NIXPKGS_RULES_JSON" ).value_or( RULES_JSON );
}


/* -------------------------------------------------------------------------- */

/**
 * @brief Create a temporary directory containing a `flake.nix` which wraps
 *        @a nixpkgsRef, applying _rules_ from `rules.json`.
 */
std::filesystem::path
createWrappedFlakeDir( const nix::FlakeRef & nixpkgsRef )
{
  /* Create a temporary directory to put the filled out template and rules file
   * in. */
  std::filesystem::path tmpDir = nix::createTempDir();
  debugLog( "created temp dir for flake template: path=" + tmpDir.string() );
  std::filesystem::path rulesFilePath = getRulesFile();
  std::filesystem::copy( rulesFilePath, tmpDir / "rules.json" );

  /* Fill out the template with the flake references and the rules file path. */
  std::ofstream            flakeOut( tmpDir / "flake.nix" );
  static const std::string flakeTemplate =
#include "./floxpkgs/flake.nix.in.hh"
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

      /* Inject rules */
      if ( line.find( "@PKGDB_RULES_FILE@" ) != std::string::npos )
        {
          line.replace( line.find( "@PKGDB_RULES_FILE@" ),
                        std::string( "@PKGDB_RULES_FILE@" ).length(),
                        ( tmpDir / "rules.json" ).string() );
        }
      flakeOut << line << '\n';
    }
  flakeOut.close();
  debugLog( "filled out flake template: flake_ref=" + nixpkgsRef.to_string()
            + " rules_file_path=" + rulesFilePath.string() );

  /* Lock the filled out template to avoid spurious re-locking and silence the
   * "Added input ..." message. */
  flox::NixState           nixState;
  nix::ref<nix::EvalState> state = nixState.getState();
  nix::flake::LockFlags    flags;
  std::string              wrappedUrl = "path:" + tmpDir.string();
  nix::FlakeRef            wrappedRef = nix::parseFlakeRef( wrappedUrl );
  nix::flake::lockFlake( *state, wrappedRef, flags );
  debugLog( "locked flake template" );

  return tmpDir;
}


/* -------------------------------------------------------------------------- */
/* Copied straight from the nix codebase since these definitions aren't in
 * header files.*/

// A github, gitlab, or sourcehut host
const static std::string hostRegexS = "[a-zA-Z0-9.-]*";
std::regex               hostRegex( hostRegexS, std::regex::ECMAScript );

std::optional<nix::fetchers::Input>
GitArchiveInputScheme::inputFromURL( const nix::ParsedURL & url ) const
{
  if ( url.scheme != this->type() ) { return {}; }

  auto path = nix::tokenizeString<std::vector<std::string>>( url.path, "/" );

  std::optional<nix::Hash>   rev;
  std::optional<std::string> ref;
  std::optional<std::string> host_url;

  auto size = path.size();
  if ( size == 3 )
    {
      if ( std::regex_match( path[2], nix::revRegex ) )
        {
          rev = nix::Hash::parseAny( path[2], nix::htSHA1 );
        }
      else if ( std::regex_match( path[2], nix::refRegex ) ) { ref = path[2]; }
      else
        {
          throw nix::BadURL(
            "in URL '%s', '%s' is not a commit hash or branch/tag name",
            url.url,
            path[2] );
        }
    }
  else if ( size > 3 )
    {
      std::string rs;
      for ( auto i = std::next( path.begin(), 2 ); i != path.end(); i++ )
        {
          rs += *i;
          if ( std::next( i ) != path.end() ) { rs += "/"; }
        }

      if ( std::regex_match( rs, nix::refRegex ) ) { ref = rs; }
      else
        {
          throw nix::BadURL( "in URL '%s', '%s' is not a branch/tag name",
                             url.url,
                             rs );
        }
    }
  else if ( size < 2 ) { throw nix::BadURL( "URL '%s' is invalid", url.url ); }

  for ( auto & [name, value] : url.query )
    {
      if ( name == "rev" )
        {
          if ( rev )
            {
              throw nix::BadURL( "URL '%s' contains multiple commit hashes",
                                 url.url );
            }
          rev = nix::Hash::parseAny( value, nix::htSHA1 );
        }
      else if ( name == "ref" )
        {
          if ( ! std::regex_match( value, nix::refRegex ) )
            {
              throw nix::BadURL( "URL '%s' contains an invalid branch/tag name",
                                 url.url );
            }
          if ( ref )
            {
              throw nix::BadURL( "URL '%s' contains multiple branch/tag names",
                                 url.url );
            }
          ref = value;
        }
      else if ( name == "host" )
        {
          if ( ! std::regex_match( value, hostRegex ) )
            {
              throw nix::BadURL( "URL '%s' contains an invalid instance host",
                                 url.url );
            }
          host_url = value;
        }
    }

  if ( ref && rev )
    {
      throw nix::BadURL(
        "URL '%s' contains both a commit hash and a branch/tag name %s %s",
        url.url,
        *ref,
        rev->gitRev() );
    }

  nix::fetchers::Input input;
  input.attrs.insert_or_assign( "type", type() );
  input.attrs.insert_or_assign( "owner", path[0] );
  input.attrs.insert_or_assign( "repo", path[1] );
  if ( rev ) { input.attrs.insert_or_assign( "rev", rev->gitRev() ); }
  if ( ref ) { input.attrs.insert_or_assign( "ref", *ref ); }
  if ( host_url ) { input.attrs.insert_or_assign( "host", *host_url ); }

  return input;
}

std::optional<nix::fetchers::Input>
GitArchiveInputScheme::inputFromAttrs(
  const nix::fetchers::Attrs & attrs ) const
{
  if ( nix::fetchers::maybeGetStrAttr( attrs, "type" ) != this->type() )
    {
      return {};
    }

  for ( auto & [name, value] : attrs )
    {
      if ( name != "type" && name != "owner" && name != "repo" && name != "ref"
           && name != "rev" && name != "narHash" && name != "lastModified"
           && name != "host" )
        {
          throw nix::Error( "unsupported input attribute '%s'", name );
        }
    }

  nix::fetchers::getStrAttr( attrs, "owner" );
  nix::fetchers::getStrAttr( attrs, "repo" );

  nix::fetchers::Input input;
  input.attrs = attrs;
  return input;
}

nix::ParsedURL
GitArchiveInputScheme::toURL( const nix::fetchers::Input & input ) const
{
  auto owner = nix::fetchers::getStrAttr( input.attrs, "owner" );
  auto repo  = nix::fetchers::getStrAttr( input.attrs, "repo" );
  auto ref   = input.getRef();
  auto rev   = input.getRev();
  auto path  = owner + "/" + repo;
  assert( ! ( ref && rev ) );
  if ( ref ) { path += "/" + *ref; }
  if ( rev ) { path += "/" + rev->to_string( nix::Base16, false ); }
  return nix::ParsedURL {
    .scheme = this->type(),
    .path   = path,
  };
}

bool
GitArchiveInputScheme::hasAllInfo( const nix::fetchers::Input & input ) const
{
  return input.getRev()
         && nix::fetchers::maybeGetIntAttr( input.attrs, "lastModified" );
}

nix::fetchers::Input
GitArchiveInputScheme::applyOverrides( const nix::fetchers::Input & _input,
                                       std::optional<std::string>   ref,
                                       std::optional<nix::Hash>     rev ) const
{
  auto input( _input );
  if ( rev && ref )
    {
      throw nix::BadURL(
        "cannot apply both a commit hash (%s) and a branch/tag "
        "name ('%s') to input '%s'",
        rev->gitRev(),
        *ref,
        input.to_string() );
    }
  if ( rev )
    {
      input.attrs.insert_or_assign( "rev", rev->gitRev() );
      input.attrs.erase( "ref" );
    }
  if ( ref )
    {
      input.attrs.insert_or_assign( "ref", *ref );
      input.attrs.erase( "rev" );
    }
  return input;
}

std::optional<std::string>
GitArchiveInputScheme::getAccessToken( const std::string & host ) const
{
  auto tokens = nix::fetchSettings.accessTokens.get();
  if ( auto token = nix::get( tokens, host ) ) { return *token; }
  return {};
}

nix::Headers
GitArchiveInputScheme::makeHeadersWithAuthTokens(
  const std::string & host ) const
{
  nix::Headers headers;
  auto         accessToken = getAccessToken( host );
  if ( accessToken )
    {
      auto hdr = accessHeaderFromToken( *accessToken );
      if ( hdr ) { headers.push_back( *hdr ); }
      else { nix::warn( "Unrecognized access token for host '%s'", host ); }
    }
  return headers;
}

std::pair<nix::StorePath, nix::fetchers::Input>
GitArchiveInputScheme::fetch( nix::ref<nix::Store>         store,
                              const nix::fetchers::Input & _input )
{
  nix::fetchers::Input input( _input );

  if ( ! nix::fetchers::maybeGetStrAttr( input.attrs, "ref" ) )
    {
      input.attrs.insert_or_assign( "ref", "HEAD" );
    }

  auto rev = input.getRev();
  if ( ! rev ) { rev = getRevFromRef( store, input ); }

  input.attrs.erase( "ref" );
  input.attrs.insert_or_assign( "rev", rev->gitRev() );

  nix::fetchers::Attrs lockedAttrs( {
    { "type", "git-tarball" },
    { "rev", rev->gitRev() },
  } );

  if ( auto res = nix::fetchers::getCache()->lookup( store, lockedAttrs ) )
    {
      input.attrs.insert_or_assign(
        "lastModified",
        nix::fetchers::getIntAttr( res->first, "lastModified" ) );
      return { std::move( res->second ), input };
    }

  auto url = getDownloadUrl( input );

  auto result = nix::fetchers::downloadTarball( store,
                                                url.url,
                                                input.getName(),
                                                true,
                                                url.headers );

  input.attrs.insert_or_assign( "lastModified",
                                uint64_t( result.lastModified ) );

  nix::fetchers::getCache()->add(
    store,
    lockedAttrs,
    { { "rev", rev->gitRev() },
      { "lastModified", uint64_t( result.lastModified ) } },
    result.tree.storePath,
    true );

  return { result.tree.storePath, input };
}


std::string
GitHubInputScheme::type() const
{
  return "github";
}

std::optional<std::pair<std::string, std::string>>
GitHubInputScheme::accessHeaderFromToken( const std::string & token ) const
{
  // Github supports PAT/OAuth2 tokens and HTTP Basic
  // Authentication.  The former simply specifies the token, the
  // latter can use the token as the password.  Only the first
  // is used here. See
  // https://developer.github.com/v3/#authentication and
  // https://docs.github.com/en/developers/apps/authorizing-oath-apps
  return std::pair<std::string, std::string>( "Authorization",
                                              nix::fmt( "token %s", token ) );
}

std::string
GitHubInputScheme::getHost( const nix::fetchers::Input & input ) const
{
  return nix::fetchers::maybeGetStrAttr( input.attrs, "host" )
    .value_or( "github.com" );
}

std::string
GitHubInputScheme::getOwner( const nix::fetchers::Input & input ) const
{
  return nix::fetchers::getStrAttr( input.attrs, "owner" );
}

std::string
GitHubInputScheme::getRepo( const nix::fetchers::Input & input ) const
{
  return nix::fetchers::getStrAttr( input.attrs, "repo" );
}

nix::Hash
GitHubInputScheme::getRevFromRef( nix::ref<nix::Store>         store,
                                  const nix::fetchers::Input & input ) const
{
  auto host = getHost( input );
  auto url  = nix::fmt( host == "github.com"
                         ? "https://api.%s/repos/%s/%s/commits/%s"
                         : "https://%s/api/v3/repos/%s/%s/commits/%s",
                       host,
                       getOwner( input ),
                       getRepo( input ),
                       *input.getRef() );

  nix::Headers headers = makeHeadersWithAuthTokens( host );

  auto json = nlohmann::json::parse( nix::readFile( store->toRealPath(
    nix::fetchers::downloadFile( store, url, "source", false, headers )
      .storePath ) ) );
  auto rev  = nix::Hash::parseAny( std::string { json["sha"] }, nix::htSHA1 );
  // nix::debug( "HEAD revision for '%s' is %s", url, rev.gitRev() );
  return rev;
}

DownloadUrl
GitHubInputScheme::getDownloadUrl( const nix::fetchers::Input & input ) const
{
  auto host = getHost( input );

  nix::Headers headers = makeHeadersWithAuthTokens( host );

  // If we have no auth headers then we default to the public archive
  // urls so we do not run into rate limits.
  const auto urlFmt
    = host != "github.com" ? "https://%s/api/v3/repos/%s/%s/tarball/%s"
      : headers.empty()    ? "https://%s/%s/%s/archive/%s.tar.gz"
                           : "https://api.%s/repos/%s/%s/tarball/%s";

  const auto url = nix::fmt( urlFmt,
                             host,
                             getOwner( input ),
                             getRepo( input ),
                             input.getRev()->to_string( nix::Base16, false ) );

  return DownloadUrl { url, headers };
}

void
GitHubInputScheme::clone( const nix::fetchers::Input & input,
                          const nix::Path &            destDir ) const
{
  auto host = getHost( input );
  nix::fetchers::Input::fromURL( nix::fmt( "git+https://%s/%s/%s.git",
                                           host,
                                           getOwner( input ),
                                           getRepo( input ) ) )
    .applyOverrides( input.getRef(), input.getRev() )
    .clone( destDir );
}

/* -------------------------------------------------------------------------- */


std::string
FloxFlakeScheme::type() const
{
  return flox::FLOX_FLAKE_TYPE;
}

std::optional<nix::fetchers::Input>
FloxFlakeScheme::inputFromURL( const nix::ParsedURL & url ) const
{
  /* TODO: if the type is flox-nixpkgs we can short circuit this */
  /* don't try to convert github references */
  if ( url.scheme != this->type() ) { return {}; }
  auto asGithub   = url;
  asGithub.scheme = "github";
  GitHubInputScheme                   githubScheme;
  std::optional<nix::fetchers::Input> fromGithub
    = githubScheme.inputFromURL( asGithub );
  if ( fromGithub.has_value() )
    {
      fromGithub->attrs.insert_or_assign( "type", flox::FLOX_FLAKE_TYPE );
      fromGithub->scheme = std::make_shared<FloxFlakeScheme>();
      return fromGithub;
    }
  else { return {}; }
}

std::optional<nix::fetchers::Input>
FloxFlakeScheme::inputFromAttrs( const nix::fetchers::Attrs & attrs ) const
{
  std::optional<nix::fetchers::Input> fromGithub
    = GitHubInputScheme::inputFromAttrs( attrs );
  if ( ! fromGithub.has_value() ) { return std::nullopt; }
  else
    {
      fromGithub->attrs.insert_or_assign( "type", flox::FLOX_FLAKE_TYPE );
      fromGithub->scheme = std::make_shared<FloxFlakeScheme>();
      return fromGithub;
    }
}

std::pair<nix::StorePath, nix::fetchers::Input>
FloxFlakeScheme::fetch( nix::ref<nix::Store>         store,
                        const nix::fetchers::Input & input )
{
  flox::debugLog( "using our fetcher" );
  nix::fetchers::Input asGithub = input;
  asGithub.attrs.insert_or_assign( "type", "github" );
  nix::FlakeRef nixpkgsRef = nix::FlakeRef::fromAttrs( asGithub.attrs );
  auto          flakeDir   = flox::createWrappedFlakeDir( nixpkgsRef );
  flox::debugLog( "created wrapped flake: path=" + std::string( flakeDir ) );
  nix::StringSink sink;
  nix::dumpPath( flakeDir, sink );
  auto               narHash = hashString( nix::htSHA256, sink.s );
  nix::ValidPathInfo info {
    *store,
    "source",
    nix::FixedOutputInfo {
      .method     = nix::FileIngestionMethod::Recursive,
      .hash       = narHash,
      .references = {},
    },
    narHash,
  };
  info.narSize = sink.s.size();

  nix::StringSource source( sink.s );
  store->addToStore( info, source );
  nix::StorePath path = info.path;
  flox::debugLog( "added filled out template flake to store: store_path="
                  + std::string( path.to_string() ) );
  asGithub.attrs.insert_or_assign( "type", flox::FLOX_FLAKE_TYPE );
  asGithub.scheme = std::make_shared<FloxFlakeScheme>();
  return std::pair<nix::StorePath, nix::fetchers::Input>( path, asGithub );
}

bool
FloxFlakeScheme::hasAllInfo( const nix::fetchers::Input & ) const
{
  return true;
}

nix::ParsedURL
FloxFlakeScheme::toURL( const nix::fetchers::Input & input ) const
{
  GitHubInputScheme githubScheme;
  auto              url = githubScheme.toURL( input );
  url.scheme            = this->type();
  return url;
}

/* -------------------------------------------------------------------------- */

static auto FloxFlakeInputScheme = nix::OnStartup(
  []
  {
    nix::fetchers::registerInputScheme( std::make_unique<FloxFlakeScheme>() );
  } );

/* -------------------------------------------------------------------------- */


}  // namespace flox


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
