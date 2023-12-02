/* ========================================================================== *
 *
 * @file docs/visit.cc
 *
 * @brief An example of using `std::visit( overloaded { ... }, value ) )`
 *        with `std::variant` to implement a type-safe visitor
 *        ( `switch` statement ).
 *
 *
 * -------------------------------------------------------------------------- */

#include <cstdlib>
#include <iostream>
#include <variant>
#include "flox/core/util.hh"


/* -------------------------------------------------------------------------- */

/** @brief An empty class. */
class Empty {};

/** @brief Another empty class. */
class Empty2 {};


/* -------------------------------------------------------------------------- */

int
main()
{
  /* Good */
  auto doVisit = []( std::variant<int, Empty> value )
  {
    std::visit( overloaded {
        []( int x ) { std::cout << "Integer: " << x << std::endl; },
        []( Empty & ) { std::cout << "Empty" << std::endl; }
    }, value );
  };

  doVisit( 1 );        // => `Integer: 1'
  doVisit( 2 );        // => `Integer: 2'
  doVisit( Empty() );  // => `Empty'


  /* Bad: Gets angry because `Empty2' isn't visited.
   * error: no type named ‘type’ in
   *   ‘struct std::__invoke_result<overloaded<main()::<
   *      lambda(std::variant<int, Empty, Empty2>)
   *    >::<lambda(int)>,
   *    main()::<lambda(std::variant<
   *      int, Empty, Empty2>)
   *    >::<lambda(Empty&)> >, Empty2&>’
   */
  #if 0
  auto evilVisit = []( std::variant<int, Empty, Empty2> value )
  {
    std::visit( overloaded {
        []( int x ) { std::cout << "Integer: " << x << std::endl; },
        []( Empty & ) { std::cout << "Empty" << std::endl; }
    }, value );
  };

  evilVisit( 1 );
  #endif

  return EXIT_SUCCESS;
}


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
