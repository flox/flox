/*
 * Unit test runner for ld-floxlib.so.
 *
 * By design ld-floxlib only parses FLOX_ENV_LIB_DIRS once per invocation,
 * so testing la_objsearch() for various combinations of env and arg input
 * requires multiple invocations. This test program calls the ld-floxlib.so
 * la_objsearch() function with the provided "name" arg and asserts that it
 * returns the expected value. It also performs a quick test of la_version()
 * for good measure while we're in the neighbourhood.
 */

#ifndef _GNU_SOURCE
#  define _GNU_SOURCE
#endif /* _GNU_SOURCE */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <assert.h>
#include <link.h>
extern int sandbox_level;
extern unsigned int la_version( unsigned int version );
extern char * la_objsearch( const char * name, uintptr_t * cookie, unsigned int flag );

int
main( int argc, char **argv )
{
  if ( argc != 3 ) {
    fprintf(stderr, "ERROR: expected 2 arguments, received %d\n"
      "USAGE: %s <name_to_lookup> <expected_value>\n", (argc - 1), argv[0]);
    exit(1);
  }

  // la_version() is basically the identity function. It's worth the
  // usecond or two to give la_version() a quick test.
  assert(la_version(1) == 1);
  assert(la_version(2) == 2);
  assert(la_version(3) != 2);
  assert(la_version(-1) == -1);

  // la_objsearch() searches the contents of the FLOX_ENV_LIB_DIRS
  // variable looking for library matches, but only when invoked
  // with the LA_SER_DEFAULT flag. Take a moment to ensure all other
  // flags return the input unaltered.
  assert(la_objsearch(argv[1], (uintptr_t *) NULL, LA_SER_ORIG   ) == argv[1]);
  assert(la_objsearch(argv[1], (uintptr_t *) NULL, LA_SER_LIBPATH) == argv[1]);
  assert(la_objsearch(argv[1], (uintptr_t *) NULL, LA_SER_RUNPATH) == argv[1]);
  assert(la_objsearch(argv[1], (uintptr_t *) NULL, LA_SER_CONFIG ) == argv[1]);
  assert(la_objsearch(argv[1], (uintptr_t *) NULL, LA_SER_SECURE ) == argv[1]);

  // Call la_objsearch() with the supplied args and assert it returns
  // the expected output.
  char * retval = la_objsearch(argv[1], (uintptr_t *) NULL, LA_SER_DEFAULT);
  if (strcmp(retval, argv[2]) == 0) {
    exit(0);
  } else {
    fprintf(stderr, "FAIL: expected '%s', received '%s'\n", argv[2], retval);
    exit(1);
  }
}
