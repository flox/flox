/*
 * Test harness for exercising ld-floxlib.so.
 *
 * Loads library function as defined with TEST_FUNCTION macro.
 */

extern unsigned int TEST_FUNCTION( unsigned int rc );

int
main()
{
  return TEST_FUNCTION(0);
}
