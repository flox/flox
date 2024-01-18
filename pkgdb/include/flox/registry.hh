/* ========================================================================== *
 *
 * @file flox/registry.hh
 *
 * @brief A set of user inputs used to set input preferences during search
 *        and resolution.
 *
 *
 * -------------------------------------------------------------------------- */

#pragma once

#include <algorithm>
#include <functional>
#include <map>
#include <vector>

#include <nix/fetchers.hh>
#include <nix/flake/flakeref.hh>
#include <nlohmann/json.hpp>

#include "compat/concepts.hh"
#include "flox/core/exceptions.hh"
#include "flox/core/types.hh"
#include "flox/core/util.hh"
#include "flox/flox-flake.hh"
#include "flox/pkgdb/pkg-query.hh"


/* -------------------------------------------------------------------------- */

namespace flox {

/* -------------------------------------------------------------------------- */

/** @brief Preferences associated with a registry input. */
struct InputPreferences
{

  /**
   * Ordered list of subtrees to be searched.
   * Results will be grouped by subtree in the order they appear here.
   */
  std::optional<std::vector<Subtree>> subtrees;

  /* Base class boilerplate */
  virtual ~InputPreferences()                  = default;
  InputPreferences()                           = default;
  InputPreferences( const InputPreferences & ) = default;
  InputPreferences( InputPreferences && )      = default;

  InputPreferences &
  operator=( const InputPreferences & )
    = default;
  InputPreferences &
  operator=( InputPreferences && )
    = default;

  explicit InputPreferences(
    const std::optional<std::vector<Subtree>> & subtrees )
    : subtrees( subtrees )
  {}

  /** @brief Reset to default state. */
  virtual void
  clear();

  /**
   * @brief Fill a @a flox::pkgdb::PkgQueryArgs struct with preferences to
   *        lookup packages filtered by @a InputPreferences requirements.
   *
   * NOTE: This DOES NOT clear @a pqa before filling it.
   * This is intended to be used after filling @a pqa with global preferences.
   * @param pqa A set of query args to _fill_ with preferences.
   * @return A reference to the modified query args.
   */
  pkgdb::PkgQueryArgs &
  fillPkgQueryArgs( pkgdb::PkgQueryArgs & pqa ) const;

  /**
   * @brief Merges this @a flox::InputPreferences with another InputPreferences,
   *        overriding values in this InputPreferences with those from the other
   *        InputPreferences.
   * @param overrides A @a flox::InputPreferences to merge and override with.
   */
  void
  merge( const InputPreferences & overrides );

  [[nodiscard]] bool
  operator==( const InputPreferences & other ) const
  {
    return this->subtrees == other.subtrees;
  }

  [[nodiscard]] bool
  operator!=( const InputPreferences & other ) const
  {
    return ! ( ( *this ) == other );
  }


}; /* End struct `InputPreferences' */


/* -------------------------------------------------------------------------- */

/** @brief Convert a JSON object to an @a flox::InputPreferences. */
void
from_json( const nlohmann::json & jfrom, InputPreferences & prefs );

/** @brief Convert an @a flox::InputPreferences to a JSON object. */
void
to_json( nlohmann::json & jto, const InputPreferences & prefs );


/* -------------------------------------------------------------------------- */

/**
 * @brief Restricts types to those which are derived from
 *        @a flox::InputPreferences.
 */
template<typename T>
concept input_preferences_typename
  = std::is_base_of<InputPreferences, T>::value && requires( T obj ) {
      {
        obj.getFlakeRef()
      } -> std::convertible_to<nix::ref<nix::FlakeRef>>;
    };


/* -------------------------------------------------------------------------- */

/** @brief Preferences associated with a named registry input. */
struct RegistryInput : public InputPreferences
{

  /* From `InputPreferences':
   *   std::optional<std::vector<Subtree>> subtrees;
   */

  std::shared_ptr<nix::FlakeRef> from; /**< A parsed flake reference. */

  RegistryInput() = default;

  RegistryInput( const std::optional<std::vector<Subtree>> & subtrees,
                 const nix::FlakeRef &                       from )
    : InputPreferences( subtrees )
  {
    if ( isNixpkgsRef( from ) )
      {
        auto patched = from;
        patched.input.attrs.insert_or_assign( "type", FLOX_FLAKE_TYPE );
        this->from = std::make_shared<nix::FlakeRef>( patched );
      }
    else { this->from = std::make_shared<nix::FlakeRef>( from ); }
  }

  explicit RegistryInput( const nix::FlakeRef & from )
  {
    if ( isNixpkgsRef( from ) )
      {
        auto patched = from;
        patched.input.attrs.insert_or_assign( "type", FLOX_FLAKE_TYPE );
        this->from = std::make_shared<nix::FlakeRef>( patched );
      }
    else { this->from = std::make_shared<nix::FlakeRef>( from ); }
  }


  /** @brief Get the flake reference associated with this input. */
  [[nodiscard]] nix::ref<nix::FlakeRef>
  getFlakeRef() const
  {
    return static_cast<nix::ref<nix::FlakeRef>>( this->from );
  };

  [[nodiscard]] bool
  operator==( const RegistryInput & other ) const
  {
    if ( static_cast<InputPreferences>( *this )
         != static_cast<InputPreferences>( other ) )
      {
        return false;
      }

    if ( this->from == other.from ) { return true; }

    if ( ( this->from == nullptr ) || ( other.from == nullptr ) )
      {
        return false;
      }

    return ( *this->from ) == ( *other.from );
  }

  [[nodiscard]] bool
  operator!=( const RegistryInput & other ) const
  {
    return ! ( ( *this ) == other );
  }


}; /* End struct `RegistryInput' */


/* -------------------------------------------------------------------------- */

/** @brief Convert a JSON object to a @a flox::RegistryInput. */
void
from_json( const nlohmann::json & jfrom, RegistryInput & rip );

/** @brief Convert a @a flox::RegistryInput to a JSON object. */
void
to_json( nlohmann::json & jto, const RegistryInput & rip );


/* -------------------------------------------------------------------------- */

/**
 * @brief Restricts types to those which can construct
 *        @a flox::RegistryInput values.
 *
 * A @a flox::registry_input_factory is a type that can construct
 * @a flox::input_preferences_typename instances.
 *
 * A @a flox::registry_input_factory must provide a
 * `using input_type = <TYPENAME>;` declaration to indicate the type of
 * @a flox::input_preferences_typename it produces.
 * It must also provide a `mkInput` function that constructs an instance of the
 * declared @a input_type from a name and a @a flox::RegistryInput.
 *
 * @see flox::RegistryInputFactory
 * @see flox::FloxFlakeInputFactory
 * @see flox::pkgdb::PkgDbInputFactory
 */
template<typename T>
concept registry_input_factory =
  /* Must have a `using input_type = <TYPENAME>;' definition. */
  requires { typename T::input_type; } &&
  /* `input_type' must satisfy `input_preferences_typesname' concept. */
  input_preferences_typename<typename T::input_type> &&
  /* Must provide a function, `mkInput' that constructs an `input_type'. */
  requires( T ref, const std::string & name, const RegistryInput & rip ) {
    {
      ref.mkInput( name, rip )
    } -> std::convertible_to<std::shared_ptr<typename T::input_type>>;
  };


/* -------------------------------------------------------------------------- */

/**
 * @brief The simplest @a flox::RegistryInput _factory_ which just
 *        copies inputs.
 */
class RegistryInputFactory
{

public:

  using input_type = RegistryInput;

  /** @brief Construct an input from a @a flox::RegistryInput. */
  [[nodiscard]] static std::shared_ptr<RegistryInput>
  mkInput( const std::string & /* unused */, const RegistryInput & input )
  {
    return std::make_shared<RegistryInput>( input );
  }


}; /* End class `RegistryInputFactory' */

static_assert( registry_input_factory<RegistryInputFactory> );


/* -------------------------------------------------------------------------- */

/**
 * @brief A set of user inputs used to set input preferences during search
 *        and resolution.
 *
 * Example Registry:
 * ```
 * {
 *   "inputs": {
 *     "nixpkgs": {
 *       "from": {
 *         "type": "github"
 *       , "owner": "NixOS"
 *       , "repo": "nixpkgs"
 *       }
 *     , "subtrees": ["legacyPackages"]
 *     }
 *   , "floco": {
 *       "from": {
 *         "type": "github"
 *       , "owner": "aakropotkin"
 *       , "repo": "floco"
 *       }
 *     , "subtrees": ["packages"]
 *     }
 *   }
 * , "defaults": {
 *     "subtrees": null
 *   }
 * , "priority": ["nixpkgs", "floco"]
 * }
 * ```
 */
struct RegistryRaw
{

  /** Settings and fetcher information associated with named inputs. */
  std::map<std::string, RegistryInput> inputs;

  /** Default/fallback settings for inputs. */
  InputPreferences defaults;

  /**
   * Priority order used to process inputs.
   * Inputs which do not appear in this list are handled in lexicographical
   * order after any explicitly named inputs.
   */
  std::vector<std::string> priority;


  /* Base class boilerplate. */
  virtual ~RegistryRaw()             = default;
  RegistryRaw()                      = default;
  RegistryRaw( const RegistryRaw & ) = default;
  RegistryRaw( RegistryRaw && )      = default;

  RegistryRaw &
  operator=( const RegistryRaw & )
    = default;
  RegistryRaw &
  operator=( RegistryRaw && )
    = default;

  explicit RegistryRaw( std::map<std::string, RegistryInput> inputs,
                        InputPreferences                     defaults = {},
                        std::vector<std::string>             priority = {} )
    : inputs( std::move( inputs ) )
    , defaults( std::move( defaults ) )
    , priority( std::move( priority ) )
  {}

  explicit RegistryRaw( std::map<std::string, RegistryInput> inputs,
                        std::vector<std::string>             priority = {} )
    : RegistryRaw( std::move( inputs ),
                   InputPreferences {},
                   std::move( priority ) )
  {}

  /**
   * @brief Return an ordered list of input names.
   *
   * This appends @a priority with any missing @a inputs in
   * lexicographical order.
   *
   * The resulting list contains wrapped references and need to be accessed
   * using @a std::reference_wrapper<T>::get().
   *
   * Example:
   * ```
   * Registry reg = R"( {
   *   "inputs": {
   *     "floco": {
   *       "from": { "type": "github", "owner": "aakropotkin", "repo": "floco" }
   *     }
   *   , "nixpkgs": {
   *       "from": { "type": "github", "owner": "NixOS", "repo": "nixpkgs" }
   *     }
   *   }
   * , "priority": ["nixpkgs"]
   * } )"_json;
   * for ( const auto & name : reg.getOrder() )
   *   {
   *     std::cout << name.get() << " ";
   *   }
   * std::cout << std::endl;
   * // => nixpkgs floco
   * ```
   *
   * @return A list of input names in order of priority.
   */
  [[nodiscard]] virtual std::vector<std::reference_wrapper<const std::string>>
  getOrder() const;

  /** @brief Reset to default state. */
  virtual void
  clear();

  /**
   * @brief Fill a @a flox::pkgdb::PkgQueryArgs struct with preferences to
   *        lookup packages in a particular input.
   * @param input The input name to be searched.
   * @param pqa   A set of query args to _fill_ with preferences.
   * @return A reference to the modified query args.
   */
  pkgdb::PkgQueryArgs &
  fillPkgQueryArgs( const std::string &   input,
                    pkgdb::PkgQueryArgs & pqa ) const;

  /**
   * @brief Merge this @a flox::RegistryRaw struct with another
   *        @a flox::RegistryRaw, overriding any existing values with those from
   *        the other RegistryRaw
   *
   */
  void
  merge( const RegistryRaw & overrides );

  [[nodiscard]] bool
  operator==( const RegistryRaw & other ) const;

  [[nodiscard]] bool
  operator!=( const RegistryRaw & other ) const
  {
    return ! ( *this == other );
  }


}; /* End struct `RegistryRaw' */


/* -------------------------------------------------------------------------- */

/** @brief Convert a JSON object to a @a flox::RegistryRaw. */
void
from_json( const nlohmann::json & jfrom, RegistryRaw & reg );

/** @brief Convert a @a flox::RegistryRaw to a JSON object. */
void
to_json( nlohmann::json & jto, const RegistryRaw & reg );

/* -------------------------------------------------------------------------- */

/**
 * @class flox::InvalidRegistryException
 * @brief An exception thrown when a registry has invalid contents.
 * @{
 */
FLOX_DEFINE_EXCEPTION( InvalidRegistryException,
                       EC_INVALID_REGISTRY,
                       "invalid registry" )
/** @} */


/* -------------------------------------------------------------------------- */

/**
 * @brief An input registry that may hold arbitrary types of inputs.
 *
 * Unlike @a flox::RegistryRaw, inputs are held in order, and any default
 * settings have been applied to inputs.
 *
 * Any type that is constructible from a @a flox::RegistryInput and (optional) a
 * `nix::ref<nix::EvalState>`, and is derived from @a flox::InputPreferences may
 * be a value type in a registry.
 */
template<registry_input_factory FactoryType>
class Registry
{

private:

  /**
   * Orginal raw registry.
   * This is saved to allow the raw user input to be recorded in lockfiles.
   */
  RegistryRaw registryRaw;

  /** A list of `<SHORTNAME>, <FLAKE>` pairs in priority order. */
  std::vector<
    std::pair<std::string, std::shared_ptr<typename FactoryType::input_type>>>
    inputs;


public:

  using input_type = typename FactoryType::input_type;


  /**
   * @brief Construct a registry from a @a flox::RegistryRaw and
   *        a _factory_.
   */
  explicit Registry( RegistryRaw registry, FactoryType & factory )
    : registryRaw( std::move( registry ) )
  {
    for ( const std::reference_wrapper<const std::string> & _name :
          this->registryRaw.getOrder() )
      {
        const auto & pair = std::find_if(
          this->registryRaw.inputs.begin(),
          this->registryRaw.inputs.end(),
          [&]( const auto & pair ) { return pair.first == _name.get(); } );

        /* Throw an exception if a registry input is an indirect reference. */
        if ( pair->second.getFlakeRef()->input.getType() == "indirect" )
          {
            throw InvalidRegistryException(
              "registry contained an indirect reference" );
          }

        /* Replace any nixpkgs refs with our own. */


        /* Fill default/fallback values if none are defined. */
        RegistryInput input = pair->second;
        if ( ! input.subtrees.has_value() )
          {
            input.subtrees = this->registryRaw.defaults.subtrees;
          }


        /* Construct the input */
        this->inputs.emplace_back(
          std::make_pair( pair->first,
                          factory.mkInput( pair->first, input ) ) );
      }
  }

  /**
   * @brief Get an input by name.
   * @param name Registry shortname for the target flake.
   * @return `nullputr` iff no such input exists,
   *         otherwise the input associated with @a name.
   */
  [[nodiscard]] std::shared_ptr<typename FactoryType::input_type>
  get( const std::string & name ) const noexcept
  {
    const auto maybeInput
      = std::find_if( this->inputs.begin(),
                      this->inputs.end(),
                      [&]( const auto & pair ) { return pair.first == name; } );
    if ( maybeInput == this->inputs.end() ) { return nullptr; }
    return maybeInput->second;
  }

  /**
   * @brief Get an input by name, or throw an error if no such input exists.
   * @param name Registry shortname for the target flake.
   * @return The input associated with @a name.
   */
  [[nodiscard]] std::shared_ptr<typename FactoryType::input_type>
  at( const std::string & name ) const
  {
    const std::shared_ptr<typename FactoryType::input_type> maybeInput
      = this->get( name );
    if ( maybeInput == nullptr )
      {
        throw std::out_of_range( "No such input '" + name + "'" );
      }
    return maybeInput;
  }

  /** @brief Get the raw registry read from the user. */
  [[nodiscard]] const RegistryRaw &
  getRaw() const
  {
    return this->registryRaw;
  }

  /* Forwarded container functions. */

  /** @brief Get the number of inputs in the registry. */
  [[nodiscard]] auto
  size() const
  {
    return this->inputs.size();
  }

  /** @brief Whether the registry is empty. */
  [[nodiscard]] auto
  empty() const
  {
    return this->inputs.empty();
  }

  /** @brief Iterate registry members in priority order. */
  [[nodiscard]] auto
  begin()
  {
    return this->inputs.begin();
  }

  /** @brief Get an iterator _sentinel_ used to identify an iterator's end. */
  [[nodiscard]] auto
  end()
  {
    return this->inputs.end();
  }

  /** @brief Iterate read-only registry members in priority order. */
  [[nodiscard]] auto
  begin() const
  {
    return this->inputs.cbegin();
  }

  /**
   * @brief Get an iterator _sentinel_ used to identify a read-only
   *        iterator's end.
   */
  [[nodiscard]] auto
  end() const
  {
    return this->inputs.cend();
  }

  /** @brief Iterate read-only registry members in priority order. */
  [[nodiscard]] auto
  cbegin() const
  {
    return this->inputs.cbegin();
  }

  /**
   * @brief Get an iterator _sentinel_ used to identify a read-only
   *        iterator's end.
   */
  [[nodiscard]] auto
  cend() const
  {
    return this->inputs.cend();
  }


}; /* End class `Registry' */


/* -------------------------------------------------------------------------- */

/**
 * @brief A simple @a flox::RegistryInput that opens a `nix` evaluator for
 *        a flake.
 */
class FloxFlakeInput : public RegistryInput
{

  /* From `RegistryInput':
   *   public:
   *     std::optional<std::vector<Subtree>>     subtrees;
   *     std::shared_ptr<nix::FlakeRef>          from;
   */

private:

  nix::ref<nix::Store>       store; /**< A `nix` store connection. */
  std::shared_ptr<FloxFlake> flake; /**< A flake with an evaluator. */
  /**
   * List of subtrees allowed by preferences, or defaults.
   * This caches the result of `getSubtrees()`.
   */
  std::optional<std::vector<Subtree>> enabledSubtrees;


public:

  /**
   * @brief Construct a @a flox::FloxFlakeInput from a `nix` store connection
   *        and @a flox::RegistryInput.
   */
  FloxFlakeInput( const nix::ref<nix::Store> & store,
                  const RegistryInput &        input )
    : RegistryInput( input ), store( store )
  {}


  /** @brief Get a handle for a flake with a `nix` evaluator. */
  [[nodiscard]] nix::ref<FloxFlake>
  getFlake();


  /**
   * @brief Get a list of enabled subtrees.
   *
   * If the user has explicitly defined a list of subtrees, then simply use
   * that list.
   * If the list is undefined, pick the first of:
   *   1. "package"
   *   2. "legacyPackages"
   */
  [[nodiscard]] const std::vector<Subtree> &
  getSubtrees();


  [[nodiscard]] RegistryInput
  getLockedInput();


}; /* End struct `FloxFlakeInput' */


/* -------------------------------------------------------------------------- */

/** @brief A factory for @a flox::FloxFlakeInput objects. */
class FloxFlakeInputFactory : NixStoreMixin
{

public:

  using input_type = FloxFlakeInput;

  /** @brief Construct a factory using a new `nix` store connection. */
  FloxFlakeInputFactory() = default;

  /** @brief Construct a factory using a `nix` store connection. */
  explicit FloxFlakeInputFactory( const nix::ref<nix::Store> & store )
    : NixStoreMixin( store )
  {}

  /** @brief Construct an input from a @a flox::RegistryInput. */
  [[nodiscard]] std::shared_ptr<FloxFlakeInput>
  mkInput( const std::string & /* unused */, const RegistryInput & input )
  {
    return std::make_shared<FloxFlakeInput>( this->getStore(), input );
  }


}; /* End class `FloxFlakeInputFactory' */


static_assert( registry_input_factory<FloxFlakeInputFactory> );


/* -------------------------------------------------------------------------- */

/**
 * @brief A registry containing `FloxFlakeInput` members.
 *
 * This specialized form of registry carries additional helpers associated with
 * _locking_ registry inputs.
 */
class FlakeRegistry : public Registry<FloxFlakeInputFactory>
{

public:

  FlakeRegistry( RegistryRaw registryRaw, FloxFlakeInputFactory & factory )
    : Registry<FloxFlakeInputFactory>( std::move( registryRaw ), factory )
  {}

  [[nodiscard]] std::map<std::string, RegistryInput>
  getLockedInputs();


}; /* End class `FlakeRegistry' */


/* -------------------------------------------------------------------------- */

/** @brief Lock an unlocked registry. */
RegistryRaw
lockRegistry( const RegistryRaw &          unlocked,
              const nix::ref<nix::Store> & store = NixStoreMixin().getStore() );


/* -------------------------------------------------------------------------- */

/**
 * @brief Get a hard coded registry for use with `flox`'s GA release.
 *
 * This registry contains on `nixpkgs` input, which is set
 * to `github:NixOS/nixpkgs/release-23.11`.
 */
RegistryRaw
getGARegistry();


/* -------------------------------------------------------------------------- */

}  // namespace flox


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
