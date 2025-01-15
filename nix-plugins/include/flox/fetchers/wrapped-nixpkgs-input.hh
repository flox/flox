/* ========================================================================== *
 *
 * @file flox/fetchers/wrapped-nixpkgs-input.hh
 *
 * @brief Executable command helpers, argument parsers, etc.
 *
 *
 * -------------------------------------------------------------------------- */

#pragma once
#include <nix/fetchers.hh>

/* -------------------------------------------------------------------------- */

namespace flox {

/**
 * @brief Helper used to convert a `github` attribute set representation,
 *        to a `flox-nixpkgs` attribute set representation.
 */
nix::fetchers::Attrs
githubAttrsToFloxNixpkgsAttrs( const nix::fetchers::Attrs & attrs );


/* -------------------------------------------------------------------------- */

/** @brief Fetches a `nixpkgs` input and wraps it with a few modifications. */
struct WrappedNixpkgsInputScheme : nix::fetchers::InputScheme
{

  [[nodiscard]] virtual std::string_view
  schemeName() const override
  {
    return "flox-nixpkgs";
  }

  /**
   * Allowed attributes in an attribute set that is converted to an
   * input.
   *
   * `type` is not included from this set, because the `type` field is
   *  parsed first to choose which scheme; `type` is always required.
   */
  virtual nix::StringSet
  allowedAttrs() const override
  {
    return { "version", "rev", "ref" };
  }


  /** @brief Convert raw attributes into an input. */
  [[nodiscard]] std::optional<nix::fetchers::Input>
  inputFromAttrs( const nix::fetchers::Settings & settings,
                  const nix::fetchers::Attrs &    attrs ) const override;

  /** @brief Convert a URL string into an input. */
  [[nodiscard]] std::optional<nix::fetchers::Input>
  inputFromURL( const nix::fetchers::Settings & settings,
                const nix::ParsedURL &          url,
                bool                            requireTree ) const override;

  /** @brief Convert input to a URL representation. */
  [[nodiscard]] nix::ParsedURL
  toURL( const nix::fetchers::Input & input ) const override;

  /**
   * Return `true` if this input is considered "locked", i.e. it has
   * attributes like a Git revision or NAR hash that uniquely
   * identify its contents.
   */
  virtual bool
  isLocked( const nix::fetchers::Input & input ) const override;

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

  [[nodiscard]] virtual std::pair<nix::ref<nix::SourceAccessor>,
                                  nix::fetchers::Input>
  getAccessor( nix::ref<nix::Store>         store,
               const nix::fetchers::Input & input ) const override;


}; /* End class `WrappedNixpkgsInputScheme' */
}  // namespace flox
