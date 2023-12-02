/* ========================================================================== *
 *
 * @file flox/parse/command.hh
 *
 * @brief Executable command helpers, argument parsers, etc.
 *
 *
 * -------------------------------------------------------------------------- */

#pragma once

#include "flox/pkgdb/command.hh"
#include "flox/resolver/descriptor.hh"


/* -------------------------------------------------------------------------- */

/** @brief Interfaces used to parse various `pkgdb` constructs. */
namespace flox::parse {

/* -------------------------------------------------------------------------- */

/** @brief Parse a descriptor into a set of @a flox::pkgdb::PkgQueryArgs. */
class DescriptorCommand
{

private:

  command::VerboseParser parser; /**< Query arguments and inputs parser */

  resolver::ManifestDescriptor descriptor;

  std::string format
    = "manifest"; /** Allowed values are "manifest" and "query" */

public:

  DescriptorCommand();

  [[nodiscard]] command::VerboseParser &
  getParser()
  {
    return this->parser;
  }

  /**
   * @brief Execute the `descriptor` routine.
   * @return `EXIT_SUCCESS` or `EXIT_FAILURE`.
   */
  int
  run();


}; /* End class `DescriptorCommand' */


/* -------------------------------------------------------------------------- */

/** @brief Parse various constructs. */
class ParseCommand
{

private:

  command::VerboseParser parser; /**< Query arguments and inputs parser */

  DescriptorCommand cmdDescriptor;


public:

  ParseCommand();

  [[nodiscard]] command::VerboseParser &
  getParser()
  {
    return this->parser;
  }

  /**
   * @brief Execute the `parse` routine.
   * @return `EXIT_SUCCESS` or `EXIT_FAILURE`.
   */
  int
  run();


}; /* End class `ParseCommand' */


/* -------------------------------------------------------------------------- */

}  // namespace flox::parse


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
