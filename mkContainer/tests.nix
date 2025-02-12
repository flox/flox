# Note that all tests should start with "test", otherwise runTests will not pick them up.
{
  lib,
  internals,
}:
let
  inherit (builtins) match tryEval;
in
lib.debug.runTests {
  testIsNixStoreUserGroupOwned = {
    expr = (null == (match internals.isNixStoreUserOwnedRegex "foo:1234"));
    expected = true;
  };
  testIsNixStoreUserOwned = {
    expr = (null == (match internals.isNixStoreUserOwnedRegex "foo"));
    expected = true;
  };
  testIsNixStoreRootOwned = {
    expr = (null == (match internals.isNixStoreUserOwnedRegex "root:root"));
    expected = false;
  };
  testIsNixStore0Owned = {
    expr = (null == (match internals.isNixStoreUserOwnedRegex "0:0"));
    expected = false;
  };
  testUserConfigMatchUnameGid = {
    expr = (match internals.unameGnameRegex "foo:1234");
    expected = [
      "foo:"
      "foo"
      null
      "1234"
      null
      "1234"
    ];
  };
  testUserConfigMatchUidGid = {
    expr = (match internals.unameGnameRegex "1234:1234");
    expected = [
      "1234:"
      null
      "1234"
      "1234"
      null
      "1234"
    ];
  };
  testUserConfigMatchUname = {
    expr = (match internals.unameGnameRegex "foo");
    expected = [
      "foo"
      "foo"
      null
      null
      null
      null
    ];
  };
  testUserConfigMatchInvalidUname = {
    expr = (match internals.unameGnameRegex "-foo");
    expected = null;
  };
  testmkUnameGnameUidGidWithUnameGid = {
    expr = (internals.mkUnameGnameUidGid "foo:1234");
    expected = {
      uname = "foo";
      gname = "flox";
      uid = 10000;
      gid = 1234;
    };
  };
  testmkUnameGnameUidGidWithUnameGname = {
    expr = (internals.mkUnameGnameUidGid "foo:bar");
    expected = {
      uname = "foo";
      gname = "bar";
      uid = 10000;
      gid = 10000;
    };
  };
  testmkUnameGnameUidGidWithUname = {
    expr = (internals.mkUnameGnameUidGid "foo");
    expected = {
      uname = "foo";
      gname = "flox";
      uid = 10000;
      gid = 10000;
    };
  };
  testmkUnameGnameUidGidWithInvalidUname = {
    expr = (tryEval (internals.mkUnameGnameUidGid "-foo")).success;
    expected = false;
  };
}
