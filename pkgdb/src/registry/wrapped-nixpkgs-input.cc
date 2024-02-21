/* ========================================================================== *
 *
 * @file registry/wrapped-nixpkgs-input.cc
 *
 * @brief Fetches a `nix` input and applies a patch before evaluation.
 *
 *
 * -------------------------------------------------------------------------- */

#include <nix/fetchers.hh>
#include <nix/url-parts.hh>

#include "flox/core/exceptions.hh"


/* -------------------------------------------------------------------------- */

namespace flox {

/* -------------------------------------------------------------------------- */

struct DownloadUrl
{
  std::string  url;
  nix::Headers headers;
};


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

  std::pair<nix::StorePath, nix::fetchers::Input>
  fetch( nix::ref<nix::Store>         store,
         const nix::fetchers::Input & _input ) override;

  nix::Hash
  getRevFromRef( nix::ref<nix::Store>         store,
                 const nix::fetchers::Input & input ) const;

  void
  clone( const nix::fetchers::Input & input,
         const nix::Path &            destDir ) const override;


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
           && ( name != "narHash" ) && ( name != "rulesVersion" )
           && ( name != "lastModified" ) )
        {
          throw nix::Error( "unsupported flox-nixpkgs input attribute '%s'",
                            name );
        }
    }

  /* Type check the following fields if they exist. */
  nix::fetchers::maybeGetStrAttr( attrs, "narHash" );
  nix::fetchers::maybeGetIntAttr( attrs, "rulesVersion" );
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
  if ( url.scheme != type() ) { return std::nullopt; }

  nix::fetchers::Input input;

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
      input.attrs["rulesVersion"]
        = static_cast<uint64_t>( atoll( path[0].c_str() + 1 ) );
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
      input.attrs["rev"] = path[1];
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
      input.attrs["ref"] = path[1];
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

}  // namespace flox


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
