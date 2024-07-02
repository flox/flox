/*
 * Test harness for exercising ld-floxlib.so.
 *
 * Loads library function as defined with TEST_FUNCTION macro.
 */

#include <stdio.h>
extern unsigned int TEST_FUNCTION( unsigned int rc );

int
main()
{
  return TEST_FUNCTION(0);
}
