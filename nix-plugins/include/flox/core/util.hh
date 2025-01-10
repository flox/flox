/* ========================================================================== *
 *
 * @file flox/core/util.hh
 *
 * @brief Miscellaneous helper functions.
 *
 *
 * -------------------------------------------------------------------------- */

#pragma once

#include <nix/util.hh>

/* -------------------------------------------------------------------------- */

/* Backported from C++20a for C++20b compatibility. */

/**
 * @brief Helper type for `std::visit( overloaded { ... }, x );` pattern.
 *
 * This is a _quality of life_ helper that shortens boilerplate required for
 * creating type matching statements.
 */
template<class... Ts>
struct overloaded : Ts...
{
  using Ts::operator()...;
};

template<class... Ts>
overloaded( Ts... ) -> overloaded<Ts...>;


/* -------------------------------------------------------------------------- */

namespace flox {

/* -------------------------------------------------------------------------- */

/** @brief Print a log message with the provided log level.
 *
 * This is a macro so that any allocations needed for msg can be optimized out.
 */
#define printLog( lvl, msg )                                                                               \
  /* See                                                                                                   \
   * https://github.com/NixOS/nix/blob/09a6e8e7030170611a833612b9f40b9a10778c18/src/libutil/logging.cc#L64 \
   * for lvl to verbosity comparison                                                                       \
   */                                                                                                      \
  if ( ! ( ( lvl ) > nix::verbosity ) ) { nix::logger->log( lvl, msg ); }

/** @brief Prints a log message to `stderr` when called with `-vvvv`. */
#define traceLog( msg ) printLog( nix::Verbosity::lvlVomit, msg )

/**
 * @brief Prints a log message to `stderr` when called with `--debug` or `-vvv`.
 */
#define debugLog( msg ) printLog( nix::Verbosity::lvlDebug, msg )

/**
 * @brief Prints a log message to `stderr` when called with `--verbose` or `-v`.
 */
#define verboseLog( msg ) printLog( nix::Verbosity::lvlTalkative, msg )

/** @brief Prints a log message to `stderr` at default verbosity. */
#define infoLog( msg ) printLog( nix::Verbosity::lvlInfo, msg )

/** @brief Prints a log message to `stderr` when verbosity is at least `-q`. */
#define warningLog( msg ) printLog( nix::Verbosity::lvlWarn, msg )

/** @brief Prints a log message to `stderr` when verbosity is at least `-qq`. */
#define errorLog( msg ) printLog( nix::Verbosity::lvlError, msg )

/* -------------------------------------------------------------------------- */

}  // namespace flox


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
