/* ========================================================================== *
 *
 * @file flox/buildenv/buildenv-lockfile.hh
 *
 * @brief The subset of a lockfile that buildenv needs in order to build an
 *        environment.
 *
 * -------------------------------------------------------------------------- */

#include "flox/buildenv/buildenv-lockfile.hh"
#include "flox/core/util.hh"
#include "flox/fetchers/wrapped-nixpkgs-input.hh"
#include "flox/lock-flake-installable.hh"
#include "flox/resolver/descriptor.hh"
#include "flox/resolver/lockfile.hh"

/* -------------------------------------------------------------------------- */

namespace flox::buildenv {

/* -------------------------------------------------------------------------- */

void
BuildenvLockfile::load_from_content( const nlohmann::json & jfrom )
{
  unsigned version = jfrom["lockfile-version"];
  debugLog( nix::fmt( "lockfile version %d", version ) );

  switch ( version )
    {
      case 0: this->from_v0_content( jfrom ); break;
      case 1: this->from_v1_content( jfrom ); break;
      default:
        throw resolver::InvalidLockfileException(
          "unsupported lockfile version",
          "only v0 and v1 are supprted" );
    }
}

/* -------------------------------------------------------------------------- */

void
BuildenvLockfile::from_v0_content( const nlohmann::json & jfrom )
{
  resolver::LockfileRaw lockfileRaw = resolver::LockfileRaw();
  jfrom.get_to( lockfileRaw );
  this->manifest = lockfileRaw.manifest;
  for ( auto [system, systemPackages] : lockfileRaw.packages )
    {
      for ( auto [installId, lockedPackage] : systemPackages )
        {
          if ( lockedPackage.has_value() )
            {
              resolver::LockedInputRaw input = resolver::LockedInputRaw();
              input.attrs = flox::githubAttrsToFloxNixpkgsAttrs(
                lockedPackage->input.attrs );
              input.url = nix::FlakeRef::fromAttrs( input.attrs ).to_string();

              this->packages.emplace_back(
                BuildenvLockedPackage { system,
                                        installId,
                                        input,
                                        lockedPackage->attrPath,
                                        lockedPackage->priority } );
            }
        }
    }
}


/* -------------------------------------------------------------------------- */

/* Convert URLs of the form
 * https://github.com/flox/nixpkgs?rev=XXX
 * to the form
 * github:flox/nixpkgs/XXX
 */
resolver::LockedInputRaw
nixpkgsHttpsToGithubInput( std::string locked_url )
{
  resolver::LockedInputRaw githubInput = resolver::LockedInputRaw();

  if ( std::string supportedUrl = "https://github.com/flox/nixpkgs";
       locked_url.substr( 0, supportedUrl.size() ) != supportedUrl )
    {
      throw resolver::InvalidLockfileException(
        "unsupported lockfile URL for v1 lockfile",
        "must begin with " + supportedUrl );
    }
  /* Copy rev and ref if they exist */
  auto httpsAttrs = nix::parseFlakeRef( locked_url ).toAttrs();
  if ( auto rev = nix::fetchers::maybeGetStrAttr( httpsAttrs, "rev" ) )
    {
      githubInput.attrs["rev"] = rev;
    };
  if ( auto ref = nix::fetchers::maybeGetStrAttr( httpsAttrs, "ref" ) )
    {
      githubInput.attrs["ref"] = ref;
    };
  httpsAttrs.erase( "ref" );
  httpsAttrs.erase( "rev" );

  /* We've already checked these are correct values with the supportedUrl check
   */
  githubInput.attrs["type"]  = "github";
  githubInput.attrs["owner"] = "flox";
  githubInput.attrs["repo"]  = "nixpkgs";
  httpsAttrs.erase( "type" );
  httpsAttrs.erase( "url" );

  /* Throw if there's anything in the URL that can't be converted from a git to
   * a github flakeref (see GitInputScheme:allowedAttrs for an exhaustive
   * list) */
  if ( ! httpsAttrs.empty() )
    {
      throw resolver::InvalidLockfileException(
        "unsupported lockfile URL for v1 lockfile: '" + locked_url
        + "' contains attributes other than 'url', 'ref', and 'rev'" );
    }

  githubInput.url = nix::FlakeRef::fromAttrs( githubInput.attrs ).to_string();

  return githubInput;
}


/* -------------------------------------------------------------------------- */

static void
buildenvPackageFromV1Descriptor( const nlohmann::json &  jfrom,
                                 std::string &&          installId,
                                 std::string &&          system,
                                 BuildenvLockedPackage & pkg )
{
  pkg.installId = installId;
  pkg.system    = system;

  // Catalog packages don't come from a flake context so only have attr-path.
  // Flake packages will always have locked-flake-attr-path.
  // For now, use this to differentiate between the two.
  if ( jfrom.contains( "locked-flake-attr-path" ) )
    {
      LockedInstallable lockedInstallable = LockedInstallable();
      jfrom.get_to( lockedInstallable );
      pkg.attrPath  = splitAttrPath( lockedInstallable.lockedFlakeAttrPath );
      pkg.priority  = jfrom["priority"];
      pkg.input     = resolver::LockedInputRaw();
      pkg.input.url = lockedInstallable.lockedUrl;
      pkg.input.attrs
        = nix::parseFlakeRef( lockedInstallable.lockedUrl ).toAttrs();
    }
  else
    {
      // We assume that all v1 catalog descriptors are from nixpkgs,
      // so we should
      // 1. Prepend `legacyPackages.system` to attrPath
      // 2. Wrap with our custom flox-nixpkgs fetcher
      std::string attrPath = jfrom["attr_path"];
      pkg.attrPath
        = splitAttrPath( "legacyPackages." + system + "." + attrPath );

      pkg.priority = jfrom["priority"];

      // Set `input` to a flox-nixpkgs input
      pkg.input              = resolver::LockedInputRaw();
      std::string locked_url = jfrom["locked_url"];
      // Convert first from https to github and then to flox-nixpkgs
      // TODO: do this in one hop instead of two
      pkg.input       = nixpkgsHttpsToGithubInput( locked_url );
      pkg.input.attrs = flox::githubAttrsToFloxNixpkgsAttrs( pkg.input.attrs );
      pkg.input.url   = nix::FlakeRef::fromAttrs( pkg.input.attrs ).to_string();
    }
}


/* -------------------------------------------------------------------------- */

void
BuildenvLockfile::from_v1_content( const nlohmann::json & jfrom )
{
  debugLog( nix::fmt( "loading v1 lockfile content" ) );

  unsigned version = jfrom["lockfile-version"];
  if ( version != 1 )
    {
      throw resolver::InvalidLockfileException(
        nix::fmt( "trying to parse v%d lockfile", version ),
        "expected v1" );
    }

  // load vars
  try
    {
      auto value = jfrom["manifest"]["vars"];
      value.get_to( this->manifest.vars );
    }
  catch ( nlohmann::json::exception & err )
    {
      throw resolver::InvalidLockfileException(
        "couldn't parse lockfile field 'manifest.vars'",
        extract_json_errmsg( err ) );
    }

  // load hooks
  try
    {
      auto hook = jfrom["manifest"]["hook"];
      hook.get_to( this->manifest.hook );
    }
  catch ( nlohmann::json::exception & err )
    {
      throw resolver::InvalidLockfileException(
        "couldn't parse lockfile field 'manifest.hook'",
        extract_json_errmsg( err ) );
    }

  // load profile
  try
    {
      auto hook = jfrom["manifest"]["profile"];
      hook.get_to( this->manifest.profile );
    }
  catch ( nlohmann::json::exception & err )
    {
      throw resolver::InvalidLockfileException(
        "couldn't parse lockfile field 'manifest.profile'",
        extract_json_errmsg( err ) );
    }

  // load packages
  try
    {
      auto packages = jfrom["packages"];
      for ( const auto & [idx, package] : packages.items() )
        {
          /* Deserialize things we want pretty errors for here so the error can
           * include idx */
          std::string installId;
          std::string system;
          try
            {
              installId = package["install_id"];
            }
          catch ( nlohmann::json::exception & err )
            {
              throw resolver::InvalidLockfileException(
                "couldn't parse lockfile field 'packages[" + idx
                  + "].install_id'",
                extract_json_errmsg( err ) );
            }

          try
            {
              system = package["system"];
            }
          catch ( nlohmann::json::exception & err )
            {
              throw resolver::InvalidLockfileException(
                "couldn't parse lockfile field 'packages[" + idx + "].system'",
                extract_json_errmsg( err ) );
            }

          BuildenvLockedPackage pkg = BuildenvLockedPackage();
          try
            {
              buildenvPackageFromV1Descriptor( package,
                                               std::move( installId ),
                                               std::move( system ),
                                               pkg );
            }
          catch ( nlohmann::json::exception & err )
            {
              throw resolver::InvalidLockfileException(
                "couldn't parse 'packages[" + idx + "]'",
                extract_json_errmsg( err ) );
            }


          this->packages.emplace_back( pkg );
        }
    }
  catch ( nlohmann::json::exception & err )
    {
      throw resolver::InvalidLockfileException(
        "couldn't parse lockfile field 'packages'",
        extract_json_errmsg( err ) );
    }

  // load options
  try
    {
      auto options = jfrom["manifest"]["options"];
      options.get_to( this->manifest.options );
    }
  catch ( nlohmann::json::exception & err )
    {
      throw resolver::InvalidLockfileException(
        "couldn't parse lockfile field 'manifest.options'",
        extract_json_errmsg( err ) );
    }

  // load build scripts
  try
    {
      auto build = jfrom["manifest"]["build"];
      build.get_to( this->manifest.build );
    }
  catch ( nlohmann::json::exception & err )
    {
      throw resolver::InvalidLockfileException(
        "couldn't parse lockfile field 'manifest.build'",
        extract_json_errmsg( err ) );
    }

  debugLog( nix::fmt( "loaded lockfile v1" ) );
}


}  // namespace flox::buildenv


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
