/* ========================================================================== *
 *
 * @file flox/flake-package.hh
 *
 * @brief Provides a @a flox::Package implementation which are pulled from
 *        evaluation of a `nix` flake.
 *
 *
 * -------------------------------------------------------------------------- */

#pragma once

#include <memory>
#include <optional>
#include <string>
#include <string_view>
#include <utility>
#include <vector>

#include <nix/eval-cache.hh>
#include <nix/names.hh>
#include <nix/ref.hh>
#include <nix/symbol-table.hh>

#include "flox/core/exceptions.hh"
#include "flox/core/types.hh"
#include "flox/package.hh"


/* -------------------------------------------------------------------------- */

namespace flox {

/* -------------------------------------------------------------------------- */

/* Forward declare a friend. */
namespace pkgdb {
class PkgDb;
}

/* -------------------------------------------------------------------------- */

/**
 * @brief A @a flox::Package implementation which are pulled from evaluation of
 *        a `nix` flake.
 */
class FlakePackage : public Package
{

public:

  friend class pkgdb::PkgDb;

private:

  Cursor   _cursor;
  AttrPath _pathS;

  bool _hasMetaAttr    = false;
  bool _hasPnameAttr   = false;
  bool _hasVersionAttr = false;

  std::string                _fullName;
  std::string                _pname;
  std::string                _version;
  std::optional<std::string> _semver;
  System                     _system;
  Subtree                    _subtree;
  std::optional<std::string> _license;


  void
  init( bool checkDrv = true );


  /* --------------------------------------------------------------------------
   */

public:

  ~FlakePackage() override = default;

  FlakePackage( const Cursor & cursor, AttrPath path, bool checkDrv = true )
    : _cursor( cursor )
    , _pathS( std::move( path ) )
    , _fullName( cursor->getAttr( "name" )->getString() )
  {
    {
      nix::DrvName dname( this->_fullName );
      this->_pname   = dname.name;
      this->_version = dname.version;
    }
    this->init( checkDrv );
  }


  FlakePackage( const Cursor &     cursor,
                nix::SymbolTable * symtab,
                bool               checkDrv = true )
    : _cursor( cursor ), _fullName( cursor->getAttr( "name" )->getString() )
  {
    {
      nix::DrvName dname( this->_fullName );
      this->_pname   = dname.name;
      this->_version = dname.version;
    }
    for ( auto & path : symtab->resolve( cursor->getAttrPath() ) )
      {
        this->_pathS.push_back( path );
      }
    this->init( checkDrv );
  }


  /* --------------------------------------------------------------------------
   */
  [[nodiscard]] std::vector<std::string>
  getOutputsToInstall() const override;

  [[nodiscard]] std::optional<bool>
  isBroken() const override;

  [[nodiscard]] std::optional<bool>
  isUnfree() const override;

  [[nodiscard]] AttrPath
  getPathStrs() const override
  {
    return this->_pathS;
  }

  [[nodiscard]] std::string
  getFullName() const override
  {
    return this->_fullName;
  }

  [[nodiscard]] std::string
  getPname() const override
  {
    return this->_pname;
  }

  [[nodiscard]] Cursor
  getCursor() const
  {
    return this->_cursor;
  }
  [[nodiscard]] Subtree

  getSubtreeType() const override
  {
    return this->_subtree;
  }

  [[nodiscard]] nix::DrvName
  getParsedDrvName() const override
  {
    return { this->_fullName };
  }

  [[nodiscard]] std::optional<std::string>
  getVersion() const override
  {
    if ( this->_version.empty() ) { return std::nullopt; }
    return this->_version;
  }

  [[nodiscard]] std::optional<std::string>
  getSemver() const override
  {
    return this->_semver;
  }

  [[nodiscard]] std::optional<std::string>
  getLicense() const override
  {
    if ( this->_license.has_value() ) { return this->_license; }
    return std::nullopt;
  }

  [[nodiscard]] std::vector<std::string>
  getOutputs() const override
  {
    MaybeCursor output = this->_cursor->maybeGetAttr( "outputs" );
    if ( output == nullptr ) { return { "out" }; }
    return output->getListOfStrings();
  }

  [[nodiscard]] std::optional<std::string>
  getDescription() const override
  {
    if ( ! this->_hasMetaAttr ) { return std::nullopt; }
    MaybeCursor description
      = this->_cursor->getAttr( "meta" )->maybeGetAttr( "description" );
    if ( description == nullptr ) { return std::nullopt; }
    try
      {
        return description->getString();
      }
    catch ( ... )
      {
        return std::nullopt;
      }
  }


}; /* End class `FlakePackage' */


/* -------------------------------------------------------------------------- */

/**
 * @class flox::PackageInitException
 * @brief An exception thrown when initializing a @a flox::FlakePackage.
 *
 * @{
 */
FLOX_DEFINE_EXCEPTION( PackageInitException,
                       EC_PACKAGE_INIT,
                       "error initializing FlakePackage" )
/** @} */


/* -------------------------------------------------------------------------- */

}  // namespace flox


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
