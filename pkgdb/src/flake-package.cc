/* ========================================================================== *
 *
 * @file flake-package.cc
 *
 * @brief Provides a @a flox::Package implementation which are pulled from
 *        evaluation of a `nix` flake.
 *
 *
 * -------------------------------------------------------------------------- */

#include <stdexcept>

#include <nix/eval-cache.hh>

#include "flox/flake-package.hh"
#include "versions.hh"


/* -------------------------------------------------------------------------- */

namespace flox {

/* -------------------------------------------------------------------------- */

void
FlakePackage::init( bool checkDrv )
{
  if ( this->_pathS.size() < 3 )
    {
      throw PackageInitException(
        "Package::init(): Package attribute paths must have at least 3 "
        "elements - the path '"
        + this->_cursor->getAttrPathStr() + "' is too short." );
    }

  if ( checkDrv && ( ! this->_cursor->isDerivation() ) )
    {
      throw PackageInitException(
        "Package::init(): Packages must be derivations but the attrset at '"
        + this->_cursor->getAttrPathStr()
        + "' does not set `.type = \"derivation\"'." );
    }

  /* Subtree type */
  try
    {
      this->_subtree = Subtree::parseSubtree( this->_pathS[0] );
    }
  catch ( const std::invalid_argument & /* unused */ )
    {
      throw PackageInitException( "FlakePackage::init(): Invalid subtree name '"
                                  + this->_pathS[0] + "' at path '"
                                  + this->_cursor->getAttrPathStr() + "'." );
    }

  this->_system = this->_pathS[1];

  MaybeCursor cursor = this->_cursor->maybeGetAttr( "meta" );
  this->_hasMetaAttr = cursor != nullptr;
  if ( cursor != nullptr )
    {
      if ( cursor = cursor->maybeGetAttr( "license" ); cursor != nullptr )
        {
          try
            {
              this->_license = cursor->getAttr( "spdxId" )->getString();
            }
          catch ( ... )
            {}
        }
    }

  cursor = this->_cursor->maybeGetAttr( "pname" );
  if ( cursor != nullptr )
    {
      try
        {
          this->_pname        = cursor->getString();
          this->_hasPnameAttr = true;
        }
      catch ( ... )
        {}
    }

  /* Version and Semver */
  cursor = this->_cursor->maybeGetAttr( "version" );
  if ( cursor != nullptr )
    {
      try
        {
          this->_version        = cursor->getString();
          this->_hasVersionAttr = true;
        }
      catch ( ... )
        {}
    }

  if ( ! this->_version.empty() )
    {
      this->_semver = versions::coerceSemver( this->_version );
    }
}


/* -------------------------------------------------------------------------- */

std::vector<std::string>
FlakePackage::getOutputsToInstall() const
{
  if ( this->_hasMetaAttr )
    {
      MaybeCursor cursor
        = this->_cursor->getAttr( "meta" )->maybeGetAttr( "outputsToInstall" );
      if ( cursor != nullptr ) { return cursor->getListOfStrings(); }
    }
  std::vector<std::string> rsl;
  for ( const std::string & output : this->getOutputs() )
    {
      rsl.push_back( output );
      if ( output == "out" ) { break; }
    }
  return rsl;
}


/* -------------------------------------------------------------------------- */

std::optional<bool>
FlakePackage::isBroken() const
{
  if ( ! this->_hasMetaAttr ) { return std::nullopt; }
  try
    {
      MaybeCursor cursor
        = this->_cursor->getAttr( "meta" )->maybeGetAttr( "broken" );
      if ( cursor == nullptr ) { return std::nullopt; }
      return cursor->getBool();
    }
  catch ( ... )
    {
      return std::nullopt;
    }
}

std::optional<bool>
FlakePackage::isUnfree() const
{
  if ( ! this->_hasMetaAttr ) { return std::nullopt; }
  try
    {
      MaybeCursor cursor
        = this->_cursor->getAttr( "meta" )->maybeGetAttr( "unfree" );
      if ( cursor == nullptr ) { return std::nullopt; }
      return cursor->getBool();
    }
  catch ( ... )
    {
      return std::nullopt;
    }
}


/* -------------------------------------------------------------------------- */

}  // namespace flox


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
