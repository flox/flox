{
  self,
  lib,
  rustPlatform,
  hostPlatform,
  # you can add imports here
  openssl,
  pkg-config,
  libiconv,
  darwin,
}:
rustPlatform.buildRustPackage rec {
  pname = "my-package";
  version = "0.0.0";
  src = self; # + "/src";

  cargoLock = {
    lockFile = self + "/Cargo.lock";
    # The hash of each dependency that uses a git source must be specified.
    # The hash can be found by setting it to lib.fakeSha256
    # as shown below and running flox build.
    # The build will fail but output the expected sha, which can then be added
    # here
    outputHashes = {
      #   "dependency-0.0.0" = lib.fakeSha256;
    };
  };



  # Non-Rust runtime dependencies (most likely libraries) of your project can 
  # be added in buildInputs.
  # Make sure to import any additional dependencies above
  buildInputs =
    [
      openssl.dev
    ]
    # Platform specific dependencies can be added as well
    # For MacOS
    ++ lib.optional hostPlatform.isDarwin [
      # If you're getting linker errors about missing libraries, you can add
      # them here
      libiconv
      # If you're getting linker errors about missing frameworks, you can add
      # apple frameworks here
      darwin.apple_sdk.frameworks.Security
    ]
    # and Linux
    ++ lib.optional hostPlatform.isLinux [ ]
    ;


  # Add runtime dependencies required by packages that depend on this package
  # to propagatedBuildInputs.
  propagatedBuildInputs = [];

  # Add buildtime dependencies (not required at runtime) to nativeBuildInputs.
  nativeBuildInputs = [
    pkg-config # for openssl
  ];


}
