/* ========================================================================== *
 *
 * @file buildenv/buildenv.cc
 *
 * @brief Realise an locked environment.
 *
 *
 * -------------------------------------------------------------------------- */

#include <algorithm>
#include <sys/stat.h>
#include <sys/types.h>

#include <nix/util.hh>

#include "flox/buildenv/realise.hh"


/* -------------------------------------------------------------------------- */

namespace flox::buildenv {

/* -------------------------------------------------------------------------- */

struct BuildEnvState
{
  std::map<std::string, Priority> priorities {};
  unsigned long                   symlinks = 0;
};


/* -------------------------------------------------------------------------- */


/* For each activated package, create symlinks */
// todo: break this function up to reduce complexity
// NOLINTBEGIN(readability-function-cognitive-complexity)
static void
createLinks( BuildEnvState &     state,
             const std::string & srcDir,
             const std::string & dstDir,
             const Priority &    priority )
{
  nix::DirEntries srcFiles;

  try
    {
      srcFiles = nix::readDirectory( srcDir );
    }
  catch ( nix::SysError & e )
    {
      if ( e.errNo == ENOTDIR )
        {
          nix::warn(
            "not including '%s' in the user environment because it's not a "
            "directory",
            srcDir );
          return;
        }
      throw;
    }

  for ( const auto & ent : srcFiles )
    {
      if ( ent.name[0] == '.' )
        {
          /* not matched by glob */
          continue;
        }
      auto srcFile = srcDir + "/" + ent.name;
      auto dstFile = dstDir + "/" + ent.name;

      struct stat srcSt
      {};
      try
        {
          if ( stat( srcFile.c_str(), &srcSt ) == -1 )
            {
              throw nix::SysError( "getting status of '%1%'", srcFile );
            }
        }
      catch ( nix::SysError & e )
        {
          if ( e.errNo == ENOENT || e.errNo == ENOTDIR )
            {
              nix::warn( "skipping dangling symlink '%s'", dstFile );
              continue;
            }
          throw;
        }

      /* The files below are special-cased so that they don't show
       * up in user profiles, either because they are useless,
       * or because they would cause pointless collisions
       * (e.g., each Python package brings its own
       * `$out/lib/pythonX.Y/site-packages/easy-install.pth'.)
       */
      if ( nix::hasSuffix( srcFile, "/propagated-build-inputs" )
           || nix::hasSuffix( srcFile, "/nix-support" )
           || nix::hasSuffix( srcFile, "/perllocal.pod" )
           || nix::hasSuffix( srcFile, "/info/dir" )
           || nix::hasSuffix( srcFile, "/log" )
           || nix::hasSuffix( srcFile, "/manifest.nix" )
           || nix::hasSuffix( srcFile, "/manifest.json" ) )
        {
          continue;
        }
      // todo: understand and document these branches
      // the short description is:
      // link directories in the source directory to the target directory
      // if the directory already exists, create a directory
      // and recursively link the contents.
      // Handle file type mismatches and conflicts with priority.
      if ( S_ISDIR( srcSt.st_mode ) )
        {
          struct stat dstSt
          {};
          auto res = lstat( dstFile.c_str(), &dstSt );
          if ( res == 0 )
            {
              if ( S_ISDIR( dstSt.st_mode ) )
                {
                  createLinks( state, srcFile, dstFile, priority );
                  continue;
                }

              if ( S_ISLNK( dstSt.st_mode ) )
                {
                  auto        target = nix::canonPath( dstFile, true );
                  struct stat canonSt
                  {};
                  if ( lstat( target.c_str(), &canonSt ) != 0 )
                    {
                      throw nix::SysError( "getting status of '%1%'", target );
                    }
                  if ( ! S_ISDIR( canonSt.st_mode ) )
                    {
                      throw nix::Error(
                        "collision between '%1%' and non-directory '%2%'",
                        srcFile,
                        target );
                    }
                  if ( unlink( dstFile.c_str() ) == -1 )
                    {
                      throw nix::SysError( "unlinking '%1%'", dstFile );
                    }

                  const auto dirPermissions = 0755;
                  if ( mkdir( dstFile.c_str(), dirPermissions ) == -1 )
                    {
                      throw nix::SysError( "creating directory '%1%'",
                                           dstFile );
                    }
                  createLinks( state,
                               target,
                               dstFile,
                               state.priorities[dstFile] );
                  createLinks( state, srcFile, dstFile, priority );
                  continue;
                }
            }
          else if ( errno != ENOENT )
            {
              throw nix::SysError( "getting status of '%1%'", dstFile );
            }
        }
      else
        {
          struct stat dstSt
          {};
          auto res = lstat( dstFile.c_str(), &dstSt );
          if ( res == 0 )
            {
              if ( S_ISLNK( dstSt.st_mode ) )
                {
                  auto prevPriority = state.priorities[dstFile];

                  // if the existing dest has a higher (numerically lower)
                  // priority -> skip
                  if ( prevPriority.priority < priority.priority ) { continue; }

                  // if src and dst have the same priority
                  if ( prevPriority.priority == priority.priority )
                    {

                      // ... and have different parents -> conflict
                      if ( prevPriority.parentPath != priority.parentPath )
                        {
                          throw FileConflict( nix::readLink( dstFile ),
                                              srcFile,
                                              priority.priority );
                        }

                      // ... and dest has a higher (numerically lower)
                      // internal priority -> skip
                      if ( prevPriority.internalPriority
                           < priority.internalPriority )
                        {
                          continue;
                        }
                    }


                  if ( unlink( dstFile.c_str() ) == -1 )
                    {
                      throw nix::SysError( "unlinking '%1%'", dstFile );
                    }
                }
              else if ( S_ISDIR( dstSt.st_mode ) )
                {
                  throw nix::Error(
                    "collision between non-directory '%1%' and directory '%2%'",
                    srcFile,
                    dstFile );
                }
            }
          else if ( errno != ENOENT )
            {
              throw nix::SysError( "getting status of '%1%'", dstFile );
            }
        }

      nix::createSymlink( srcFile, dstFile );
      state.priorities[dstFile] = priority;
      state.symlinks++;
    }
}
// NOLINTEND(readability-function-cognitive-complexity)


/* -------------------------------------------------------------------------- */

// todo: break this function up to reduce complexity
// NOLINTBEGIN(readability-function-cognitive-complexity)
void
buildEnvironment( const std::string & out, std::vector<RealisedPackage> & pkgs )
{
  BuildEnvState state;

  std::set<std::string> done;
  std::set<std::string> postponed;

  auto addPkg = [&]( const std::string & pkgDir, const Priority & priority )
  {
    if ( ! done.insert( pkgDir ).second ) { return; }
    createLinks( state, pkgDir, out, priority );

    try
      {
        for ( const auto & path : nix::tokenizeString<std::vector<std::string>>(
                nix::readFile( pkgDir
                               + "/nix-support/propagated-user-env-packages" ),
                " \n" ) )
          {
            if ( ! done.contains( path ) ) { postponed.insert( path ); }
          }
      }
    catch ( nix::SysError & e )
      {
        if ( e.errNo != ENOENT && e.errNo != ENOTDIR ) { throw; }
      }

    try
      {
        for ( const auto & path : nix::tokenizeString<std::vector<std::string>>(
                nix::readFile( pkgDir
                               + "/nix-support/propagated-build-inputs" ),
                " \n" ) )
          {
            if ( ! done.contains( path ) ) { postponed.insert( path ); }
          }
      }
    catch ( nix::SysError & e )
      {
        if ( e.errNo != ENOENT && e.errNo != ENOTDIR ) { throw; }
      }
  };

  /* Symlink to the packages that have been installed explicitly by the user.
   * Process in priority order to reduce unnecessary symlink/unlink steps.
   *
   * Note that we sort by priority, then by internal priority, then by path.
   * Internal priority is used to avoid conflicts
   * between outputs of the same derivation.
   *
   * The handling of internal priorities for outputs of the same derivation
   * is performed in `buildenv::createLinks'. */
  std::sort( pkgs.begin(),
             pkgs.end(),
             []( const RealisedPackage & first, const RealisedPackage & second )
             {
               auto firstP  = first.priority;
               auto secondP = second.priority;

               // order by priority
               if ( firstP.priority < secondP.priority ) { return true; }
               if ( firstP.priority > secondP.priority ) { return false; }

               // ... then internal priority
               if ( firstP.internalPriority < secondP.internalPriority )
                 {
                   return true;
                 }
               if ( firstP.internalPriority > secondP.internalPriority )
                 {
                   return false;
                 }

               // ... then (arbitrarily) by path
               return first.path < second.path;
             } );

  for ( const auto & pkg : pkgs )
    {
      if ( pkg.active ) { addPkg( pkg.path, pkg.priority ); }
    }

  /* Symlink the packages that have been "propagated" by packages
   * installed by the user
   * (i.e., package X declares that it wants Y installed as well).
   * We do these later because they have a lower priority in case of collisions.
   */
  // TODO: consider making this optional?
  // TODO: include paths recursively?
  // NOLINTNEXTLINE(cppcoreguidelines-avoid-magic-numbers,readability-magic-numbers)
  auto priorityCounter = 1000U;
  while ( ! postponed.empty() )
    {
      std::set<std::string> pkgDirs;
      postponed.swap( pkgDirs );
      for ( const auto & pkgDir : pkgDirs )
        {
          addPkg( pkgDir, Priority( priorityCounter++ ) );
        }
    }

  if ( nix::lvlDebug <= nix::verbosity )
    {
      nix::logger->log(
        nix::lvlDebug,
        nix::fmt( "created %d symlinks in user environment", state.symlinks ) );
    }
}
// NOLINTEND(readability-function-cognitive-complexity)

/* -------------------------------------------------------------------------- */

}  // namespace flox::buildenv


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
