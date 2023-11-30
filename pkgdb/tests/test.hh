/* ========================================================================== *
 *
 *
 *
 * -------------------------------------------------------------------------- */

#pragma once

#include <string>
#include <iostream>
#include <cstddef>


/* -------------------------------------------------------------------------- */

/* This shouldn't happen, but it's a sane fallback for running from the
 * project root. */
#ifndef TEST_DATA_DIR
#  define TEST_DATA_DIR  "./tests/data"
#endif  /* End `ifndef TEST_DATA_DIR' */


/* -------------------------------------------------------------------------- */

static const std::string nixpkgsRev =
  "e8039594435c68eb4f780f3e9bf3972a7399c4b1";

static const std::string nixpkgsRef =
  "github:NixOS/nixpkgs/e8039594435c68eb4f780f3e9bf3972a7399c4b1";

static const std::string nixpkgsFingerprintStr =
  "5fde12e3424840cc2752dae09751b09b03f5a33c3ec4de672fc89d236720bdc7";


/**
 * These counts indicate the total number of derivations under
 * `<nixpkgsRef>#legacyPackages.x86_64-linux.**' which we will use to sanity
 * check calls to `size()'.
 * Note that the legacy implementation used to populate `DbPackageSet' will
 * fail to evaluate 3 packages which require `NIXPKGS_ALLOW_BROKEN', causing
 * different sizes to be collected ( until migration is coompleted ).
 */
static const size_t unbrokenPkgCount = 64163;
static const size_t fullPkgCount     = 64040;


/* -------------------------------------------------------------------------- */

/** @brief Wrap a test function pretty printing its name on failure. */
template <typename F, typename ... Args>
  static int
runTest( std::string_view name, F f, Args && ... args )
{
  try
    {
      if ( ! f( std::forward<Args>( args )... ) )
        {
          std::cerr << "  fail: " << name << std::endl;
          return EXIT_FAILURE;
        }
    }
  catch( const std::exception & e )
    {
      std::cerr << "  ERROR: " << name << ": " << e.what() << std::endl;
      return EXIT_FAILURE;
    }
  return EXIT_SUCCESS;
}


/* -------------------------------------------------------------------------- */

/**
 * @brief Wrap a test routine which returns an exit code, and set a provided
 *        variable to the resulting code on failure.
 *
 * This pattern allows early tests to still run later ones, while preserving
 * a "global" exit status.
 * 
 * This emits a warning on clang until we start using clang 12
 * "must specify at least one argument for '...' parameter of variadic macro"
 * https://github.com/llvm/llvm-project/issues/50951
 */
#define _RUN_TEST( _EXIT_CODE, _NAME, ... )                       \
  {                                                               \
    int _exitCode = runTest(             ( # _NAME )              \
                       ,             ( test_ ## _NAME )           \
           __VA_OPT__( , ) __VA_ARGS__                            \
                       );                                         \
    if ( _exitCode != EXIT_SUCCESS ) { _EXIT_CODE = _exitCode; }  \
  }


/* -------------------------------------------------------------------------- */

/**
 * @brief For use inside of a function which returns a boolean.
 *
 * Assert that and expression is `true', otherwise print it and return `false'.
 */
#define EXPECT( EXPR )                      \
  if ( ! ( EXPR ) )                         \
    {                                       \
      std::cerr << "Expectation failed: ";  \
      std::cerr << ( # EXPR );              \
      std::cerr << std::endl;               \
      return false;                         \
    }


/**
 * @brief For use inside of a function which returns a boolean.
 *
 * Assert that two expressions produce equal results, otherwise print them and
 * return `false'.
 *
 * Beware of comparing two char *. The following, for example, will fail:
 * std::string foo( "foo" );
 * EXPECT_EQ( "foo", foo.c_str() );
 */
#define EXPECT_EQ( EXPR_A, EXPR_B )                                            \
  {                                                                            \
    auto valA = ( EXPR_A );                                                    \
    auto valB = ( EXPR_B );                                                    \
    if ( valA != valB )                                                        \
      {                                                                        \
        std::cerr << "Expectation failed: ( ";                                 \
        std::cerr << ( # EXPR_A );                                             \
        std::cerr << " ) == ( ";                                               \
        std::cerr << ( # EXPR_B );                                             \
        std::cerr << " ). Got '" << valA << "' != '" << valB << "'"            \
                  << std::endl;                                                \
        return false;                                                          \
      }                                                                        \
  }


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
