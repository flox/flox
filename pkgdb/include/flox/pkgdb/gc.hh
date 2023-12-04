#pragma once

#include <filesystem>
#include <string>


namespace flox::pkgdb {

[[nodiscard]] std::vector<std::filesystem::path>
findStaleDatabases( const std::filesystem::path & cacheDir, int minAgeDays );

}  // namespace flox::pkgdb
