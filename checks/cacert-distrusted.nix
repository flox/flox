# Verifies the CA bundle flox ships: that `cacert-distrusted` rejects the four
# compromised roots for TLS, and that every flox package that bakes in a CA
# bundle bakes in that one rather than the nixpkgs `cacert`.
{ pkgs }:
let
  expectedBundle = "${pkgs.cacert-distrusted}/etc/ssl/certs/ca-bundle.crt";

  # `NIXPKGS_CACERT_BUNDLE_CRT` as each package's build sees it.
  bakedBundles = {
    flox-cli = pkgs.flox-cli.drvAttrs.NIXPKGS_CACERT_BUNDLE_CRT;
    flox-activations = pkgs.flox-activations.drvAttrs.NIXPKGS_CACERT_BUNDLE_CRT;
    nef-lock-catalog = pkgs.nef-lock-catalog.drvAttrs.NIXPKGS_CACERT_BUNDLE_CRT;
    rust-internal-deps = pkgs.rust-internal-deps.drvAttrs.NIXPKGS_CACERT_BUNDLE_CRT;
    crate2nix-builds = pkgs.crate2nix-builds.crateEnvs.flox-core.NIXPKGS_CACERT_BUNDLE_CRT;
  };

  wrongBundles = pkgs.lib.filterAttrs (_: bundle: bundle != expectedBundle) bakedBundles;
in
assert pkgs.lib.assertMsg (wrongBundles == { }) ''
  These packages bake in a CA bundle other than cacert-distrusted:
  ${builtins.concatStringsSep "\n" (
    pkgs.lib.mapAttrsToList (name: bundle: "  ${name}: ${bundle}") wrongBundles
  )}
'';
pkgs.runCommand "check-cacert-distrusted"
  {
    cacertUnbundled = pkgs.cacert-distrusted.unbundled;
    nativeBuildInputs = [ pkgs.openssl ];
  }
  ''
    # buildcatrust keeps blacklisted certs in the bundle and marks them
    # "Rejected Uses: TLS Web Server Authentication", so check for that
    # marking rather than for absence. Mirrors the nixpkgs cacert
    # passthru.tests.blacklist-utf8 pattern.
    #
    # isTrusted <unbundled-dir> <sha256-fingerprint>: 0 if trusted for TLS,
    # 1 if rejected or absent.
    isTrusted() {
      for f in $1/etc/ssl/certs/*.crt; do
        if ! [[ -s "$f" ]]; then continue; fi
        fingerprint="$(openssl x509 -in "$f" -noout -fingerprint -sha256 | cut -f2 -d=)"
        if [[ "x$fingerprint" == "x$2" ]]; then
          if openssl x509 -in "$f" -noout -text | grep -A1 '^Rejected Uses:' | grep -q 'TLS Web Server'; then
            return 1
          fi
          return 0
        fi
      done
      return 1
    }

    # Fingerprints verified against nss-cacert-3.121-unbundled.
    declare -A distrusted=(
      ["Comodo AAA Services root"]="D7:A7:A0:FB:5D:7E:27:31:D7:71:E9:48:4E:BC:DE:F7:1D:5F:0C:3E:0A:29:48:78:2B:C8:3E:E0:EA:69:9E:F4"
      ["emSign ECC Root CA - G3"]="86:A1:EC:BA:08:9C:4A:8D:3B:BE:27:34:C6:12:BA:34:1D:81:3E:04:3C:F9:E8:A8:62:CD:5C:57:A3:6B:BE:6B"
      ["emSign Root CA - C1"]="12:56:09:AA:30:1D:A0:A2:49:B9:7A:82:39:CB:6A:34:21:6F:44:DC:AC:9F:39:54:B1:42:92:F2:E8:C8:60:8F"
      ["emSign ECC Root CA - C3"]="BC:4D:80:9B:15:18:9D:78:DB:3E:1D:8C:F4:F9:72:6A:79:5D:A1:64:3C:A5:F1:35:8E:1D:DB:0E:DC:0D:7E:B3"
    )

    for name in "''${!distrusted[@]}"; do
      if isTrusted "$cacertUnbundled" "''${distrusted[$name]}"; then
        echo "FAIL: $name is still trusted for TLS" >&2
        exit 1
      fi
      echo "OK: $name is rejected for TLS"
    done

    # Mozilla did not distrust emSign Root CA - G1; it must stay trusted.
    if ! isTrusted "$cacertUnbundled" "40:F6:AF:03:46:A9:9A:A1:CD:1D:55:5A:4E:9C:CE:62:C7:F9:63:46:03:EE:40:66:15:83:3D:C8:C8:D0:03:67"; then
      echo "FAIL: emSign Root CA - G1 is not trusted" >&2
      exit 1
    fi
    echo "OK: emSign Root CA - G1 is trusted"

    touch "$out"
  ''
