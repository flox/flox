#include <nix/archive.hh>
#include <nix/cache.hh>
#include <nix/eval.hh>
#include <nix/fetch-settings.hh>
#include <nix/fetchers.hh>
#include <nix/store-api.hh>
#include <nix/url-parts.hh>
#include <optional>
#include <regex>
#include <string>

#include "flox/core/util.hh"
#include "flox/registry/floxpkgs.hh"

namespace nix::fetchers {

/* -------------------------------------------------------------------------- */
/* Copied straight from the nix codebase since these definitions aren't in
 * header files.*/

struct DownloadUrl
{
  std::string url;
  Headers     headers;
};

// A github, gitlab, or sourcehut host
const static std::string hostRegexS = "[a-zA-Z0-9.-]*";  // FIXME: check
std::regex               hostRegex( hostRegexS, std::regex::ECMAScript );

struct GitHubInputScheme;

struct GitArchiveInputScheme : InputScheme
{
  virtual std::string
  type() const
    = 0;

  virtual std::optional<std::pair<std::string, std::string>>
  accessHeaderFromToken( const std::string & token ) const = 0;

  std::optional<Input>
  inputFromURL( const ParsedURL & url ) const override
  {
    if ( url.scheme != type() ) { return {}; }

    auto path = tokenizeString<std::vector<std::string>>( url.path, "/" );

    std::optional<Hash>        rev;
    std::optional<std::string> ref;
    std::optional<std::string> host_url;

    auto size = path.size();
    if ( size == 3 )
      {
        if ( std::regex_match( path[2], revRegex ) )
          {
            rev = Hash::parseAny( path[2], htSHA1 );
          }
        else if ( std::regex_match( path[2], refRegex ) ) { ref = path[2]; }
        else
          {
            throw BadURL(
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

        if ( std::regex_match( rs, refRegex ) ) { ref = rs; }
        else
          {
            throw BadURL( "in URL '%s', '%s' is not a branch/tag name",
                          url.url,
                          rs );
          }
      }
    else if ( size < 2 ) { throw BadURL( "URL '%s' is invalid", url.url ); }

    for ( auto & [name, value] : url.query )
      {
        if ( name == "rev" )
          {
            if ( rev )
              {
                throw BadURL( "URL '%s' contains multiple commit hashes",
                              url.url );
              }
            rev = Hash::parseAny( value, htSHA1 );
          }
        else if ( name == "ref" )
          {
            if ( ! std::regex_match( value, refRegex ) )
              {
                throw BadURL( "URL '%s' contains an invalid branch/tag name",
                              url.url );
              }
            if ( ref )
              {
                throw BadURL( "URL '%s' contains multiple branch/tag names",
                              url.url );
              }
            ref = value;
          }
        else if ( name == "host" )
          {
            if ( ! std::regex_match( value, hostRegex ) )
              {
                throw BadURL( "URL '%s' contains an invalid instance host",
                              url.url );
              }
            host_url = value;
          }
      }

    if ( ref && rev )
      {
        throw BadURL(
          "URL '%s' contains both a commit hash and a branch/tag name %s %s",
          url.url,
          *ref,
          rev->gitRev() );
      }

    Input input;
    input.attrs.insert_or_assign( "type", type() );
    input.attrs.insert_or_assign( "owner", path[0] );
    input.attrs.insert_or_assign( "repo", path[1] );
    if ( rev ) { input.attrs.insert_or_assign( "rev", rev->gitRev() ); }
    if ( ref ) { input.attrs.insert_or_assign( "ref", *ref ); }
    if ( host_url ) { input.attrs.insert_or_assign( "host", *host_url ); }

    return input;
  }

  std::optional<Input>
  inputFromAttrs( const Attrs & attrs ) const override
  {
    if ( maybeGetStrAttr( attrs, "type" ) != type() ) { return {}; }

    for ( auto & [name, value] : attrs )
      {
        if ( name != "type" && name != "owner" && name != "repo"
             && name != "ref" && name != "rev" && name != "narHash"
             && name != "lastModified" && name != "host" )
          {
            throw Error( "unsupported input attribute '%s'", name );
          }
      }

    getStrAttr( attrs, "owner" );
    getStrAttr( attrs, "repo" );

    Input input;
    input.attrs = attrs;
    return input;
  }

  ParsedURL
  toURL( const Input & input ) const override
  {
    auto owner = getStrAttr( input.attrs, "owner" );
    auto repo  = getStrAttr( input.attrs, "repo" );
    auto ref   = input.getRef();
    auto rev   = input.getRev();
    auto path  = owner + "/" + repo;
    assert( ! ( ref && rev ) );
    if ( ref ) { path += "/" + *ref; }
    if ( rev ) { path += "/" + rev->to_string( Base16, false ); }
    return ParsedURL {
      .scheme = type(),
      .path   = path,
    };
  }

  bool
  hasAllInfo( const Input & input ) const override
  {
    return input.getRev() && maybeGetIntAttr( input.attrs, "lastModified" );
  }

  Input
  applyOverrides( const Input &              _input,
                  std::optional<std::string> ref,
                  std::optional<Hash>        rev ) const override
  {
    auto input( _input );
    if ( rev && ref )
      {
        throw BadURL( "cannot apply both a commit hash (%s) and a branch/tag "
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
  getAccessToken( const std::string & host ) const
  {
    auto tokens = fetchSettings.accessTokens.get();
    if ( auto token = get( tokens, host ) ) { return *token; }
    return {};
  }

  Headers
  makeHeadersWithAuthTokens( const std::string & host ) const
  {
    Headers headers;
    auto    accessToken = getAccessToken( host );
    if ( accessToken )
      {
        auto hdr = accessHeaderFromToken( *accessToken );
        if ( hdr ) { headers.push_back( *hdr ); }
        else { warn( "Unrecognized access token for host '%s'", host ); }
      }
    return headers;
  }

  virtual Hash
  getRevFromRef( nix::ref<Store> store, const Input & input ) const
    = 0;

  virtual DownloadUrl
  getDownloadUrl( const Input & input ) const
    = 0;

  std::pair<StorePath, Input>
  fetch( ref<Store> store, const Input & _input ) override
  {
    Input input( _input );

    if ( ! maybeGetStrAttr( input.attrs, "ref" ) )
      {
        input.attrs.insert_or_assign( "ref", "HEAD" );
      }

    auto rev = input.getRev();
    if ( ! rev ) { rev = getRevFromRef( store, input ); }

    input.attrs.erase( "ref" );
    input.attrs.insert_or_assign( "rev", rev->gitRev() );

    Attrs lockedAttrs( {
      { "type", "git-tarball" },
      { "rev", rev->gitRev() },
    } );

    if ( auto res = getCache()->lookup( store, lockedAttrs ) )
      {
        input.attrs.insert_or_assign(
          "lastModified",
          getIntAttr( res->first, "lastModified" ) );
        return { std::move( res->second ), input };
      }

    auto url = getDownloadUrl( input );

    auto result
      = downloadTarball( store, url.url, input.getName(), true, url.headers );

    input.attrs.insert_or_assign( "lastModified",
                                  uint64_t( result.lastModified ) );

    getCache()->add( store,
                     lockedAttrs,
                     { { "rev", rev->gitRev() },
                       { "lastModified", uint64_t( result.lastModified ) } },
                     result.tree.storePath,
                     true );

    return { result.tree.storePath, input };
  }
};

struct GitHubInputScheme : GitArchiveInputScheme
{
  std::string
  type() const override
  {
    return "github";
  }

  std::optional<std::pair<std::string, std::string>>
  accessHeaderFromToken( const std::string & token ) const override
  {
    // Github supports PAT/OAuth2 tokens and HTTP Basic
    // Authentication.  The former simply specifies the token, the
    // latter can use the token as the password.  Only the first
    // is used here. See
    // https://developer.github.com/v3/#authentication and
    // https://docs.github.com/en/developers/apps/authorizing-oath-apps
    return std::pair<std::string, std::string>( "Authorization",
                                                fmt( "token %s", token ) );
  }

  std::string
  getHost( const Input & input ) const
  {
    return maybeGetStrAttr( input.attrs, "host" ).value_or( "github.com" );
  }

  std::string
  getOwner( const Input & input ) const
  {
    return getStrAttr( input.attrs, "owner" );
  }

  std::string
  getRepo( const Input & input ) const
  {
    return getStrAttr( input.attrs, "repo" );
  }

  Hash
  getRevFromRef( nix::ref<Store> store, const Input & input ) const override
  {
    auto host = getHost( input );
    auto url
      = fmt( host == "github.com" ? "https://api.%s/repos/%s/%s/commits/%s"
                                  : "https://%s/api/v3/repos/%s/%s/commits/%s",
             host,
             getOwner( input ),
             getRepo( input ),
             *input.getRef() );

    Headers headers = makeHeadersWithAuthTokens( host );

    auto json = nlohmann::json::parse( readFile( store->toRealPath(
      downloadFile( store, url, "source", false, headers ).storePath ) ) );
    auto rev  = Hash::parseAny( std::string { json["sha"] }, htSHA1 );
    debug( "HEAD revision for '%s' is %s", url, rev.gitRev() );
    return rev;
  }

  DownloadUrl
  getDownloadUrl( const Input & input ) const override
  {
    auto host = getHost( input );

    Headers headers = makeHeadersWithAuthTokens( host );

    // If we have no auth headers then we default to the public archive
    // urls so we do not run into rate limits.
    const auto urlFmt
      = host != "github.com" ? "https://%s/api/v3/repos/%s/%s/tarball/%s"
        : headers.empty()    ? "https://%s/%s/%s/archive/%s.tar.gz"
                             : "https://api.%s/repos/%s/%s/tarball/%s";

    const auto url = fmt( urlFmt,
                          host,
                          getOwner( input ),
                          getRepo( input ),
                          input.getRev()->to_string( Base16, false ) );

    return DownloadUrl { url, headers };
  }

  void
  clone( const Input & input, const Path & destDir ) const override
  {
    auto host = getHost( input );
    Input::fromURL( fmt( "git+https://%s/%s/%s.git",
                         host,
                         getOwner( input ),
                         getRepo( input ) ) )
      .applyOverrides( input.getRef(), input.getRev() )
      .clone( destDir );
  }
};

/* -------------------------------------------------------------------------- */

struct FloxFlakeScheme : GitHubInputScheme
{
  std::string
  type() const override
  {
    return flox::FLOX_FLAKE_TYPE;
  }

  std::optional<Input>
  inputFromURL( const ParsedURL & url ) const override
  {
    /* TODO: if the type is flox-nixpkgs we can short circuit this */
    /* don't try to convert github references */
    if ( url.scheme != type() ) { return {}; }
    auto asGithub   = url;
    asGithub.scheme = "github";
    GitHubInputScheme    githubScheme;
    std::optional<Input> fromGithub = githubScheme.inputFromURL( asGithub );
    if ( fromGithub.has_value() ) { return fromGithub; }
    else { return {}; }
  }

  std::optional<Input>
  inputFromAttrs( const Attrs & attrs ) const override
  {
    std::optional<Input> fromGithub
      = GitHubInputScheme::inputFromAttrs( attrs );
    if ( fromGithub.has_value() ) { return fromGithub; }
    else { return {}; }
  }

  std::pair<StorePath, Input>
  fetch( ref<Store> store, const Input & input ) override
  {
    flox::debugLog( "using our fetcher" );
    Input backToGithub = input;
    backToGithub.attrs.insert_or_assign( "type", "github" );
    nix::FlakeRef nixpkgsRef = nix::FlakeRef::fromAttrs( backToGithub.attrs );
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
                    + path.to_string() );
    return std::pair<StorePath, Input>( path, input );
  }

  bool
  hasAllInfo( const Input & ) const override
  {
    return true;
  }
};

/* -------------------------------------------------------------------------- */

static auto FloxFlakeInputScheme = OnStartup(
  [] { registerInputScheme( std::make_unique<FloxFlakeScheme>() ); } );

/* -------------------------------------------------------------------------- */

};  // namespace nix::fetchers
