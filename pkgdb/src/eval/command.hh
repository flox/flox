/* ========================================================================== *
 *
 * @file flox/eval.hh
 *
 * @brief Executable command helpers, argument parsers, etc.
 *
 *
 * -------------------------------------------------------------------------- */

#pragma once

#include <filesystem>

#include <argparse/argparse.hpp>

#include "flox/core/command.hh"
#include "flox/core/nix-state.hh"


/* -------------------------------------------------------------------------- */

namespace flox {

/* -------------------------------------------------------------------------- */

/** @brief Evaluate a `nix` expression with `flox` extensions. */
class EvalCommand : flox::NixState
{

private:

  command::VerboseParser parser;

  enum output_style {
    STYLE_VALUE, /**< Emit `nix` values. */
    STYLE_RAW,   /**< Emit strings without quotes. */
    STYLE_JSON   /**< Emit JSON. */
  };             /* End enum `output_style' */
  output_style style = STYLE_VALUE;

  std::optional<std::filesystem::path> file;
  std::optional<std::string>           expr;


public:

  EvalCommand();

  [[nodiscard]] command::VerboseParser &
  getParser()
  {
    return this->parser;
  }

  /**
   * @brief Execute the `eval` routine.
   * @return `EXIT_SUCCESS` or `EXIT_FAILURE`.
   */
  int
  run();


}; /* End class `EvalCommand' */


/* -------------------------------------------------------------------------- */

}  // namespace flox


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
