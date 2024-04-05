#include "flox/pkgdb/metrics.hh"
#include <sentry.h>
#include <string>

namespace flox {

void sentryReporting::init(bool debug)
{
  const std::string DSN = "https://85c9526795f5047c99a5247aab616295@o4506548203094016.ingest.us.sentry.io/4506548241825792";
  sentry_options_t *options = sentry_options_new();
  sentry_options_set_dsn(options, DSN.c_str());
  // This is also the default-path. For further information and recommendations:
  // https://docs.sentry.io/platforms/native/configuration/options/#database-path
  sentry_options_set_database_path(options, ".sentry-native");
//   sentry_options_set_release(options, "my-project-name@2.3.12");
  sentry_options_set_debug(options, debug ? 1 : 0);
  sentry_init(options);

}

void sentryReporting::report()
{

}

void sentryReporting::shutdown()
{
  // make sure everything flushes
  sentry_close();
}

}