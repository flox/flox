/* ========================================================================== *
 *
 *
 *
 * -------------------------------------------------------------------------- */

#include "versions.hh"
#include "test.hh"


/* -------------------------------------------------------------------------- */

bool
test_semverSat1()
{
  std::list<std::string> sats = versions::semverSat(
    "^4.2.0",
    { "4.0.0", "4.2.0", "4.2.1", "4.3.0", "5.0.0", "3.9.9" } );
  return ( sats.size() == 3 )
         && ( std::find( sats.begin(), sats.end(), "4.2.0" ) != sats.end() )
         && ( std::find( sats.begin(), sats.end(), "4.2.1" ) != sats.end() )
         && ( std::find( sats.begin(), sats.end(), "4.3.0" ) != sats.end() );
}


/* -------------------------------------------------------------------------- */

bool
test_isSemver0()
{
  EXPECT( versions::isSemver( "4.2.0" ) );
  EXPECT( versions::isSemver( "4.2.0-pre" ) );
  EXPECT( ! versions::isSemver( "v4.2.0" ) );
  EXPECT( ! versions::isSemver( "v4.2.0-pre" ) );
  return true;
}


/* -------------------------------------------------------------------------- */

/** @brief `%Y-%m-%d` or `%m-%d-%Y` but may contain trailing characters. */
bool
test_isDate0()
{
  EXPECT( versions::isDate( "10-25-1917" ) );
  EXPECT( versions::isDate( "1917-10-25" ) );
  EXPECT( ! versions::isDate( "1917-25-10" ) );

  EXPECT( versions::isDate( "10-25-1917-pre" ) );
  EXPECT( versions::isDate( "1917-10-25-pre" ) );
  EXPECT( ! versions::isDate( "1917-25-10-pre" ) );

  EXPECT( ! versions::isDate( "1917-10-25xxx" ) );

  EXPECT( ! versions::isDate( "10:25:1917" ) );
  EXPECT( ! versions::isDate( "1917:25:10" ) );
  return true;
}


/* -------------------------------------------------------------------------- */

bool
test_isSemverRange0()
{
  EXPECT( versions::isSemverRange( "^4.2.0" ) );
  EXPECT( versions::isSemverRange( "4.2.0" ) );
  EXPECT( versions::isSemverRange( "4.2" ) );
  EXPECT( versions::isSemverRange( "4 - 5" ) );

  EXPECT( ! versions::isSemverRange( "howdy" ) );
  EXPECT( ! versions::isSemverRange( "howdy ^4.2.0" ) );

  /* Globs/special */
  EXPECT( versions::isSemverRange( "" ) );
  EXPECT( versions::isSemverRange( "*" ) );
  EXPECT( versions::isSemverRange( "latest" ) );
  EXPECT( versions::isSemverRange( "any" ) );
  EXPECT( versions::isSemverRange( " * " ) );

  return true;
}


/* -------------------------------------------------------------------------- */

int
main()
{
  int ec = EXIT_SUCCESS;
#define RUN_TEST( ... ) _RUN_TEST( ec, __VA_ARGS__ )

  RUN_TEST( semverSat1 );
  RUN_TEST( isSemver0 );
  RUN_TEST( isDate0 );
  RUN_TEST( isSemverRange0 );

  return ec;
}


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
