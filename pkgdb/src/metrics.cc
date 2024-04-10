#include "flox/pkgdb/metrics.hh"

#ifndef __APPLE__
#  include <sentry.h>
#endif
#include <string>

namespace flox {

void
sentryReporting::init( bool debug )
{
// Sentry reporting on Darwin will take more effort, including getting the
// Sentry libs into nix, as well as looking at the backend needs (breakpad or
// inproc). See https://github.com/flox/flox/issues/1056 for details.
#ifndef __APPLE__
  std::string dsn;

  if ( const char * dsnVal = std::getenv( "FLOX_SENTRY_DSN" );
       dsnVal != nullptr )
    {
      dsn = dsnVal;
    }
  else
    {
      // If DSN is not set, don't continue initializing Sentry
      return;
    }

  const char *      env_val = std::getenv( "FLOX_SENTRY_ENV" );
  const std::string env     = ( env_val == nullptr ? "development" : env_val );
  env_val                   = std::getenv( "FLOX_VERSION" );
  const std::string version = ( env_val == nullptr ? "x.y.z" : env_val );

  sentry_options_t * options = sentry_options_new();
  sentry_options_set_dsn( options, dsn.c_str() );
  sentry_options_set_environment( options, env.c_str() );

  // This is also the default-path. For further information and recommendations:
  // https://docs.sentry.io/platforms/native/configuration/options/#database-path
  sentry_options_set_database_path( options, ".sentry-native" );

  // TODO - Get actual version / commit hash ?
  sentry_options_set_release( options, ( "pkgdb@" + version ).c_str() );
  sentry_options_set_debug( options, debug ? 1 : 0 );
  sentry_init( options );

  sentryInitialized = true;

  if ( std::getenv( "_FLOX_TEST_SENTRY_CRASH" ) != nullptr ) { abort(); }

// Example usage for reporting a message
//   report_message(SENTRY_LEVEL_INFO, "pkgdb", "Hello world from pkgdb!");
#endif
}

void
sentryReporting::report_message( const sentry_level_t level,
                                 const std::string &  logger,
                                 const std::string &  message )
{
#ifndef __APPLE__
  if ( sentryInitialized )
    {
      sentry_capture_event( sentry_value_new_message_event( level,
                                                            logger.c_str(),
                                                            message.c_str() ) );
    }
#endif
}

void
sentryReporting::shutdown()
{
#ifndef __APPLE__
  // make sure everything flushes
  if ( sentryInitialized ) { sentry_close(); }
  sentryInitialized = false;
#endif
}

}  // namespace flox