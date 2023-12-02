/* ========================================================================== *
 *
 * @file flox/exceptions.cc
 *
 * @brief Definitions of various `std::exception` children used for throwing
 *        errors with nice messages and typed discrimination.
 *
 *
 * -------------------------------------------------------------------------- */

#include <map>
#include <optional>
#include <string>

#include <nlohmann/json.hpp>

#include "flox/core/exceptions.hh"


/* -------------------------------------------------------------------------- */

namespace flox {

/* -------------------------------------------------------------------------- */

void
to_json( nlohmann::json & jto, const FloxException & err )
{
  jto = {
    { "exit_code", err.getErrorCode() },
    { "category_message", err.getCategoryMessage() },
  };
  auto contextMsg = err.getContextMessage();
  auto caughtMsg  = err.getCaughtMessage();
  if ( contextMsg.has_value() ) { jto["context_message"] = *contextMsg; };
  if ( caughtMsg.has_value() ) { jto["caught_message"] = *caughtMsg; };
}


/* -------------------------------------------------------------------------- */

}  // namespace flox


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
