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

  virtual ~FlakePackage() = default;

  FlakePackage( Cursor cursor, const AttrPath & path, bool checkDrv = true )
    : _cursor( cursor )
    , _pathS( path )
    , _fullName( cursor->getAttr( "name" )->getString() )
  {
    {
      nix::DrvName dname( this->_fullName );
      this->_pname   = dname.name;
      this->_version = dname.version;
    }
    this->init( checkDrv );
  }


  FlakePackage( Cursor cursor, nix::SymbolTable * symtab, bool checkDrv = true )
    : _cursor( cursor ), _fullName( cursor->getAttr( "name" )->getString() )
  {
    {
      nix::DrvName dname( this->_fullName );
      this->_pname   = dname.name;
      this->_version = dname.version;
    }
    for ( auto & p : symtab->resolve( cursor->getAttrPath() ) )
      {
        this->_pathS.push_back( p );
      }
    this->init( checkDrv );
  }


  /* --------------------------------------------------------------------------
   */

  std::vector<std::string>
  getOutputsToInstall() const override;
  std::optional<bool>
  isBroken() const override;
  std::optional<bool>
  isUnfree() const override;

  AttrPath
  getPathStrs() const override
  {
    return this->_pathS;
  }
  std::string
  getFullName() const override
  {
    return this->_fullName;
  }
  std::string
  getPname() const override
  {
    return this->_pname;
  }
  Cursor
  getCursor() const
  {
    return this->_cursor;
  }
  Subtree
  getSubtreeType() const override
  {
    return this->_subtree;
  }

  nix::DrvName
  getParsedDrvName() const override
  {
    return nix::DrvName( this->_fullName );
  }

  std::optional<std::string>
  getVersion() const override
  {
    if ( this->_version.empty() ) { return std::nullopt; }
    else { return this->_version; }
  }

  std::optional<std::string>
  getSemver() const override
  {
    return this->_semver;
  }

  std::optional<std::string>
  getLicense() const override
  {
    if ( this->_license.has_value() ) { return this->_license; }
    else { return std::nullopt; }
  }

  std::vector<std::string>
  getOutputs() const override
  {
    MaybeCursor o = this->_cursor->maybeGetAttr( "outputs" );
    if ( o == nullptr ) { return { "out" }; }
    else { return o->getListOfStrings(); }
  }

  std::optional<std::string>
  getDescription() const override
  {
    if ( ! this->_hasMetaAttr ) { return std::nullopt; }
    MaybeCursor l
      = this->_cursor->getAttr( "meta" )->maybeGetAttr( "description" );
    if ( l == nullptr ) { return std::nullopt; }
    try
      {
        return l->getString();
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
