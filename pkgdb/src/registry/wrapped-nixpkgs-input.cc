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
      // TODO: Create an exception for this input
      throw FloxException(
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
      return {};
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

  nix::fetchers::maybeGetIntAttr( attrs, "rulesVersion" );

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

}  // namespace flox


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
