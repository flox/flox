/* ========================================================================== *
 *
 * @file flox/pkgdb/gc.cc
 *
 * @brief `flox` garbage collection.
 *
 *
 * -------------------------------------------------------------------------- */

#pragma once

#include <filesystem>
#include <string>
#include <vector>


/* -------------------------------------------------------------------------- */

namespace flox::pkgdb {

/* -------------------------------------------------------------------------- */

/** @brief Find all stale databases in the cache directory. */
[[nodiscard]] std::vector<std::filesystem::path>
findStaleDatabases( const std::filesystem::path & cacheDir, int minAgeDays );


/* -------------------------------------------------------------------------- */

}  // namespace flox::pkgdb


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
