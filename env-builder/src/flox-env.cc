#include "flox/flox-env.hh"
#include <boost/algorithm/string/join.hpp>
#include <filesystem>
#include <flox/resolver/lockfile.hh>
#include <fstream>
#include <nix/builtins/buildenv.hh>
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

#ifndef PROFILE_D_SCRIPT_DIR
#  define PROFILE_D_SCRIPT_DIR "invalid_profile.d_script_path"
#endif

#ifndef SET_PROMPT_BASH_SH
#  define SET_PROMPT_BASH_SH "invalid_set-prompt-bash.sh_path"
#endif

/* -------------------------------------------------------------------------- */

namespace flox {
using namespace nix;
using namespace flox::resolver;

/* -------------------------------------------------------------------------- */

const nix::StorePath
addDirToStore( EvalState &         state,
               Path const &        dir,
               nix::StorePathSet & references )
{

  /* Add the symlink tree to the store. */
  StringSink sink;
  dumpPath( dir, sink );

  auto narHash = hashString( htSHA256, sink.s );
  ValidPathInfo info {
            *state.store,
            "environment",
            FixedOutputInfo {
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

  StringSource source( sink.s );
  state.store->addToStore( info, source );
  return std::move( info.path );
}

const nix::StorePath
createEnvironmentStorePath(
  nix::EvalState &    state,
  nix::Packages &     pkgs,
  nix::StorePathSet & references,
  std::map<StorePath, std::pair<std::string, resolver::LockedPackageRaw>> &
    originalPackage )
{
  /* build the profile into a tempdir */
  auto tempDir = createTempDir();
  try
    {
      buildProfile( tempDir, std::move( pkgs ) );
    }
  catch ( BuildEnvFileConflictError & e )
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


StorePath
createFloxEnv( EvalState &          state,
               resolver::Lockfile & lockfile,
               System &             system )
{


  auto packages = lockfile.getLockfileRaw().packages.find( system );
  if ( packages == lockfile.getLockfileRaw().packages.end() )
    {
      throw Error( "No packages found for system '%s'", system );
    }

  /* extract all packages */

  std::vector<std::pair<std::string, resolver::LockedPackageRaw>>
    locked_packages;

  for ( auto const & package : packages->second )
    {
      if ( ! package.second.has_value() ) { continue; }
      auto const & locked_package = package.second.value();
      locked_packages.push_back( { package.first, locked_package } );
    }

  /**
   * extract derivations
   */
  StorePathSet                      references;
  std::vector<StorePathWithOutputs> drvsToBuild;
  Packages                          pkgs;
  std::map<StorePath, std::pair<std::string, resolver::LockedPackageRaw>>
    originalPackage;

  for ( auto const & [name, package] : locked_packages )
    {

      auto package_input_ref = FlakeRef( package.input );
      auto package_flake
        = flake::lockFlake( state, package_input_ref, flake::LockFlags {} );

      auto vFlake = state.allocValue();
      flake::callFlake( state, package_flake, *vFlake );
      state.forceAttrs( *vFlake, noPos, "while parsing flake" );


      auto output = vFlake->attrs->get( state.symbols.create( "outputs" ) );


      /* evaluate the package */
      for ( auto path_segment : package.attrPath )
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


      auto package_drv = getDerivation( state, *output->value, false );

      if ( ! package_drv.has_value() )
        {
          throw Error( "Failed to get derivation for package '%s'",
                       nlohmann::json( package ).dump().c_str() );
        }

      /* Collect all outputs to include in the environment */
      for ( auto output : package_drv->queryOutputs() )
        {
          if ( ! output.second.has_value() )
            {
              continue;
            }  // skip outputs without path
          pkgs.emplace_back(
            state.store->printStorePath( output.second.value() ),
            true,
            package.priority );
          references.insert( output.second.value() );
          originalPackage.insert(
            { output.second.value(), { name, package } } );
        }

      /* Collect drvs that may yet need to be built */
      if ( auto drvPath = package_drv->queryDrvPath() )
        {
          drvsToBuild.push_back( { *drvPath } );
        }
    }

  /* Build derivations that make up the environment */
  // todo: check if this builds `outputsToInstall` only
  // todo: do we need to honor repair flag? state.repair ? bmRepair : bmNormal
  state.store->buildPaths( toDerivedPaths( drvsToBuild ) );

  std::stringstream commonActivate;

  auto tempDir = std::filesystem::path( createTempDir() );
  std::filesystem::create_directories( tempDir / "activate" );

  /* Add hook */
  // todo: is it script _xor_ file?
  //       currently it is assumed that `hook.script` and `hook.file` are
  //       mutually exclusive
  if ( auto hook = lockfile.getManifest().getManifestRaw().hook )
    {
      nix::Path script_path;

      // either set script path to a temporary file
      if ( auto script = hook->script )
        {
          script_path = createTempFile().second;
          std::ofstream file( script_path );
          file << script.value();
          file.close();
        }

      // ... or to the file specified in the manifest
      if ( auto file = hook->file ) { script_path = file.value(); }

      if ( ! script_path.empty() )
        {

          std::filesystem::copy_file( script_path,
                                      tempDir / "activate" / "hook.sh" );
          std::filesystem::permissions( tempDir / "activate" / "hook.sh",
                                        std::filesystem::perms::owner_exec,
                                        std::filesystem::perm_options::add );

          commonActivate << "\nsource \"$FLOX_ENV/activate/hook.sh\"\n";
        }
    }

  /* Add environment variables */
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

          commonActivate << fmt( "export %s=\"%s\"\n", name, value );
        }
    }

  /* Add bash activation script. */
  std::ofstream bashActivate( tempDir / "activate" / "bash" );
  /* If this gets bigger, we could factor this out into a file that gets
   * sourced, like we do for zsh. */
  bashActivate << R"(
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
  bashActivate << "\nsource " << SET_PROMPT_BASH_SH << "\n";
  bashActivate << commonActivate.str();
  bashActivate.close();

  /* Add zsh activation script. Functionality shared between all environments is
   * in flox.zdotdir/.zshrc. */
  std::ofstream zshActivate( tempDir / "activate" / "zsh" );
  zshActivate << commonActivate.str();
  zshActivate.close();

  auto activation_store_path
    = state.store->addToStore( "activation-scripts", tempDir );
  references.insert( activation_store_path );
  pkgs.emplace_back( state.store->printStorePath( activation_store_path ),
                     true,
                     0 );


  /* Add profile.d scripts */
  auto profile_d_scripts_path
    = state.store->parseStorePath( PROFILE_D_SCRIPT_DIR );
  state.store->ensurePath( profile_d_scripts_path );
  references.insert( profile_d_scripts_path );
  pkgs.emplace_back( state.store->printStorePath( profile_d_scripts_path ),
                     true,
                     0 );

  return createEnvironmentStorePath( state, pkgs, references, originalPackage );
}


struct CmdBuildEnv : nix::EvalCommand
{
  std::string              lockfile_content;
  std::optional<nix::Path> out_link;

  CmdBuildEnv()
  {
    addFlag( { .longName    = "lockfile",
               .shortName   = 'l',
               .description = "locked manifest",
               .labels      = { "lockfile" },
               .handler     = { &lockfile_content } } );

    addFlag( { .longName    = "out-link",
               .shortName   = 'o',
               .description = "output link",
               .labels      = { "out-link" },
               .handler     = { &out_link } } );
  }

  std::string
  description() override
  {
    return "build flox env";
  }

  std::string
  doc() override
  {
    return "TODO";
  }


  void
  run( ref<Store> store ) override
  {

    logger->log( nix::Verbosity::lvlDebug,
                 fmt( "lockfile: %s\n", lockfile_content.c_str() ) );

    LockfileRaw lockfile_raw = nlohmann::json::parse( lockfile_content );
    auto        lockfile     = Lockfile( lockfile_raw );

    auto state = getEvalState();

    // todo: allow to specify system?
    auto system = nix::nativeSystem;

    auto store_path = flox::createFloxEnv( *state, lockfile, system );

    // throw Error( "store_path: asdasdasdsadasdsada\n"
    //            );


    std::cout << fmt( "%s\n", store->printStorePath( store_path ).c_str() );

    auto store2 = store.dynamic_pointer_cast<LocalFSStore>();

    if ( ! store2 ) { throw Error( "store is not a LocalFSStore" ); }

    if ( out_link.has_value() )
      {
        auto out_link_path
          = store2->addPermRoot( store_path, absPath( out_link.value() ) );
        logger->log( nix::Verbosity::lvlDebug,
                     fmt( "out_link_path: %s\n", out_link_path.c_str() ) );
      }
  }
};

static auto rCmdBuildEnv = nix::registerCommand<CmdBuildEnv>( "build-env" );


/* -------------------------------------------------------------------------- */

}  // namespace flox


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
