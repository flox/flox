#include "flox/buildenv.hh"

#include <algorithm>
#include <fcntl.h>
#include <sys/stat.h>
#include <sys/types.h>

namespace flox::buildenv {
using namespace nix;


struct State
{
  std::map<Path, Priority> priorities;
  unsigned long            symlinks = 0;
};

/* For each activated package, create symlinks */
static void
createLinks( State &          state,
             const Path &     srcDir,
             const Path &     dstDir,
             const Priority & priority )
{
  DirEntries srcFiles;

  try
    {
      srcFiles = readDirectory( srcDir );
    }
  catch ( SysError & e )
    {
      if ( e.errNo == ENOTDIR )
        {
          warn( "not including '%s' in the user environment because it's not a "
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

      struct stat srcSt;
      try
        {
          if ( stat( srcFile.c_str(), &srcSt ) == -1 )
            {
              throw SysError( "getting status of '%1%'", srcFile );
            }
        }
      catch ( SysError & e )
        {
          if ( e.errNo == ENOENT || e.errNo == ENOTDIR )
            {
              warn( "skipping dangling symlink '%s'", dstFile );
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
      if ( hasSuffix( srcFile, "/propagated-build-inputs" )
           || hasSuffix( srcFile, "/nix-support" )
           || hasSuffix( srcFile, "/perllocal.pod" )
           || hasSuffix( srcFile, "/info/dir" ) || hasSuffix( srcFile, "/log" )
           || hasSuffix( srcFile, "/manifest.nix" )
           || hasSuffix( srcFile, "/manifest.json" ) )
        {
          continue;
        }
      // todo: understand and document these branches
      // the short description is:
      // link directories in the source directory to the target directory
      // if the directory already exists, create a directory
      // and recursively link the contents.
      // Handle file type mismatches and conflicts with priority.
      else if ( S_ISDIR( srcSt.st_mode ) )
        {
          struct stat dstSt;
          auto        res = lstat( dstFile.c_str(), &dstSt );
          if ( res == 0 )
            {
              if ( S_ISDIR( dstSt.st_mode ) )
                {
                  createLinks( state, srcFile, dstFile, priority );
                  continue;
                }
              else if ( S_ISLNK( dstSt.st_mode ) )
                {
                  auto target = canonPath( dstFile, true );
                  if ( ! S_ISDIR( lstat( target ).st_mode ) )
                    {
                      throw Error(
                        "collision between '%1%' and non-directory '%2%'",
                        srcFile,
                        target );
                    }
                  if ( unlink( dstFile.c_str() ) == -1 )
                    {
                      throw SysError( "unlinking '%1%'", dstFile );
                    }
                  if ( mkdir( dstFile.c_str(), 0755 ) == -1 )
                    {
                      throw SysError( "creating directory '%1%'", dstFile );
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
              throw SysError( "getting status of '%1%'", dstFile );
            }
        }
      else
        {
          struct stat dstSt;
          auto        res = lstat( dstFile.c_str(), &dstSt );
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
                          throw BuildEnvFileConflictError( readLink( dstFile ),
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
                      throw SysError( "unlinking '%1%'", dstFile );
                    }
                }
              else if ( S_ISDIR( dstSt.st_mode ) )
                {
                  throw Error(
                    "collision between non-directory '%1%' and directory '%2%'",
                    srcFile,
                    dstFile );
                }
            }
          else if ( errno != ENOENT )
            {
              throw SysError( "getting status of '%1%'", dstFile );
            }
        }

      createSymlink( srcFile, dstFile );
      state.priorities[dstFile] = priority;
      state.symlinks++;
    }
}

void
buildEnvironment( const Path & out, Packages && pkgs )
{
  State state;

  std::set<Path> done, postponed;

  auto addPkg = [&]( const Path & pkgDir, const Priority & priority )
  {
    if ( ! done.insert( pkgDir ).second ) { return; }
    createLinks( state, pkgDir, out, priority );

    try
      {
        for ( const auto & p : tokenizeString<std::vector<std::string>>(
                readFile( pkgDir
                          + "/nix-support/propagated-user-env-packages" ),
                " \n" ) )
          {
            if ( ! done.count( p ) ) { postponed.insert( p ); }
          }
      }
    catch ( SysError & e )
      {
        if ( e.errNo != ENOENT && e.errNo != ENOTDIR ) { throw; }
      }

    try
      {
        for ( const auto & p : tokenizeString<std::vector<std::string>>(
                readFile( pkgDir + "/nix-support/propagated-build-inputs" ),
                " \n" ) )
          {
            if ( ! done.count( p ) ) { postponed.insert( p ); }
          }
      }
    catch ( SysError & e )
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
   * is performed in `buildenv::createLinks`.
   */
  std::sort( pkgs.begin(),
             pkgs.end(),
             []( const Package & a, const Package & b )
             {
               auto aP = a.priority;
               auto bP = b.priority;

               // order by priority
               if ( aP.priority < bP.priority ) { return true; }
               if ( aP.priority > bP.priority ) { return false; }

               // ... then internal priority
               if ( aP.internalPriority < bP.internalPriority ) { return true; }
               if ( aP.internalPriority > bP.internalPriority )
                 {
                   return false;
                 }

               // ... then (arbitrarily) by path
               return a.path < b.path;
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
  // todo: consider making this optional?
  // todo: include paths recursively?
  auto priorityCounter = 1000u;
  while ( ! postponed.empty() )
    {
      std::set<Path> pkgDirs;
      postponed.swap( pkgDirs );
      for ( const auto & pkgDir : pkgDirs )
        {
          printf( "postponed: %s\n", pkgDir.c_str() );
          addPkg( pkgDir, Priority { priorityCounter++ } );
        }
    }

  debug( "created %d symlinks in user environment", state.symlinks );
}

}  // namespace flox::buildenv
