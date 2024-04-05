#include "flox/pkgdb/metrics.hh"
#include <sentry.h>
#include <string>

namespace flox {

void
sentryReporting::init( bool debug )
{
  const std::string  DSN     = "https://"
                               "85c9526795f5047c99a5247aab616295@o4506548203094016."
                               "ingest.us.sentry.io/4506548241825792";
  sentry_options_t * options = sentry_options_new();
  sentry_options_set_dsn( options, DSN.c_str() );
  // This is also the default-path. For further information and recommendations:
  // https://docs.sentry.io/platforms/native/configuration/options/#database-path
  sentry_options_set_database_path( options, ".sentry-native" );
  sentry_options_set_release( options, "my-project-name@2.3.12" );
  sentry_options_set_debug( options, debug ? 1 : 0 );
  sentry_init( options );

  // Example usage for reporting a message
  //   report_message(SENTRY_LEVEL_INFO, "pkgdb", "Hello world from pkgdb!");
}

void
sentryReporting::report_message( const sentry_level_t level,
                                 const std::string &  logger,
                                 const std::string &  message )
{
  sentry_capture_event(
    sentry_value_new_message_event( level, logger.c_str(), message.c_str()
                                    // /*   level */ SENTRY_LEVEL_INFO,
                                    // /*  logger */ "custom",
                                    // /* message */ "It works!"
                                    ) );
}

void
sentryReporting::shutdown()
{
  // make sure everything flushes
  sentry_close();
}

}  // namespace flox