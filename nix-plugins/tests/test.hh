/* ========================================================================== *
 *
 *
 *
 * -------------------------------------------------------------------------- */

#pragma once

#include <cstddef>
#include <iostream>
#include <string>


/* -------------------------------------------------------------------------- */

/* This shouldn't happen, but it's a sane fallback for running from the
 * project root. */
#ifndef TEST_DATA_DIR
#  define TEST_DATA_DIR "./tests/data"
#endif /* End `ifndef TEST_DATA_DIR' */


/* -------------------------------------------------------------------------- */

static const std::string nixpkgsRev
  = "ab5fd150146dcfe41fda501134e6503932cc8dfd";

static const std::string nixpkgsRef
  = "github:NixOS/nixpkgs/ab5fd150146dcfe41fda501134e6503932cc8dfd";

static const std::string nixpkgsFingerprintStr
  = "9bb3d4c033fbad8efb5e28ffcd1d70383e0c5bbcb7cc5c526b824524467b19b9";

/* The version of curl in nixpkgsRev */
static const std::string curlVersion = "8.4.0";


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
template<typename F, typename... Args>
static int
runTest( std::string_view name, F f, Args &&... args )
{
  try
    {
      if ( ! f( std::forward<Args>( args )... ) )
        {
          std::cerr << "  fail: " << name << std::endl;
          return EXIT_FAILURE;
        }
    }
  catch ( const std::exception & e )
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
#define _RUN_TEST( _EXIT_CODE, _NAME, ... )                               \
  {                                                                       \
    int _exitCode                                                         \
      = runTest( ( #_NAME ), (test_##_NAME) __VA_OPT__(, ) __VA_ARGS__ ); \
    if ( _exitCode != EXIT_SUCCESS ) { _EXIT_CODE = _exitCode; }          \
  }


/* -------------------------------------------------------------------------- */

/**
 * @brief For use inside of a function which returns a boolean.
 *
 * Report a failure with a message and return `false'.
 */
#define EXPECT_FAIL( MSG )               \
  {                                      \
    std::cerr << "Expectation failed: "; \
    std::cerr << ( MSG );                \
    std::cerr << std::endl;              \
    return false;                        \
  }


/* -------------------------------------------------------------------------- */

/**
 * @brief For use inside of a function which returns a boolean.
 *
 * Assert that and expression is `true', otherwise print it and return `false'.
 */
#define EXPECT( EXPR ) \
  if ( ! ( EXPR ) ) { EXPECT_FAIL( #EXPR ) }


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
#define EXPECT_EQ( EXPR_A, EXPR_B )                                 \
  {                                                                 \
    auto valA = ( EXPR_A );                                         \
    auto valB = ( EXPR_B );                                         \
    if ( valA != valB )                                             \
      {                                                             \
        std::cerr << "Expectation failed: ( ";                      \
        std::cerr << ( #EXPR_A );                                   \
        std::cerr << " ) == ( ";                                    \
        std::cerr << ( #EXPR_B );                                   \
        std::cerr << " ). Got '" << valA << "' != '" << valB << "'" \
                  << std::endl;                                     \
        return false;                                               \
      }                                                             \
  }


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
