# ============================================================================ #
#
# A cross-platform environment manager with sharing as a service.
#
# ---------------------------------------------------------------------------- #
{
  description = "flox - Harness the power of Nix";

  # cache.flox.dev advertises no priority and would otherwise rank above
  # cache.nixos.org (priority 40) for every path both caches carry.
  # priority=50 restricts it to the paths only it serves: flox closures.
  nixConfig.extra-substituters = [ "https://cache.flox.dev?priority=50" ];
  nixConfig.extra-trusted-public-keys = [
    "flox-cache-public-1:7F4OyH7ZCnFhcze3fJdfyXYLQw/aV7GEed86nQ7IsOs="
  ];

  # Roll forward monthly as **our** stable branch advances. Note that we also
  # build against the staging branch in CI to detect regressions before they
  # reach stable.
  inputs.nixpkgs.url = "github:flox/nixpkgs/stable";

  inputs.pre-commit-hooks.url = "github:cachix/pre-commit-hooks.nix";
  inputs.pre-commit-hooks.inputs.nixpkgs.follows = "nixpkgs";

  inputs.crane.url = "github:ipetkov/crane";

  inputs.fenix.url = "github:nix-community/fenix";
  inputs.fenix.inputs.nixpkgs.follows = "nixpkgs";

  # -------------------------------------------------------------------------- #

  outputs =
    inputs:
    let
      # ------------------------------------------------------------------------ #
      nixpkgs.legacyPackages = {
        inherit (inputs.nixpkgs.legacyPackages)
          x86_64-linux
          x86_64-darwin
          aarch64-linux
          aarch64-darwin
          ;
      };
      nixpkgs.lib = inputs.nixpkgs.lib;
    in
    rec {
      # Overlays
      # --------
      overlays.deps = nixpkgs.lib.composeManyExtensions [
        (final: prev: {
          # Add IWYU pragmas to `nlohmann_json'
          # ( _include what you use_ extensions to headers for static analysis )
          nlohmann_json = final.callPackage ./pkgs/nlohmann_json { inherit (prev) nlohmann_json; };

          # Uncomment to compile Nix with debug symbols on Linux
          # nix = final.enableDebugging (final.callPackage ./pkgs/nix {});
          nix = final.callPackage ./pkgs/nix { };

          cpp-semver = final.callPackage ./pkgs/cpp-semver { };

          process-compose = final.callPackage ./pkgs/process-compose { inherit (prev) process-compose; };

          # Distrust four CA roots whose private keys were compromised.
          #
          # The three emSign entries are a backport of the distrust that
          # upstream nixpkgs applied natively in NSS 3.131.1 (nixpkgs
          # #576390, then #576347). flox pins NSS 3.121, so we carry them
          # here. On a future nixpkgs bump to ≥3.131.1 they become
          # harmless no-ops (NSS already rejects them in certdata).
          #
          # "Comodo AAA Services root" is a separate compromise; NSS never
          # distrusted it, so this entry must remain regardless of the NSS
          # version we pin.
          #
          # buildcatrust keeps the certs in the bundle and marks them
          # "Rejected Uses: TLS Web Server Authentication" — grep-for-absence
          # is the wrong check; use `openssl x509 -text | grep Rejected`.
          #
          # macOS is unaffected (Keychain); this override matters on Linux
          # where reqwest → native-tls → OpenSSL, Nix subprocess downloads,
          # and NIX_SSL_CERT_FILE all read NIXPKGS_CACERT_BUNDLE_CRT.
          #
          # Deliberately not an override of `cacert` itself: `cacert` is in the
          # closure of a large part of nixpkgs (python3Packages.certifi, and
          # through sphinx also ghc), so a global override forces source
          # rebuilds of pandoc, shellcheck, nixfmt and hundreds of other
          # derivations. Only the flox packages that bake the bundle take this
          # attribute (see overlays.flox).
          flox-cacert = prev.cacert.override {
            blacklist = [
              "Comodo AAA Services root"
            ]
            # NSS 3.131.1 removes these three roots from certdata, and
            # buildcatrust fails the build on a blocklist label it cannot
            # find, so they must drop out together with the nixpkgs bump.
            ++ final.lib.optionals (final.lib.versionOlder prev.cacert.version "3.131.1") [
              "emSign ECC Root CA - G3"
              "emSign Root CA - C1"
              "emSign ECC Root CA - C3"
              # emSign Root CA - G1 is NOT distrusted (Mozilla did not
              # revoke it); omitting it is intentional.
            ];
          };

        })
        inputs.fenix.overlays.default
      ];

      # Packages defined in this repository.
      overlays.flox =
        final: prev:
        let
          callPackage = final.lib.callPackageWith (
            final
            // {
              inherit inputs; # passing in inputs... beware
              inherit (inputs) self;
              pkgsFor = final;
            }
          );
          nixpkgsInputLockedURL =
            nixpkgsInput: "github:flox/nixpkgs/${nixpkgsInput.rev}?narHash=${nixpkgsInput.narHash}";
        in
        {
          # Generates a `.git/hooks/pre-commit' script.
          pre-commit-check = callPackage ./pkgs/pre-commit-check { inherit (inputs) pre-commit-hooks; };

          GENERATED_DATA = ./test_data/generated;
          UNIT_TEST_GENERATED = ./test_data/unit_test_generated;
          MANUALLY_GENERATED = ./test_data/manually_generated;

          # We depend on several nightly features of rustfmt,
          # so pick the current nightly version.
          # We're using `default.withComponents`
          # which _should_ only pull the nightly rustfmt component.
          # Alternatively, we could use nixpkgs.rustfmt,
          # and rebuild with a (stable) fenix toolchain and `asNightly = true`,
          # which would avoid the need to pull another channel altogether.
          rustfmt = final.fenix.default.withComponents [ "rustfmt" ];
          rust-toolchain = final.fenix.stable;

          rust-external-deps = callPackage ./pkgs/rust-external-deps { };
          rust-internal-deps = callPackage ./pkgs/rust-internal-deps {
            inherit nixpkgsInputLockedURL;
            cacert = final.flox-cacert;
          };

          # Same binaries built from the generated Cargo.nix.
          #
          # `flox-activations` is this package's own build, passed back
          # through the overlay's fixed point. Only the `flox` crate reads the
          # resulting path, and building flox-activations does not, so this
          # does not need a second instantiation of the crate graph.
          crate2nix-builds = callPackage ./pkgs/crate2nix {
            inherit nixpkgsInputLockedURL;
            inherit (final.crate2nix-builds) flox-activations;
            cacert = final.flox-cacert;
          };

          # (Linux-only) LD_AUDIT library for using dynamic libraries in Flox envs.
          ld-floxlib = callPackage ./pkgs/ld-floxlib { };
          flox-src = callPackage ./pkgs/flox-src { };
          flox-interpreter = callPackage ./pkgs/flox-interpreter { };
          flox-package-builder = callPackage ./pkgs/flox-package-builder { };

          # Package Database Utilities: scrape, search, and resolve.
          flox-nix-plugins = callPackage ./pkgs/flox-nix-plugins { };
          flox-buildenv = callPackage ./pkgs/flox-buildenv { };
          flox-activations = callPackage ./pkgs/flox-activations { cacert = final.flox-cacert; };
          nef-lock-catalog = callPackage ./pkgs/nef-lock-catalog { cacert = final.flox-cacert; };
          flox-cli = callPackage ./pkgs/flox-cli { cacert = final.flox-cacert; };

          flox-manpages = callPackage ./pkgs/flox-manpages { }; # Flox Command Line Interface Manpages
          flox = callPackage ./pkgs/flox { cacert = final.flox-cacert; }; # Flox Command Line Interface ( production build ).

          # Wrapper scripts for running test suites.
          flox-cli-tests = callPackage ./pkgs/flox-cli-tests { };
        };

      overlays.development = final: prev: {
        floxDevelopmentPackages =
          let
            # Create a flox-activations package that just copies the Cargo built
            # development binary into $out/libexec/flox-activations
            floxActivationsBin = "${builtins.path { path = builtins.getEnv "FLOX_ACTIVATIONS_BIN"; }}";
            cargoBuiltFloxActivations =
              prev.runCommandNoCC "flox-activations"
                {
                  name = "flox-activations";
                  # Ensure OpenSSL is in the closure for the cargo-built binary
                  # which links against it via sentry -> native-tls
                  propagatedBuildInputs = prev.lib.optionals prev.stdenv.isLinux [ prev.openssl.out ];
                }
                ''
                  mkdir -p $out/libexec
                  cp ${floxActivationsBin} $out/libexec/flox-activations
                '';
          in
          prev.lib.makeScope prev.newScope (self: {
            rust-internal-deps = prev.rust-internal-deps.override {
              flox-buildenv = null;
              flox-package-builder = null;
              flox-nix-plugins = null;
              flox-mk-container = null;
              flox-interpreter = null;
            };

            flox-cli = prev.flox-cli.override {
              flox-interpreter = null;
              rust-internal-deps = self.rust-internal-deps;
            };
            crate2nix-builds = prev.crate2nix-builds.override {
              flox-activations = null;
              flox-buildenv = null;
              flox-package-builder = null;
              flox-nix-plugins = null;
              flox-mk-container = null;
              flox-interpreter = null;
            };
            flox-activations = prev.flox-activations.override { };
            flox-interpreter = prev.flox-interpreter.override {
              flox-activations = cargoBuiltFloxActivations;
            };
            flox-package-builder = prev.flox-package-builder.override { };
            flox-buildenv = prev.flox-buildenv.override {
              flox-interpreter = null;
              flox-activations = cargoBuiltFloxActivations;
            };
            checksFor = checks.${prev.system};

            flox-cli-tests = prev.flox-cli-tests.override {
              PROJECT_TESTS_DIR = "/cli/tests";
              localDev = true;
            };
          });
      };
      # Composes dependency overlays and the overlay defined here.
      overlays.default = nixpkgs.lib.composeManyExtensions [
        overlays.deps
        overlays.flox
        overlays.development
      ];

      # ------------------------------------------------------------------------ #

      # Apply overlays to the `nixpkgs` _base_ set.
      # This is exposed as an output later; but we don't use the name
      # `legacyPackages' to avoid checking the full closure with
      # `nix flake check' and `nix search'.
      pkgsContext = builtins.mapAttrs (system: pkgs: pkgs.extend overlays.default) nixpkgs.legacyPackages;

      # ------------------------------------------------------------------------ #

      checks = builtins.mapAttrs (system: pkgs: {
        inherit (pkgs) pre-commit-check;
        nixos-module = import ./modules/nixos/check.nix {
          inherit pkgs nixpkgs;
          module = import ./modules/nixos pkgsContext;
        };

        # Regression test: verify that the four distrusted CA roots are
        # rejected for TLS Web Server Authentication in the overridden
        # bundle, and that emSign Root CA - G1 remains trusted.
        #
        # buildcatrust marks blocklisted CAs with "Rejected Uses:" in the
        # p11-kit file that OpenSSL reads; `isTrusted` below mirrors the
        # nixpkgs cacert passthru.tests.blacklist-utf8 pattern exactly.
        #
        # Fingerprints verified against nss-cacert-3.121-unbundled.
        cacert-emudhra-distrust =
          pkgs.runCommand "verify-cacert-emudhra-distrust"
            {
              cacertUnbundled = pkgs.flox-cacert.unbundled;
              nativeBuildInputs = [ pkgs.openssl ];
            }
            ''
              isTrusted() {
                # isTrusted <unbundled-dir> <sha256-fingerprint>
                # Returns 0 (trusted) or 1 (rejected/absent for TLS).
                for f in $1/etc/ssl/certs/*.crt; do
                  if ! [[ -s "$f" ]]; then continue; fi
                  fingerprint="$(openssl x509 -in "$f" -noout -fingerprint -sha256 | cut -f2 -d=)"
                  if [[ "x$fingerprint" == "x$2" ]]; then
                    if openssl x509 -in "$f" -noout -text | grep -q '^Rejected Uses:'; then
                      if openssl x509 -in "$f" -noout -text | grep -A1 '^Rejected Uses:' | grep -q 'TLS Web Server'; then
                        return 1
                      fi
                    fi
                    return 0
                  fi
                done
                return 1
              }

              # These roots must be rejected for TLS. The emSign entries are
              # only present while the pinned NSS still ships them (see the
              # version gate on the override).
              declare -A distrusted=(
                ["Comodo AAA Services root"]="D7:A7:A0:FB:5D:7E:27:31:D7:71:E9:48:4E:BC:DE:F7:1D:5F:0C:3E:0A:29:48:78:2B:C8:3E:E0:EA:69:9E:F4"
                ${pkgs.lib.optionalString (pkgs.lib.versionOlder pkgs.cacert.version "3.131.1") ''
                  ["emSign ECC Root CA - G3"]="86:A1:EC:BA:08:9C:4A:8D:3B:BE:27:34:C6:12:BA:34:1D:81:3E:04:3C:F9:E8:A8:62:CD:5C:57:A3:6B:BE:6B"
                  ["emSign Root CA - C1"]="12:56:09:AA:30:1D:A0:A2:49:B9:7A:82:39:CB:6A:34:21:6F:44:DC:AC:9F:39:54:B1:42:92:F2:E8:C8:60:8F"
                  ["emSign ECC Root CA - C3"]="BC:4D:80:9B:15:18:9D:78:DB:3E:1D:8C:F4:F9:72:6A:79:5D:A1:64:3C:A5:F1:35:8E:1D:DB:0E:DC:0D:7E:B3"
                ''}
              )

              for name in "''${!distrusted[@]}"; do
                fp="''${distrusted[$name]}"
                if isTrusted "$cacertUnbundled" "$fp"; then
                  echo "FAIL: $name ($fp) is still trusted for TLS — distrust override did not apply" >&2
                  exit 1
                else
                  echo "OK: $name is rejected for TLS"
                fi
              done

              # emSign Root CA - G1 was NOT distrusted by Mozilla and must
              # remain trusted.
              G1_FP="40:F6:AF:03:46:A9:9A:A1:CD:1D:55:5A:4E:9C:CE:62:C7:F9:63:46:03:EE:40:66:15:83:3D:C8:C8:D0:03:67"
              if isTrusted "$cacertUnbundled" "$G1_FP"; then
                echo "OK: emSign Root CA - G1 remains trusted"
              else
                echo "FAIL: emSign Root CA - G1 was incorrectly distrusted" >&2
                exit 1
              fi

              touch "$out"
            '';

      }) pkgsContext;

      # ------------------------------------------------------------------------ #

      packages = builtins.mapAttrs (system: pkgs: {
        inherit (pkgs)
          flox-interpreter
          flox-nix-plugins
          flox-buildenv
          flox-package-builder
          flox-activations
          nef-lock-catalog
          flox-cli
          flox-cli-tests
          flox-manpages
          flox
          ld-floxlib
          pre-commit-check
          rust-external-deps
          rust-internal-deps
          floxDevelopmentPackages
          nix
          ;

        crate2nix-flox = pkgs.crate2nix-builds.flox;
        crate2nix-flox-activations = pkgs.crate2nix-builds.flox-activations;
        crate2nix-nef-lock-catalog = pkgs.crate2nix-builds.nef-lock-catalog;

        default = pkgs.flox;
      }) pkgsContext;

      # ------------------------------------------------------------------------ #

      devShells = builtins.mapAttrs (system: pkgsBase: {
        default = pkgsBase.floxDevelopmentPackages.callPackage ./shells/default { };
        crate2nix = pkgsBase.floxDevelopmentPackages.callPackage ./shells/crate2nix { };
      }) pkgsContext;

      # ------------------------------------------------------------------------ #

      # NixOS/Darwin/HomeManager module
      nixosModules.flox = import ./modules/nixos pkgsContext;
      darwinModules.flox = import ./modules/darwin.nix pkgsContext;
      homeModules.flox = import ./modules/home.nix pkgsContext;

      # ------------------------------------------------------------------------ #
    };

  # -------------------------------------------------------------------------- #
}
