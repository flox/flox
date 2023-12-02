/* ========================================================================== *
 *
 * This file provides two approaches which allow us to unit test non-public
 * class methods.
 *
 * We prefer Approach 1 because it does not pollute public headers, but
 * Approach 2 is also provided for reference.
 *
 *
 * -------------------------------------------------------------------------- */

#include <cstddef>
#include <iostream>


/* -------------------------------------------------------------------------- */

/* Approach 1: Using `protected' instead of `private' methods.
 * This requires that you modify the visibility of methods that you want
 * to test. */

class RealClass0 {
protected:
  int times2( int x ) { return 2 * x; }
};  /* End class `RealClass' */


/* In practice this would be defined with your tests. */
class TestClass0 : public RealClass0 {
public:
  using RealClass0::times2;
};  /* End class `TestClass' */


/* -------------------------------------------------------------------------- */

/* Approach 2: Using `friend' classes.
 * This does not require changes to visiblity, but does require a forward
 * declaration of `TestClass1' in your public headers. */

class TestClass1;

class RealClass1 {
private:
    int times2( int x ) { return 2 * x; }

  friend class TestClass1;
};  /* End class `RealClass' */


/* In practice this would be defined with your tests. */
class TestClass1 : RealClass1 {
  public:
    using RealClass1::times2;
};  /* End class `TestClass' */


/* -------------------------------------------------------------------------- */

int
main()
{
  TestClass0 tc0;
  std::cout << tc0.times2( 2 ) << std::endl;

  TestClass1 tc1;
  std::cout << tc1.times2( 2 ) << std::endl;

  return EXIT_SUCCESS;
}


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
