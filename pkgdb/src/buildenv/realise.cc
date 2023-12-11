/* ========================================================================== *
 *
 * @file buildenv/realise.cc
 *
 * @brief Evaluate an environment definition and realise it.
 *
 *
 * -------------------------------------------------------------------------- */

#include <fstream>
#include <filesystem>

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

#include "flox/resolver/lockfile.hh"
#include "flox/buildenv/realise.hh"
#include "flox/resolver/lockfile.hh"


/* -------------------------------------------------------------------------- */

namespace flox::buildenv {

/* -------------------------------------------------------------------------- */

#ifndef PROFILE_D_SCRIPT_DIR
#  define PROFILE_D_SCRIPT_DIR "invalid_profile.d_script_path"
#endif

#ifndef SET_PROMPT_BASH_SH
#  define SET_PROMPT_BASH_SH "invalid_set-prompt-bash.sh_path"
#endif


/* -------------------------------------------------------------------------- */

static const std::string BASH_ACTIVATE_SCRIPT = R"(
# We use --rcfile to activate using bash which skips sourcing ~/.bashrc,
# so source that here.
if [ -f ~/.bashrc ]
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
addDirToStore( nix::EvalState &         state,
               std::string const &        dir,
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
                .method = FileIngestionMethod::Recursive,
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
  nix::EvalState &           state,
  std::vector<Package> & pkgs,
  nix::StorePathSet &        references,
  std::map<StorePath, std::pair<std::string, resolver::LockedPackageRaw>> &
    originalPackage )
{
  /* build the profile into a tempdir */
  auto tempDir = createTempDir();
  try
    {
      buildenv::buildEnvironment( tempDir, std::move( pkgs ) );
    }
  catch ( buildenv::BuildEnvFileConflictError & e )
    {
      auto [storePathA, filePath] = state.store->toStorePath( e.fileA );
      auto [storePathB, _]        = state.store->toStorePath( e.fileB );

      auto [nameA, packageA] = originalPackage.at( storePathA );
      auto [nameB, packageB] = originalPackage.at( storePathB );


      throw FloxException(
        "environment error",
        "failed to build environment",
        fmt( "file conflict between packages '%s' and '%s' at '%s'"
             "\n\n\tresolve by setting the priority of the preferred package "
             "to a value lower than '%d'",
             nameA,
             nameB,
             filePath,
             e.priority ) );
    }
  return addDirToStore( state, tempDir, references );
}

/* -------------------------------------------------------------------------- */

static nix::Attr
extractAttrPath( nix::EvalState & state,
                 nix::Value &     vFlake,
                 flox::AttrPath   attrPath )
{

  state.forceAttrs( vFlake, noPos, "while parsing flake" );


  auto output = vFlake.attrs->get( state.symbols.create( "outputs" ) );

  for ( auto path_segment : attrPath )
    {
      state.forceAttrs( *output->value,
                        output->pos,
                        "while parsing cached flake data" );

      auto next
        = output->value->attrs->get( state.symbols.create( path_segment ) );

      if ( ! next )
        {
          std::ostringstream str;
          output->value->print( state.symbols, str );
          throw Error( "Attribute '%s' not found in set '%s'",
                       path_segment,
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
               System &             system )
{
  auto packages = lockfile.getLockfileRaw().packages.find( system );
  if ( packages == lockfile.getLockfileRaw().packages.end() )
    {
      // TODO: throw structured exception
      throw Error( "No packages found for system '%s'", system );
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
  nix::StorePathSet                 references;
  std::vector<StorePathWithOutputs> drvsToBuild;
  std::vector<Package>              pkgs;
  std::map<StorePath, std::pair<std::string, resolver::LockedPackageRaw>>
    originalPackage;

  for ( auto const & [pId, package] : locked_packages )
    {

      auto package_input_ref = FlakeRef( package.input );
      auto package_flake
        = flake::lockFlake( state, package_input_ref, flake::LockFlags {} );

      auto vFlake = state.allocValue();
      flake::callFlake( state, package_flake, *vFlake );

      /* Get referenced output. */
      auto output = extractAttrPath( state, *vFlake, package.attrPath );

      /* Interpret ooutput as derivation. */
      auto package_drv = getDerivation( state, *output.value, false );

      if ( ! package_drv.has_value() )
        {
          throw Error( "Failed to get derivation for package '%s'",
                       nlohmann::json( package ).dump().c_str() );
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
            buildenv::Priority {
              package.priority,
              packagePath,
              /* idx should always fit in uint its unlikely a package has more
               * than 4 billion outputs. */
              static_cast<unsigned>( idx ),
            } );
          references.insert( output.second.value() );
          originalPackage.insert( { output.second.value(), { pId, package } } );
        }

      /* Collect drvs that may yet need to be built. */
      if ( auto drvPath = package_drv->queryDrvPath() )
        {
          drvsToBuild.push_back( { *drvPath } );
        }
    }

  // TODO: check if this builds `outputsToInstall` only
  // TODO: do we need to honor repair flag? state.repair ? bmRepair : bmNormal
  /* Build derivations that make up the environment */
  state.store->buildPaths( toDerivedPaths( drvsToBuild ) );

  /* verbatim content of the activate script common to all shells */
  std::stringstream commonActivate;

  auto tempDir = std::filesystem::path( createTempDir() );
  std::filesystem::create_directories( tempDir / "activate" );

  /* Add hook script
   *
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
          script_path = createTempFile().second;
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
                         << "\n";
        }
    }

  /* Add environment variables.
   * Read environment variables from the manifest and add them as exports to the
   * activate script. */
  if ( auto vars = lockfile.getManifest().getManifestRaw().vars )
    {

      for ( auto [name, value] : vars.value() )
        {
          /* Double quote value and replace " with \".
           * Note that we could instead do something similar to what
           * nixpkgs.lib.escapeShellArg does to disable these variables
           * dynamically expanding at runtime. */
          size_t i = 0;
          while ( ( i = value.find( "\"", i ) ) != std::string::npos )
            {
              value.replace( i, 1, "\\\"" );
              i += 2;
            }

          commonActivate << fmt( "export %s=\"%s\"", name, value ) << "\n";
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
   * Functionality shared between all environments is in flox.zdotdir/.zshrc. */
  std::ofstream zshActivate( tempDir / "activate" / "zsh" );
  zshActivate << commonActivate.str();
  zshActivate.close();

  auto activation_store_path
    = state.store->addToStore( "activation-scripts", tempDir );
  references.insert( activation_store_path );
  pkgs.emplace_back( state.store->printStorePath( activation_store_path ),
                     true,
                     buildenv::Priority { 0 } );

  /* Insert profile.d scripts.
   * The store path is provided at compile time via the `PROFILE_D_SCRIPT_DIR'
   * environment variable.
   * See also: `./pkgs/flox-env-builder/default.nix` */
  auto profile_d_scripts_path
    = state.store->parseStorePath( PROFILE_D_SCRIPT_DIR );
  state.store->ensurePath( profile_d_scripts_path );
  references.insert( profile_d_scripts_path );
  pkgs.emplace_back( state.store->printStorePath( profile_d_scripts_path ),
                     true,
                     buildenv::Priority { 0 } );

  return createEnvironmentStorePath( state, pkgs, references, originalPackage );
}


/* -------------------------------------------------------------------------- */

void
buildEnvironment( const std::string & out, std::vector<Package> & pkgs );


/* -------------------------------------------------------------------------- */

}  // namespace flox::buildenv


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
