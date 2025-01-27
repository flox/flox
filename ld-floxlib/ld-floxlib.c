/*
 * ld-floxlib - LD_AUDIT library that uses the GNU dynamic rtld-audit(7)
 *              hook to serve up dynamic libraries from FLOX_ENV_DIRS
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

// Define the maximum number of directories that can be specified in
// any of the FLOX_ENV_DIRS, LD_FLOXLIB_DIRS_PATH or LD_FLOXLIB_FILES_PATH
// environment variables. This is an arbitrary limit that should be
// more than enough for most cases.
#define LIB_ENVVAR_MAXENTRIES 256

// Define the maximum length of a directory path in an environment variable.
// This is also somewhat arbitrary but it should be more than enough for most
// cases.
#define LIB_ENVVAR_MAXLEN PATH_MAX

#define LIB_SUFFIX "/lib"

static int    audit_ld_floxlib = -1;
static int    debug_ld_floxlib = -1;
static char   name_buf[PATH_MAX];
static int    flox_env_dirs_count = -1;
static char   flox_env_dirs_buf[LIB_ENVVAR_MAXLEN];
static char * flox_env_dirs[LIB_ENVVAR_MAXENTRIES];
static int    ld_floxlib_dirs_path_count = -1;
static char   ld_floxlib_dirs_path_buf[LIB_ENVVAR_MAXLEN];
static char * ld_floxlib_dirs_path[LIB_ENVVAR_MAXENTRIES];
static int    ld_floxlib_files_path_count = -1;
static char   ld_floxlib_files_path_buf[LIB_ENVVAR_MAXLEN];
static char * ld_floxlib_files_path[LIB_ENVVAR_MAXENTRIES];

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
          char * basename = strrchr( name, '/' );
          if ( basename != NULL ) { basename++; }
          else { basename = (char *) name; }

          if ( flox_env_dirs_count == -1 )
            {
              // Populate flox_env_dirs_buf from FLOX_ENV_DIRS, tokenizing the buffer
              // by replacing colons with NULLs as we count the entries and saving
              // pointers to each of the paths in the flox_env_dirs[] array.
              flox_env_dirs_count = 0;
              const char * flox_env_dirs_env = getenv( "FLOX_ENV_DIRS" );
              if ( flox_env_dirs_env != NULL )
                {
                  if ( sizeof( flox_env_dirs_env )
                       >= LIB_ENVVAR_MAXLEN )
                    {
                      fprintf( stderr,
                               "ERROR: la_objsearch() "
                               "FLOX_ENV_DIRS is too long, "
                               "truncating to %d characters\n",
                               LIB_ENVVAR_MAXLEN );
                    }

                  strncpy( flox_env_dirs_buf,
                           flox_env_dirs_env,
                           sizeof( flox_env_dirs_buf ) );

                  // Iterate over the colon-separated list of paths in the
                  // flox_env_dirs_buf buffer, tokenizing as we go and
                  // maintaining a count of the number of entries found.
                  char * lib_dir = NULL;
                  char * saveptr = NULL;  // For strtok_r() context

                  lib_dir = strtok_r( flox_env_dirs_buf, ":", &saveptr );
                  while ( lib_dir != NULL )
                    {
                      if ( flox_env_dirs_count
                           >= LIB_ENVVAR_MAXENTRIES )
                        {
                          fprintf( stderr,
                                   "ERROR: la_objsearch() "
                                   "FLOX_ENV_DIRS has too many entries, "
                                   "truncating to the first %d\n",
                                   LIB_ENVVAR_MAXENTRIES );
                          break;
                        }
                      if ( debug_ld_floxlib )
                        {
                          fprintf( stderr,
                                   "DEBUG: la_objsearch() "
                                   "flox_env_dirs[%d] = %s\n",
                                   flox_env_dirs_count,
                                   lib_dir );
                        }
                      flox_env_dirs[flox_env_dirs_count] = malloc(strlen(lib_dir) + strlen(LIB_SUFFIX) + 1);
                      if (flox_env_dirs[flox_env_dirs_count] == NULL) {
                        fprintf(stderr, "Memory allocation failed\n");
                        break;
                      }
                      strcpy(flox_env_dirs[flox_env_dirs_count], lib_dir);
                      strcat(flox_env_dirs[flox_env_dirs_count], LIB_SUFFIX);
                      lib_dir = strtok_r( NULL, ":", &saveptr );
                      flox_env_dirs_count++;
                    }
                }
            }

          // Repeat for the LD_FLOXLIB_DIRS_PATH variable.
          if ( ld_floxlib_dirs_path_count == -1 )
            {
              // Similarly populate ld_floxlib_dirs_path_buf from LD_FLOXLIB_DIRS_PATH.
              ld_floxlib_dirs_path_count = 0;
              const char * ld_floxlib_dirs_path_env = getenv( "LD_FLOXLIB_DIRS_PATH" );
              if ( ld_floxlib_dirs_path_env != NULL )
                {
                  if ( sizeof( ld_floxlib_dirs_path_env )
                       >= LIB_ENVVAR_MAXLEN )
                    {
                      fprintf( stderr,
                               "ERROR: la_objsearch() "
                               "LD_FLOXLIB_DIRS_PATH is too long, "
                               "truncating to %d characters\n",
                               LIB_ENVVAR_MAXLEN );
                    }

                  strncpy( ld_floxlib_dirs_path_buf,
                           ld_floxlib_dirs_path_env,
                           sizeof( ld_floxlib_dirs_path_buf ) );

                  char * lib_dir = NULL;
                  char * saveptr = NULL;  // For strtok_r() context

                  lib_dir = strtok_r( ld_floxlib_dirs_path_buf, ":", &saveptr );
                  while ( lib_dir != NULL )
                    {
                      if ( ld_floxlib_dirs_path_count
                           >= LIB_ENVVAR_MAXENTRIES )
                        {
                          fprintf( stderr,
                                   "ERROR: la_objsearch() "
                                   "LD_FLOXLIB_DIRS_PATH has too many entries, "
                                   "truncating to the first %d\n",
                                   LIB_ENVVAR_MAXENTRIES );
                          break;
                        }
                      if ( debug_ld_floxlib )
                        {
                          fprintf( stderr,
                                   "DEBUG: la_objsearch() "
                                   "ld_floxlib_dirs_path[%d] = %s\n",
                                   ld_floxlib_dirs_path_count,
                                   lib_dir );
                        }
                      ld_floxlib_dirs_path[ld_floxlib_dirs_path_count]
                        = lib_dir;
                      lib_dir = strtok_r( NULL, ":", &saveptr );
                      ld_floxlib_dirs_path_count++;
                    }
                }
            }

          if ( ld_floxlib_files_path_count == -1 )
            {
              // Populate ld_floxlib_files_path_buf from LD_FLOXLIB_FILES_PATH, tokenizing the
              // buffer by replacing colons with NULLs as we count the entries and
              // saving pointers to each of the paths in the ld_floxlib_files_path[] array.
              ld_floxlib_files_path_count = 0;
              const char * ld_floxlib_files_path_env = getenv( "LD_FLOXLIB_FILES_PATH" );
              if ( ld_floxlib_files_path_env != NULL )
                {
                  if ( sizeof( ld_floxlib_files_path_env )
                       >= LIB_ENVVAR_MAXLEN )
                    {
                      fprintf( stderr,
                               "ERROR: la_objsearch() "
                               "LD_FLOXLIB_FILES_PATH is too long, "
                               "truncating to %d characters\n",
                               LIB_ENVVAR_MAXLEN );
                    }

                  strncpy( ld_floxlib_files_path_buf,
                           ld_floxlib_files_path_env,
                           sizeof( ld_floxlib_files_path_buf ) );

                  char * lib_dir = NULL;
                  char * saveptr = NULL;  // For strtok_r() context

                  lib_dir = strtok_r( ld_floxlib_files_path_buf, ":", &saveptr );
                  while ( lib_dir != NULL )
                    {
                      if ( ld_floxlib_files_path_count
                           >= LIB_ENVVAR_MAXENTRIES )
                        {
                          fprintf( stderr,
                                   "ERROR: la_objsearch() "
                                   "LD_FLOXLIB_FILES_PATH has too many entries, "
                                   "truncating to the first %d\n",
                                   LIB_ENVVAR_MAXENTRIES );
                          break;
                        }
                      if ( debug_ld_floxlib )
                        {
                          fprintf( stderr,
                                   "DEBUG: la_objsearch() "
                                   "ld_floxlib_files_path[%d] = %s\n",
                                   ld_floxlib_files_path_count,
                                   lib_dir );
                        }
                      ld_floxlib_files_path[ld_floxlib_files_path_count]
                        = lib_dir;
                      lib_dir = strtok_r( NULL, ":", &saveptr );
                      ld_floxlib_files_path_count++;
                    }
                }
            }

          // Iterate over the list of files in flox_env_dirs, ld_floxlib_files_path
          // and ld_floxlib_dirs_path (in that order) looking for the requested
          // library.  If found, return the full path to the library and otherwise
          // return the original name.
          static int i;

          for ( i = 0; i < flox_env_dirs_count; i++ )
            {
              {
                (void) snprintf( name_buf,
                                 sizeof( name_buf ),
                                 "%s/%s",
                                 flox_env_dirs[i],
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
              }
            }

          for ( i = 0; i < ld_floxlib_files_path_count; i++ )
            {
              {
                // Only continue if the requested file matches the basename.
                char * file_basename = strrchr( ld_floxlib_files_path[i], '/' );
                if ( file_basename != NULL ) { file_basename++; }
                else { file_basename = (char *) ld_floxlib_files_path[i]; }
                if (strcmp(file_basename, basename) != 0)
                  {
                    continue;
                  }
                if ( debug_ld_floxlib )
                  {
                    fprintf( stderr,
                             "DEBUG: la_objsearch() checking: %s\n",
                             ld_floxlib_files_path[i] );
                  }
                fd = open( ld_floxlib_files_path[i], O_RDONLY );
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
                                 ld_floxlib_files_path[i] );
                      }
                    return ld_floxlib_files_path[i];
                  }
              }
            }

          for ( i = 0; i < ld_floxlib_dirs_path_count; i++ )
            {
              {
                (void) snprintf( name_buf,
                                 sizeof( name_buf ),
                                 "%s/%s",
                                 ld_floxlib_dirs_path[i],
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
              }
            }

        }
    }
  return (char *) name;
}
/* vim: set et ts=4: */
