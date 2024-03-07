/*
 * ld-floxlib - LD_AUDIT library that uses the GNU dynamic rtld-audit(7)
 *              hook to serve up dynamic libraries from FLOX_ENV_LIB_DIRS
 *              for the benefit of Nix-packaged binaries served up by flox
 *              developer environments, but only after all other possible
 *              locations have been exhausted. It provides a more targeted
 *              and safer mechanism than setting LD_LIBRARY_PATH, which has
 *              the potential to cause problems with other binaries not built
 *              and packaged with Nix. In this respect it is similar to the
 *              DYLD_FALLBACK_LIBRARY_PATH environment variable on Mac OS X
 *              which provides a colon-separated list of directories to search
 *              for dynamic libraries as a last resort as described in dyld(1).
 *
 *              See rtld-audit(7) for more information on the operation
 *              of the GNU dynamic linker and how it calls la_objsearch()
 *              repeatedly in the process of searching for a library in
 *              various locations.
 */

#ifndef _GNU_SOURCE
#  define _GNU_SOURCE
#endif /* _GNU_SOURCE */

#include <fcntl.h>
#include <limits.h>
#include <link.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/param.h>
#include <sys/types.h>
#include <unistd.h>

// Declare version bindings to work with minimum supported GLIBC versions.
#if defined( __aarch64__ )
// aarch64 Linux only goes back to 2.17.
__asm__( ".symver close,close@GLIBC_2.17" );
__asm__( ".symver fprintf,fprintf@GLIBC_2.17" );
__asm__( ".symver getenv,getenv@GLIBC_2.17" );
__asm__( ".symver open,open@GLIBC_2.17" );
__asm__( ".symver snprintf,snprintf@GLIBC_2.17" );
__asm__( ".symver stderr,stderr@GLIBC_2.17" );
__asm__( ".symver strrchr,strrchr@GLIBC_2.17" );
__asm__( ".symver strtok,strtok@GLIBC_2.17" );
#elif defined( __x86_64__ )
// x86_64 Linux goes back to 2.2.5.
__asm__( ".symver close,close@GLIBC_2.2.5" );
__asm__( ".symver fprintf,fprintf@GLIBC_2.2.5" );
__asm__( ".symver getenv,getenv@GLIBC_2.2.5" );
__asm__( ".symver open,open@GLIBC_2.2.5" );
__asm__( ".symver snprintf,snprintf@GLIBC_2.2.5" );
__asm__( ".symver stderr,stderr@GLIBC_2.2.5" );
__asm__( ".symver strrchr,strrchr@GLIBC_2.2.5" );
__asm__( ".symver strtok,strtok@GLIBC_2.2.5" );
#else
// Punt .. just go with default symbol bindings and hope for the best.
#endif

static int  audit_ld_floxlib = -1;
static int  debug_ld_floxlib = -1;
static char name_buf[PATH_MAX];

unsigned int
la_version( unsigned int version )
{
  return version;
}

char *
la_objsearch( const char * name, uintptr_t * cookie, unsigned int flag )
{
  if ( debug_ld_floxlib < 0 )
    {
      debug_ld_floxlib = ( getenv( "LD_FLOXLIB_DEBUG" ) != NULL );
    }

  if ( debug_ld_floxlib )
    {
      fprintf( stderr,
               "DEBUG: la_objsearch(%s, %s)\n",
               name,
               ( flag == LA_SER_ORIG )      ? "LA_SER_ORIG"
               : ( flag == LA_SER_LIBPATH ) ? "LA_SER_LIBPATH"
               : ( flag == LA_SER_RUNPATH ) ? "LA_SER_RUNPATH"
               : ( flag == LA_SER_DEFAULT ) ? "LA_SER_DEFAULT"
               : ( flag == LA_SER_CONFIG )  ? "LA_SER_CONFIG"
               : ( flag == LA_SER_SECURE )  ? "LA_SER_SECURE"
                                            : "???" );
    }

  // Only look for the library once the dynamic linker has exhausted
  // all of the other possible search locations, and only if it isn't
  // already specified by way of an explicit path.
  if ( flag == LA_SER_DEFAULT )
    {
      int fd = open( name, O_RDONLY );
      if ( fd != -1 ) { close( fd ); }
      else
        {
          char * basename          = strrchr( name, '/' );
          char * flox_env_lib_dirs = getenv( "FLOX_ENV_LIB_DIRS" );

          if ( basename != NULL ) { basename++; }
          else { basename = (char *) name; }

          if ( flox_env_lib_dirs != NULL )
            {
              // Iterate over the colon-separated list of paths in
              // FLOX_ENV_LIB_DIRS looking for the requested library.
              // If found, return the full path to the library and
              // otherwise return the original name.
              char * flox_env_library_dir = NULL;
              flox_env_library_dir        = strtok( flox_env_lib_dirs, ":" );
              while ( flox_env_library_dir != NULL )
                {
                  (void) snprintf( name_buf,
                                   sizeof( name_buf ),
                                   "%s/%s",
                                   flox_env_library_dir,
                                   basename );
                  if ( debug_ld_floxlib )
                    {
                      fprintf( stderr,
                               "DEBUG: la_objsearch() checking: %s\n",
                               name_buf );
                    }
                  fd = open( name_buf, O_RDONLY );
                  if ( fd != -1 )
                    {
                      close( fd );
                      if ( audit_ld_floxlib < 0 )
                        {
                          audit_ld_floxlib
                            = ( getenv( "LD_FLOXLIB_AUDIT" ) != NULL );
                        }
                      if ( audit_ld_floxlib || debug_ld_floxlib )
                        {
                          fprintf( stderr,
                                   "AUDIT: la_objsearch() resolved %s -> %s\n",
                                   name,
                                   name_buf );
                        }
                      return name_buf;
                    }
                  flox_env_library_dir = strtok( NULL, ":" );
                }
            }
        }
    }
  return (char *) name;
}
/* vim: set et ts=4: */
