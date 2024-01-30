/* ========================================================================== *
 *
 * @file flox/registry/floxpkgs.hh
 *
 * @brief Provides a specialized `FloxFlake' which applies rules/pre-processing
 *        to a `flake' before it is evaluated.
 *        This is used to implement the `floxpkgs' catalog.
 *
 *
 * -------------------------------------------------------------------------- */

#pragma once

#include "flox/flox-flake.hh"
#include <regex>


/* -------------------------------------------------------------------------- */

namespace flox {

/* -------------------------------------------------------------------------- */

struct DownloadUrl
{
  std::string  url;
  nix::Headers headers;
};


/* -------------------------------------------------------------------------- */

struct GitArchiveInputScheme : nix::fetchers::InputScheme
{
  virtual std::string
  type() const
    = 0;

  virtual std::optional<std::pair<std::string, std::string>>
  accessHeaderFromToken( const std::string & token ) const = 0;

  std::optional<nix::fetchers::Input>
  inputFromURL( const nix::ParsedURL & url ) const override;

  std::optional<nix::fetchers::Input>
  inputFromAttrs( const nix::fetchers::Attrs & attrs ) const override;

  nix::ParsedURL
  toURL( const nix::fetchers::Input & input ) const override;

  bool
  hasAllInfo( const nix::fetchers::Input & input ) const override;

  nix::fetchers::Input
  applyOverrides( const nix::fetchers::Input & _input,
                  std::optional<std::string>   ref,
                  std::optional<nix::Hash>     rev ) const override;

  std::optional<std::string>
  getAccessToken( const std::string & host ) const;

  nix::Headers
  makeHeadersWithAuthTokens( const std::string & host ) const;

  virtual nix::Hash
  getRevFromRef( nix::ref<nix::Store>         store,
                 const nix::fetchers::Input & input ) const
    = 0;

  virtual DownloadUrl
  getDownloadUrl( const nix::fetchers::Input & input ) const
    = 0;

  std::pair<nix::StorePath, nix::fetchers::Input>
  fetch( nix::ref<nix::Store>         store,
         const nix::fetchers::Input & _input ) override;
};

struct GitHubInputScheme : GitArchiveInputScheme
{
  virtual std::string
  type() const override;

  std::optional<std::pair<std::string, std::string>>
  accessHeaderFromToken( const std::string & token ) const override;

  std::string
  getHost( const nix::fetchers::Input & input ) const;

  std::string
  getOwner( const nix::fetchers::Input & input ) const;

  std::string
  getRepo( const nix::fetchers::Input & input ) const;

  nix::Hash
  getRevFromRef( nix::ref<nix::Store>         store,
                 const nix::fetchers::Input & input ) const override;

  DownloadUrl
  getDownloadUrl( const nix::fetchers::Input & input ) const override;

  void
  clone( const nix::fetchers::Input & input,
         const nix::Path &            destDir ) const override;
};


/* -------------------------------------------------------------------------- */

/**
 * @brief A fetcher that wraps a nixpkgs flake in a wrapper flake to apply
 * allow/disallow/alias rules.
 */
struct FloxFlakeScheme : GitHubInputScheme
{
  std::string
  type() const override;

  std::optional<nix::fetchers::Input>
  inputFromURL( const nix::ParsedURL & url ) const override;

  std::optional<nix::fetchers::Input>
  inputFromAttrs( const nix::fetchers::Attrs & attrs ) const override;

  std::pair<nix::StorePath, nix::fetchers::Input>
  fetch( nix::ref<nix::Store>         store,
         const nix::fetchers::Input & input ) override;

  bool
  hasAllInfo( const nix::fetchers::Input & ) const override;

  nix::ParsedURL
  toURL( const nix::fetchers::Input & input ) const override;
};


/* -------------------------------------------------------------------------- */

[[nodiscard]] std::filesystem::path
createWrappedFlakeDir( const nix::FlakeRef & nixpkgsRef );


/* -------------------------------------------------------------------------- */

/** @brief Computes the hash of the rules file. */
[[nodiscard]] std::string
getRulesHash();

/** @brief Computes the hash of the rules processor. */
[[nodiscard]] std::string
getRulesProcessorHash();

/** @brief Computes the nar has for a wrapped flake. */
[[nodiscard]] std::string
getWrappedFlakeNarHash( nix::FlakeRef const & ref );


/* -------------------------------------------------------------------------- */

/** @brief A container holding all of the attributes injected by our fetcher. */
struct OurAttrs
{
  std::optional<std::string> rules;
  std::optional<std::string> rulesProcessor;

  OurAttrs( const std::optional<std::string> & rules,
            const std::optional<std::string> & rulesProcessor )
    : rules( rules ), rulesProcessor( rulesProcessor ) {};
};

/** @brief Removes our fetcher-specific attributes from `attrs` and returns them
 * so they can be restored later.*/
[[nodiscard]] OurAttrs
removeOurInputAttrs( nix::fetchers::Attrs & attrs );

/** @brief Set all of our attrs in `attrs`, overwriting any previous values for
 * those attributes.*/
void
restoreOurInputAttrs( nix::fetchers::Attrs & attrs, const OurAttrs & fields );

/** @brief Converts a flox-nixpkgs attrset to a GitHub attrset, returning both
 * the stripped attrset and the attrs that were removed.*/
[[nodiscard]] std::pair<nix::fetchers::Attrs, OurAttrs>
toGitHubAttrs( const nix::fetchers::Attrs & attrs );

/** @brief Converts a GitHub attrset to a flox-nixpkgs attrset. */
[[nodiscard]] nix::fetchers::Attrs
fromGitHubAttrs( const nix::fetchers::Attrs & attrs,
                 const OurAttrs &             ourAttrs );


/* -------------------------------------------------------------------------- */

}  // namespace flox


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
