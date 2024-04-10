/* ========================================================================== *
 *
 * @file flox/pkgdb/metrics.hh
 *
 * @brief Metrics reporting
 *
 *
 * -------------------------------------------------------------------------- */
#ifdef __linux__
#  include <sentry.h>
#endif
#include <string>

#pragma once

namespace flox {

class MetricsReporting
{
public:

  MetricsReporting() : sentryInitialized( false ) {}
  virtual ~MetricsReporting() = default;

  virtual void
  init( bool debug )
    = 0;

  virtual void
  shutdown()
    = 0;

protected:

  bool sentryInitialized;
};

class SentryReporting : public MetricsReporting
{
public:

  SentryReporting() : MetricsReporting() {}

  virtual void
  init( bool debug );

#ifdef __linux__
  virtual void
  report_message( const sentry_level_t level,
                  const std::string &  logger,
                  const std::string &  message );
#endif

  virtual void
  shutdown();

  virtual ~SentryReporting() {}
};

}  // namespace flox
