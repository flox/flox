/* ========================================================================== *
 *
 * @file nix/run.cc
 *
 * @brief Helpers used by `flox run` subcommand.
 *
 *
 * -------------------------------------------------------------------------- */

#pragma once

#include <string>
#include <optional>

#include <nix/store-api.hh>

/* -------------------------------------------------------------------------- */

namespace nix {

/* -------------------------------------------------------------------------- */

void runProgramInStore(
        ref<Store>                        store
, const std::string                     & program
, const Strings                         & args
,       std::optional<std::string_view>   system = std::nullopt
);

}  /* End namespace `nix' */


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
