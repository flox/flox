/* ========================================================================== *
 *
 * @file flox/pkgdb/metrics.hh
 *
 * @brief Metrics reporting
 *
 *
 * -------------------------------------------------------------------------- */

#pragma once

namespace flox {

class metricsReporting
{
public:

    metricsReporting() {}
    virtual ~metricsReporting() = default;

    virtual void init(bool debug) = 0;
    virtual void report() = 0;
    virtual void shutdown() = 0;

};

class sentryReporting : public metricsReporting
{
public:

    sentryReporting() : metricsReporting() {}

    virtual void init(bool debug);
    virtual void report();
    virtual void shutdown();

    virtual ~sentryReporting() { shutdown(); }

};

}