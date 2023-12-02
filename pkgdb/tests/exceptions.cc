/* ========================================================================== *
 *
 * @file resolver.cc
 *
 * @brief Tests for `flox::exceptions`.
 *
 *
 * -------------------------------------------------------------------------- */

#include <cstdlib>
#include <iostream>

#include "flox/core/command.hh"
#include "flox/core/exceptions.hh"
#include "test.hh"


/* -------------------------------------------------------------------------- */

using namespace flox;

/* -------------------------------------------------------------------------- */

/** @brief Test what() correctly calls virtual methods. */
bool
test_what0()
{
  FloxException base( "context" );
  EXPECT_EQ( std::string( base.what() ),
             std::string( "general error: context" ) );

  command::InvalidArgException derived( "context" );
  FloxException *              derivedPtr = &derived;

  EXPECT_EQ( std::string( derivedPtr->what() ),
             std::string( "invalid argument: context" ) );

  return true;
}


/* -------------------------------------------------------------------------- */

int
main()
{
  int exitCode = EXIT_SUCCESS;
#define RUN_TEST( ... ) _RUN_TEST( exitCode, __VA_ARGS__ )

  RUN_TEST( what0 );

  return exitCode;
}


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
