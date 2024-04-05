/* ========================================================================== *
 *
 * @file flox/pkgdb/metrics.hh
 *
 * @brief Metrics reporting
 *
 *
 * -------------------------------------------------------------------------- */
#include <sentry.h>
#include <string>

#pragma once

namespace flox {

class metricsReporting
{
public:

  metricsReporting() {}
  virtual ~metricsReporting() = default;

  virtual void
  init( bool debug )
    = 0;

  virtual void
  shutdown()
    = 0;
};

class sentryReporting : public metricsReporting
{
public:

  sentryReporting() : metricsReporting() {}

  virtual void
  init( bool debug );

  virtual void
  report_message( const sentry_level_t level,
                  const std::string &  logger,
                  const std::string &  message );

  virtual void
  shutdown();

  virtual ~sentryReporting() { shutdown(); }
};

}  // namespace flox