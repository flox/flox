/* ========================================================================== *
 *
 * @file flox/core/nix-state.hh
 *
 * @brief Manages a `nix` runtime state blob with associated helpers.
 *
 *
 * -------------------------------------------------------------------------- */

#pragma once

#include <memory>

#include <nix/eval.hh>
#include <nix/ref.hh>
#include <nix/repair-flag.hh>
#include <nix/search-path.hh>
#include <nix/store-api.hh>


/* -------------------------------------------------------------------------- */

/* Forward Declarations. */

namespace nix {
class Logger;
}


/* -------------------------------------------------------------------------- */

namespace flox {

/* -------------------------------------------------------------------------- */

/** @brief Create a custom `nix::Logger` which ignores some messages. */
nix::Logger *
makeFilteredLogger( bool printBuildLogs );


/* -------------------------------------------------------------------------- */

/**
 * @brief Perform one time `nix` global runtime setup.
 *
 * You may safely call this function multiple times, after the first invocation
 * it is effectively a no-op.
 *
 * This replaces the default `nix::Logger` with a @a flox::FilteredLogger.
 */
void
initNix();


/* -------------------------------------------------------------------------- */

/** @brief Mixin which provides a lazy handle to a `nix` store connection. */
class NixStoreMixin
{

private:

  std::shared_ptr<nix::Store> store; /**< `nix` store connection.   */


public:

  /* Copy/Move base class boilerplate */
  NixStoreMixin( const NixStoreMixin & ) = default;
  NixStoreMixin( NixStoreMixin && )      = default;

  virtual ~NixStoreMixin() = default;

  NixStoreMixin &
  operator=( const NixStoreMixin & )
    = default;
  NixStoreMixin &
  operator=( NixStoreMixin && )
    = default;


  /**
   * @brief Construct `NixStoreMixin` from an existing store connection.
   *
   * This may be useful if you wish to avoid a non-default store.
   * @param store An open `nix` store connection.
   */
  explicit NixStoreMixin( const nix::ref<nix::Store> & store )
    : store( static_cast<std::shared_ptr<nix::Store>>( store ) )
  {
    initNix();
  }

  /**
   * @brief Construct `NixStoreMixin` using the systems default `nix` store.
   */
  NixStoreMixin() { initNix(); }


  /**
   * @brief Lazily open a `nix` store connection.
   *
   * Connection remains open for lifetime of object.
   */
  nix::ref<nix::Store>
  getStore()
  {
    if ( this->store == nullptr ) { this->store = nix::openStore(); }
    return static_cast<nix::ref<nix::Store>>( this->store );
  }


}; /* End class `NixStoreMixin' */


/* -------------------------------------------------------------------------- */

/**
 * @brief Runtime state containing a `nix` store connection and a
 *        `nix` evaluator.
 */
class NixState : public NixStoreMixin
{

private:

  /* From `NixStoreMixin':
   *   std::shared_ptr<nix::Store> store
   */

  std::shared_ptr<nix::EvalState> state; /**< `nix` evaluator instance. */


public:

  /** @brief Construct `NixState` using the systems default `nix` store. */
  NixState() = default;

  /**
   * @brief Construct `NixState` from an existing store connection.
   *
   * This may be useful if you wish to avoid a non-default store.
   * @param store An open `nix` store connection.
   */
  explicit NixState( nix::ref<nix::Store> & store ) : NixStoreMixin( store ) {}


  /**
   * @brief Lazily open a `nix` evaluator.
   *
   * Evaluator remains open for lifetime of object.
   */
  nix::ref<nix::EvalState>
  getState()
  {
    if ( this->state == nullptr )
      {
        this->state = std::make_shared<nix::EvalState>( nix::SearchPath(),
                                                        this->getStore(),
                                                        this->getStore() );
        this->state->repair = nix::NoRepair;
      }
    return static_cast<nix::ref<nix::EvalState>>( this->state );
  }


}; /* End class `NixState' */


/* -------------------------------------------------------------------------- */

}  // namespace flox


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
