#include "flox/pkgdb/metrics.hh"
#ifdef __linux__
#  include <sentry.h>
#endif
#include "flox/core/util.hh"
#include "flox/pkgdb/read.hh"
#include <string>

namespace flox {

bool            MetricsReporting::initialized = false;
SentryReporting sentryReporting;


std::filesystem::path
getSentryDbDir()
{
  // For further information and recommendations:
  // https://docs.sentry.io/platforms/native/configuration/options/#database-path
  return flox::getFloxCachedir() / ".sentry";
}

void
SentryReporting::init( bool debug )
{
#ifdef __linux__
  // Sentry reporting on Darwin will take more effort, including getting the
  // Sentry libs into nix, as well as looking at the backend needs (breakpad or
  // inproc). See https://github.com/flox/flox/issues/1056 for details.
  std::string dsn;

  if ( const char * dsnVal = std::getenv( "FLOX_SENTRY_DSN" );
       dsnVal != nullptr )
    {
      dsn = dsnVal;
    }
  else
    {
      // If DSN is not set, don't continue initializing Sentry
      debugLog(
        "Environment var FLOX_SENTRY_DSN not set, Sentry is disabled." ) return;
    }

  const char *      env_val = std::getenv( "FLOX_SENTRY_ENV" );
  const std::string env     = ( env_val == nullptr ? "development" : env_val );
  env_val                   = std::getenv( "FLOX_VERSION" );
  const std::string version = ( env_val == nullptr ? "x.y.z" : env_val );

  sentry_options_t * options = sentry_options_new();
  sentry_options_set_dsn( options, dsn.c_str() );
  sentry_options_set_environment( options, env.c_str() );
  sentry_options_set_database_path( options, getSentryDbDir().c_str() );

  // TODO - Get actual version / commit hash ?
  sentry_options_set_release( options, ( "pkgdb@" + version ).c_str() );
  sentry_options_set_debug( options, debug ? 1 : 0 );
  sentry_init( options );

  initialized = true;

  // Example usage for reporting a message
  //   report_message(SENTRY_LEVEL_INFO, "pkgdb", "Hello world from pkgdb!");
  return;

#else
  // This is mainly to get rid of the unused bool warning when building on mac.
  if ( debug ) { debugLog( "Sentry reporting disabled on Darwin." ); }
  return;
#endif
}

#ifdef __linux__
void
SentryReporting::report_message( const sentry_level_t level,
                                 const std::string &  logger,
                                 const std::string &  message )
{
  if ( initialized )
    {
      sentry_capture_event( sentry_value_new_message_event( level,
                                                            logger.c_str(),
                                                            message.c_str() ) );
    }
}
#endif

void
SentryReporting::shutdown()
{
#ifdef __linux__
  // make sure everything flushes
  if ( initialized )
    {
      int res = sentry_close();
      debugLog( nix::fmt( "sentry_close returned %d", res ) );
    }
  initialized = false;
#endif
}

}  // namespace flox
