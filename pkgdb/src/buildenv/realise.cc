/* ========================================================================== *
 *
 * @file buildenv/realise.cc
 *
 * @brief Evaluate an environment definition and realise it.
 *
 *
 * -------------------------------------------------------------------------- */

#include <filesystem>
#include <fstream>

#include <nix/command.hh>
#include <nix/derivations.hh>
#include <nix/eval-inline.hh>
#include <nix/eval.hh>
#include <nix/flake/flake.hh>
#include <nix/get-drvs.hh>
#include <nix/globals.hh>
#include <nix/local-fs-store.hh>
#include <nix/path-with-outputs.hh>
#include <nix/profiles.hh>
#include <nix/shared.hh>
#include <nix/store-api.hh>
#include <nix/util.hh>
#include <nlohmann/json.hpp>

#include "flox/buildenv/realise.hh"
#include "flox/resolver/lockfile.hh"


/* -------------------------------------------------------------------------- */

namespace flox::buildenv {

/* -------------------------------------------------------------------------- */

#ifndef PROFILE_D_SCRIPTS_DIR
#  error "PROFILE_D_SCRIPTS_DIR must be set to the path of `etc/profile.d/'"
#endif

#ifndef SET_PROMPT_BASH_SH
#  error "SET_PROMPT_BASH_SH must be set to the path of `set-prompt.bash.sh'"
#endif

/* -------------------------------------------------------------------------- */

static const std::string BASH_ACTIVATE_SCRIPT = R"(
# We use --rcfile to activate using bash which skips sourcing ~/.bashrc,
# so source that here.
if [ -f ~/.bashrc -a "${FLOX_SOURCED_FROM_SHELL_RC:-}" != 1 ]
then
    source ~/.bashrc
fi

if [ -d "$FLOX_ENV/etc/profile.d" ]; then
  declare -a _prof_scripts;
  _prof_scripts=( $(
    shopt -s nullglob;
    echo "$FLOX_ENV/etc/profile.d"/*.sh;
  ) );
  for p in "${_prof_scripts[@]}"; do . "$p"; done
  unset _prof_scripts;
fi
)";


/* -------------------------------------------------------------------------- */

static const nix::StorePath
addDirToStore( nix::EvalState &    state,
               std::string const & dir,
               nix::StorePathSet & references )
{

  /* Add the symlink tree to the store. */
  nix::StringSink sink;
  dumpPath( dir, sink );

  auto narHash = hashString( nix::htSHA256, sink.s );
  nix::ValidPathInfo info {
            *state.store,
            "environment",
            nix::FixedOutputInfo {
                .method = nix::FileIngestionMethod::Recursive,
                .hash = narHash,
                .references = {
                    .others = std::move(references),
                    // profiles never refer to themselves
                    .self = false,
                },
            },
            narHash,
        };
  info.narSize = sink.s.size();

  nix::StringSource source( sink.s );
  state.store->addToStore( info, source );
  return std::move( info.path );
}


/* -------------------------------------------------------------------------- */

const nix::StorePath
createEnvironmentStorePath(
  nix::EvalState &               state,
  std::vector<RealisedPackage> & pkgs,
  nix::StorePathSet &            references,
  std::map<nix::StorePath, std::pair<std::string, resolver::LockedPackageRaw>> &
    originalPackage )
{
  /* build the profile into a tempdir */
  auto tempDir = nix::createTempDir();
  try
    {
      buildenv::buildEnvironment( tempDir, std::move( pkgs ) );
    }
  catch ( buildenv::BuildEnvFileConflictError & err )
    {
      auto [storePathA, filePath] = state.store->toStorePath( err.getFileA() );
      auto [storePathB, _]        = state.store->toStorePath( err.getFileB() );

      auto [nameA, packageA] = originalPackage.at( storePathA );
      auto [nameB, packageB] = originalPackage.at( storePathB );


      throw FloxException(
        "environment error",
        "failed to build environment",
        nix::fmt(
          "file conflict between packages '%s' and '%s' at '%s'"
          "\n\n\tresolve by setting the priority of the preferred package "
          "to a value lower than '%d'",
          nameA,
          nameB,
          filePath,
          err.getPriority() ) );
    }
  return addDirToStore( state, tempDir, references );
}

/* -------------------------------------------------------------------------- */

static nix::Attr
extractAttrPath( nix::EvalState & state,
                 nix::Value &     vFlake,
                 flox::AttrPath   attrPath )
{
  state.forceAttrs( vFlake, nix::noPos, "while parsing flake" );


  auto output = vFlake.attrs->get( state.symbols.create( "outputs" ) );

  for ( auto attrName : attrPath )
    {
      state.forceAttrs( *output->value,
                        output->pos,
                        "while parsing cached flake data" );

      auto next = output->value->attrs->get( state.symbols.create( attrName ) );

      if ( ! next )
        {
          std::ostringstream str;
          output->value->print( state.symbols, str );
          throw FloxException( "attribute `%s' not found in set `%s'",
                               attrName,
                               str.str() );
        }
      output = next;
    }

  return *output;
}


/* -------------------------------------------------------------------------- */

nix::StorePath
createFloxEnv( nix::EvalState &     state,
               resolver::Lockfile & lockfile,
               const System &       system )
{
  auto packages = lockfile.getLockfileRaw().packages.find( system );
  if ( packages == lockfile.getLockfileRaw().packages.end() )
    {
      // TODO: throw structured exception
      throw FloxException( "No packages found for system `" + system + "'" );
    }

  /* Extract all packages */
  std::vector<std::pair<std::string, resolver::LockedPackageRaw>>
    locked_packages;

  for ( auto const & package : packages->second )
    {
      if ( ! package.second.has_value() ) { continue; }
      auto const & locked_package = package.second.value();
      locked_packages.push_back( { package.first, locked_package } );
    }

  /* Extract derivations */
  nix::StorePathSet                      references;
  std::vector<nix::StorePathWithOutputs> drvsToBuild;
  std::vector<RealisedPackage>           pkgs;
  std::map<nix::StorePath, std::pair<std::string, resolver::LockedPackageRaw>>
    originalPackage;

  for ( auto const & [pId, package] : locked_packages )
    {
      // TODO: use `FloxFlake'
      auto packageInputRef = nix::FlakeRef( package.input );
      auto packageFlake    = nix::flake::lockFlake( state,
                                                 packageInputRef,
                                                 nix::flake::LockFlags {} );

      auto vFlake = state.allocValue();
      nix::flake::callFlake( state, packageFlake, *vFlake );

      /* Get referenced output. */
      auto output = extractAttrPath( state, *vFlake, package.attrPath );

      /* Interpret ooutput as derivation. */
      auto package_drv = getDerivation( state, *output.value, false );

      if ( ! package_drv.has_value() )
        {
          throw FloxException( "Failed to get derivation for package `"
                               + nlohmann::json( package ).dump() + "'" );
        }

      auto packagePath
        = state.store->printStorePath( package_drv->queryOutPath() );

      /* Collect all outputs to include in the environment.
       *
       * Set the priority of the outputs to the priority of the package
       * and the internal priority to the index of the output.
       * This way `buildenv::buildEnvironment` can resolve conflicts between
       * outputs of the same derivation. */
      for ( auto [idx, output] : enumerate( package_drv->queryOutputs() ) )
        {
          /* Skip outputs without path */
          if ( ! output.second.has_value() ) { continue; }
          pkgs.emplace_back(
            state.store->printStorePath( output.second.value() ),
            true,
            buildenv::Priority( package.priority,
                                packagePath,
                                /* idx should always fit in uint its unlikely a
                                 * package has more than 4 billion outputs. */
                                static_cast<unsigned>( idx ) ) );
          references.insert( output.second.value() );
          originalPackage.insert( { output.second.value(), { pId, package } } );
        }

      /* Collect drvs that may yet need to be built. */
      if ( auto drvPath = package_drv->queryDrvPath() )
        {
          drvsToBuild.push_back( nix::StorePathWithOutputs { *drvPath, {} } );
        }
    }

  // TODO: check if this builds `outputsToInstall` only
  // TODO: do we need to honor repair flag? state.repair ? bmRepair : bmNormal
  /* Build derivations that make up the environment */
  state.store->buildPaths( nix::toDerivedPaths( drvsToBuild ) );

  /* verbatim content of the activate script common to all shells */
  std::stringstream commonActivate;

  auto tempDir = std::filesystem::path( nix::createTempDir() );
  std::filesystem::create_directories( tempDir / "activate" );

  /* Add environment variables.
   * Read environment variables from the manifest and add them as exports to the
   * activate script. */
  if ( auto vars = lockfile.getManifest().getManifestRaw().vars )
    {

      for ( auto [name, value] : vars.value() )
        {
          /* Single quote value and replace ' with '\''.
           *
           * This is the same as what nixpkgs.lib.escapeShellArg does.
           * to disable these variables dynamically expanding at runtime.
           *
           * 'foo''\\''bar' is evaluated as  foo'bar  in bash/zsh*/
          size_t i = 0;
          while ( ( i = value.find( "'", i ) ) != std::string::npos )
            {
              value.replace( i, 1, "'\\''" );
              i += 4;
            }

          commonActivate << nix::fmt( "export %s='%s'", name, value )
                         << std::endl;
        }
    }

  /* Add hook script.
   * Write hook script to a temporary file and copy it to the environment.
   * Add source command to the activate script. */
  // TODO: is it script _xor_ file?
  // Currently it is assumed that `hook.script` and `hook.file` are
  // mutually exclusive.
  // If both are set, `hook.file` takes precedence.
  if ( auto hook = lockfile.getManifest().getManifestRaw().hook )
    {
      nix::Path script_path;

      /* Either set script path to a temporary file. */
      if ( auto script = hook->script )
        {
          script_path = nix::createTempFile().second;
          std::ofstream file( script_path );
          file << script.value();
          file.close();
        }

      /* ...Or to the file specified in the manifest. */
      if ( auto file = hook->file ) { script_path = file.value(); }

      if ( ! script_path.empty() )
        {

          std::filesystem::copy_file( script_path,
                                      tempDir / "activate" / "hook.sh" );
          std::filesystem::permissions( tempDir / "activate" / "hook.sh",
                                        std::filesystem::perms::owner_exec,
                                        std::filesystem::perm_options::add );
          commonActivate << "source \"$FLOX_ENV/activate/hook.sh\""
                         << std::endl;
        }
    }

  /* Add bash activation script. */
  std::ofstream bashActivate( tempDir / "activate" / "bash" );
  /* If this gets bigger, we could factor this out into a file that gets
   * sourced, like we do for zsh. */
  bashActivate << BASH_ACTIVATE_SCRIPT << "\n";
  bashActivate << "source " << SET_PROMPT_BASH_SH << "\n";
  bashActivate << commonActivate.str();
  bashActivate.close();

  /* Add zsh activation script.
   * Functionality shared between all environments is
   * in `flox.zdotdir/.zshrc'. */
  std::ofstream zshActivate( tempDir / "activate" / "zsh" );
  zshActivate << commonActivate.str();
  zshActivate.close();

  auto activationStorePath
    = state.store->addToStore( "activation-scripts", tempDir );
  references.insert( activationStorePath );
  pkgs.emplace_back( state.store->printStorePath( activationStorePath ),
                     true,
                     buildenv::Priority() );

  /* Insert profile.d scripts.
   * The store path is provided at compile time via the `PROFILE_D_SCRIPTS_DIR'
   * environment variable. */
  auto profileScriptsPath
    = state.store->parseStorePath( PROFILE_D_SCRIPTS_DIR );
  state.store->ensurePath( profileScriptsPath );
  references.insert( profileScriptsPath );
  pkgs.emplace_back( state.store->printStorePath( profileScriptsPath ),
                     true,
                     buildenv::Priority() );

  return createEnvironmentStorePath( state, pkgs, references, originalPackage );
}


/* -------------------------------------------------------------------------- */

}  // namespace flox::buildenv


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
