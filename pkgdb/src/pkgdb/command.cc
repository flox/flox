/* ========================================================================== *
 *
 * @file pkgdb/command.cc
 *
 * @brief Executable command helpers, argument parsers, etc.
 *
 *
 * -------------------------------------------------------------------------- */

#include <filesystem>
#include <memory>
#include <optional>
#include <string>
#include <string_view>
#include <variant>

#include <argparse/argparse.hpp>
#include <nix/flake/flake.hh>
#include <nix/ref.hh>
#include <nix/types.hh>
#include <nix/util.hh>

#include "flox/core/command.hh"
#include "flox/core/exceptions.hh"
#include "flox/core/util.hh"
#include "flox/flox-flake.hh"
#include "flox/pkgdb/command.hh"
#include "flox/pkgdb/read.hh"
#include "flox/pkgdb/write.hh"
#include "flox/registry.hh"


/* -------------------------------------------------------------------------- */

/* Forward Declarations. */

namespace nix {
class Store;
}


/* -------------------------------------------------------------------------- */

namespace flox::pkgdb {

/* -------------------------------------------------------------------------- */

argparse::Argument &
DbPathMixin::addDatabasePathOption( argparse::ArgumentParser & parser )
{
  return parser.add_argument( "-d", "--database" )
    .help( "use database at PATH" )
    .metavar( "PATH" )
    .nargs( 1 )
    .action(
      [&]( const std::string & dbPath )
      {
        this->dbPath = nix::absPath( dbPath );
        std::filesystem::create_directories( this->dbPath->parent_path() );
      } );
}


/* -------------------------------------------------------------------------- */

template<>
void
PkgDbMixin<PkgDb>::openPkgDb()
{
  if ( this->db != nullptr ) { return; } /* Already loaded. */
  if ( ( this->flake != nullptr ) && this->dbPath.has_value() )
    {
      this->db
        = std::make_shared<PkgDb>( this->flake->lockedFlake,
                                   static_cast<std::string>( *this->dbPath ) );
    }
  else if ( this->flake != nullptr )
    {
      this->dbPath = flox::pkgdb::genPkgDbName(
        this->flake->lockedFlake.getFingerprint() );
      std::filesystem::create_directories( this->dbPath->parent_path() );
      this->db
        = std::make_shared<PkgDb>( this->flake->lockedFlake,
                                   static_cast<std::string>( *this->dbPath ) );
    }
  else if ( this->dbPath.has_value() )
    {
      std::filesystem::create_directories( this->dbPath->parent_path() );
      this->db
        = std::make_shared<PkgDb>( static_cast<std::string>( *this->dbPath ) );
    }
  else
    {
      throw flox::FloxException(
        "You must provide either a path to a database, or a flake-reference." );
    }
}


/* -------------------------------------------------------------------------- */

template<>
void
PkgDbMixin<PkgDbReadOnly>::openPkgDb()
{
  if ( this->db != nullptr ) { return; } /* Already loaded. */

  if ( ! this->dbPath.has_value() )
    {
      if ( this->flake == nullptr )
        {
          throw flox::FloxException(
            "You must provide either a path to a database, or "
            "a flake-reference." );
        }
      this->dbPath = flox::pkgdb::genPkgDbName(
        this->flake->lockedFlake.getFingerprint() );
    }

  /* Initialize empty DB if none exists. */
  if ( ! std::filesystem::exists( this->dbPath.value() ) )
    {
      if ( this->flake != nullptr )
        {
          std::filesystem::create_directories( this->dbPath->parent_path() );
          flox::pkgdb::PkgDb pdb( this->flake->lockedFlake,
                                  static_cast<std::string>( *this->dbPath ) );
        }
    }

  if ( this->flake != nullptr )
    {
      this->db = std::make_shared<PkgDbReadOnly>(
        this->flake->lockedFlake.getFingerprint(),
        static_cast<std::string>( *this->dbPath ) );
    }
  else
    {
      this->db = std::make_shared<PkgDbReadOnly>(
        static_cast<std::string>( *this->dbPath ) );
    }
}


/* -------------------------------------------------------------------------- */

template<pkgdb_typename T>
argparse::Argument &
PkgDbMixin<T>::addTargetArg( argparse::ArgumentParser & parser )
{
  return parser.add_argument( "target" )
    .help( "the source ( database path or flake-ref ) to read" )
    .required()
    .metavar( "<DB-PATH|FLAKE-REF>" )
    .action(
      [&]( const std::string & target )
      {
        if ( flox::isSQLiteDb( target ) )
          {
            this->dbPath = nix::absPath( target );
          }
        else /* flake-ref */
          {
            try
              {
                this->parseFlakeRef( target );
                nix::ref<nix::Store> store = this->getStore();
                FloxFlakeInput       input( store, this->getRegistryInput() );
                this->flake
                  = static_cast<std::shared_ptr<FloxFlake>>( input.getFlake() );
              }
            catch ( ... )
              {
                if ( std::filesystem::exists( target ) )
                  {
                    throw command::InvalidArgException(
                      "Argument '" + target
                      + "' is neither a flake "
                        "reference nor SQLite3 database" );
                  }
                throw;
              }
          }
      } );
}


/* -------------------------------------------------------------------------- */

template struct PkgDbMixin<PkgDbReadOnly>;
template struct PkgDbMixin<PkgDb>;


/* -------------------------------------------------------------------------- */

}  // namespace flox::pkgdb


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
