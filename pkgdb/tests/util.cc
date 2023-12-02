/* ========================================================================== *
 *
 * @file util.cc
 *
 * @brief Tests for `flox` utility interfaces.
 *
 *
 * -------------------------------------------------------------------------- */

#include <cstdlib>
#include <iostream>

#include "flox/core/types.hh"
#include "flox/core/util.hh"
#include "test.hh"


/* -------------------------------------------------------------------------- */

bool
test_splitAttrPath0()
{
  EXPECT( flox::splitAttrPath( "a.b.c" )
          == ( flox::AttrPath { "a", "b", "c" } ) );
  return true;
}


/* -------------------------------------------------------------------------- */

bool
test_splitAttrPath1()
{
  EXPECT( flox::splitAttrPath( "a.'b.c'.d" )
          == ( flox::AttrPath { "a", "b.c", "d" } ) );
  return true;
}


/* -------------------------------------------------------------------------- */

bool
test_splitAttrPath2()
{
  EXPECT( flox::splitAttrPath( "a.\"b.c\".d" )
          == ( flox::AttrPath { "a", "b.c", "d" } ) );
  return true;
}


/* -------------------------------------------------------------------------- */

bool
test_splitAttrPath3()
{
  EXPECT( flox::splitAttrPath( "a.\"b.'c.d'.e\".f" )
          == ( flox::AttrPath { "a", "b.'c.d'.e", "f" } ) );
  return true;
}


/* -------------------------------------------------------------------------- */

bool
test_splitAttrPath4()
{
  EXPECT( flox::splitAttrPath( "a.\\\"b.c" )
          == ( flox::AttrPath { "a", "\"b", "c" } ) );
  return true;
}


/* -------------------------------------------------------------------------- */

bool
test_splitAttrPath5()
{
  EXPECT( flox::splitAttrPath( "a.'\"b'.c" )
          == ( flox::AttrPath { "a", "\"b", "c" } ) );
  return true;
}


/* -------------------------------------------------------------------------- */

bool
test_splitAttrPath6()
{
  EXPECT( flox::splitAttrPath( "a.\\\\\\..c" )
          == ( flox::AttrPath { "a", "\\.", "c" } ) );
  return true;
}


/* -------------------------------------------------------------------------- */

/** @brief Test conversion of variants with 2 options. */
bool
test_variantJSON0()
{
  using Trivial = std::variant<bool, std::string>;

  Trivial        tbool = true;
  Trivial        tstr  = "Howdy";
  nlohmann::json jto   = tbool;

  EXPECT_EQ( jto, true );

  jto = tstr;
  EXPECT_EQ( jto, "Howdy" );

  return true;
}


/* -------------------------------------------------------------------------- */

/** @brief Test conversion of variants with 3 options. */
bool
test_variantJSON1()
{
  using Trivial = std::variant<int, bool, std::string>;

  Trivial        tint  = 420;
  Trivial        tbool = true;
  Trivial        tstr  = "Howdy";
  nlohmann::json jto   = tint;

  EXPECT_EQ( jto, 420 );

  jto = tbool;
  EXPECT_EQ( jto, true );

  jto = tstr;
  EXPECT_EQ( jto, "Howdy" );

  return true;
}


/* -------------------------------------------------------------------------- */

/** @brief Test conversion of variants with 2 options in a vector. */
bool
test_variantJSON2()
{
  using Trivial = std::variant<bool, std::string>;

  std::vector<Trivial> tvec = { true, "Howdy" };

  nlohmann::json jto = tvec;

  EXPECT( jto.is_array() );
  EXPECT_EQ( jto.at( 0 ), true );
  EXPECT_EQ( jto.at( 1 ), "Howdy" );

  std::vector<Trivial> back = jto;
  EXPECT_EQ( back.size(), std::size_t( 2 ) );

  EXPECT( std::holds_alternative<bool>( back.at( 0 ) ) );
  EXPECT_EQ( std::get<bool>( back.at( 0 ) ), std::get<bool>( tvec.at( 0 ) ) );

  EXPECT( std::holds_alternative<std::string>( back.at( 1 ) ) );
  EXPECT_EQ( std::get<std::string>( back.at( 1 ) ),
             std::get<std::string>( tvec.at( 1 ) ) );

  return true;
}


/* -------------------------------------------------------------------------- */

/** @brief Test conversion of variants with 3 options in a vector. */
bool
test_variantJSON3()
{
  /* NOTE: `bool` MUST come before `int` to avoid coercion!
   * `std::string` always has to go last. */
  using Trivial = std::variant<bool, int, std::string>;

  std::vector<Trivial> tvec = { true, "Howdy", 420 };

  nlohmann::json jto = tvec;

  EXPECT( jto.is_array() );
  EXPECT_EQ( jto.at( 0 ), true );
  EXPECT_EQ( jto.at( 1 ), "Howdy" );
  EXPECT_EQ( jto.at( 2 ), 420 );

  std::vector<Trivial> back = jto;
  EXPECT_EQ( back.size(), std::size_t( 3 ) );

  EXPECT( std::holds_alternative<bool>( back.at( 0 ) ) );
  EXPECT_EQ( std::get<bool>( back.at( 0 ) ), std::get<bool>( tvec.at( 0 ) ) );

  EXPECT( std::holds_alternative<std::string>( back.at( 1 ) ) );
  EXPECT_EQ( std::get<std::string>( back.at( 1 ) ),
             std::get<std::string>( tvec.at( 1 ) ) );

  EXPECT( std::holds_alternative<int>( back.at( 2 ) ) );
  EXPECT_EQ( std::get<int>( back.at( 2 ) ), std::get<int>( tvec.at( 2 ) ) );

  return true;
}


/* -------------------------------------------------------------------------- */

bool
test_hasPrefix0()
{
  EXPECT( flox::hasPrefix( "foo", "foobar" ) );
  EXPECT( ! flox::hasPrefix( "bar", "foobar" ) );
  EXPECT( ! flox::hasPrefix( "foobar", "foo" ) );
  return true;
}


/* -------------------------------------------------------------------------- */

bool
test_ltrim_copy0()
{
  std::string str( "  foo " );
  EXPECT_EQ( flox::ltrim_copy( str ), "foo " );
  EXPECT_EQ( flox::ltrim_copy( str ), flox::ltrim_copy( str ) );
  return true;
}


/* -------------------------------------------------------------------------- */

bool
test_rtrim_copy0()
{
  std::string str( "  foo " );
  EXPECT_EQ( flox::rtrim_copy( str ), "  foo" );
  EXPECT_EQ( flox::rtrim_copy( str ), flox::rtrim_copy( str ) );
  return true;
}


/* -------------------------------------------------------------------------- */

bool
test_trim_copy0()
{
  std::string str( "  foo " );
  EXPECT_EQ( flox::trim_copy( str ), "foo" );
  EXPECT_EQ( flox::trim_copy( str ), flox::trim_copy( str ) );
  return true;
}


/* -------------------------------------------------------------------------- */

int
main()
{
  int ec = EXIT_SUCCESS;
#define RUN_TEST( ... ) _RUN_TEST( ec, __VA_ARGS__ )

  RUN_TEST( splitAttrPath0 );
  RUN_TEST( splitAttrPath1 );
  RUN_TEST( splitAttrPath2 );
  RUN_TEST( splitAttrPath3 );
  RUN_TEST( splitAttrPath4 );
  RUN_TEST( splitAttrPath5 );
  RUN_TEST( splitAttrPath6 );

  RUN_TEST( variantJSON0 );
  RUN_TEST( variantJSON1 );
  RUN_TEST( variantJSON2 );
  RUN_TEST( variantJSON3 );

  RUN_TEST( ltrim_copy0 );
  RUN_TEST( rtrim_copy0 );
  RUN_TEST( trim_copy0 );

  RUN_TEST( hasPrefix0 );

  return ec;
}


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
