# Checks the CA roots that flox's binaries trust:
#
# - Every package that sets `NIXPKGS_CACERT_BUNDLE_CRT`, and the installers'
#   `pkgsFor.cacert`, use the same bundle, because the CLI compares the bundle
#   it was built with against the one its wrapper sets. Add new packages that
#   set it to `bundles`.
# - The package set's `cacert` is the stock one, so nixpkgs packages that
#   depend on it are still substituted.
# - The bundle doesn't trust the distrusted roots for TLS servers, but does
#   trust a control root, so the check can't pass by matching nothing.
{
  lib,
  runCommand,
  openssl,
  cacert,
  stockCacert,
  flox,
  flox-cli,
  flox-activations,
  nef-lock-catalog,
  rust-internal-deps,
  crate2nix-builds,
}:
let
  # nixpkgs' cacert already doesn't trust Comodo AAA Services root for TLS
  # servers, so its entry guards against nixpkgs trusting it again rather than
  # against it being dropped from the blacklist.
  distrusted = [
    "Comodo AAA Services root"
    "emSign ECC Root CA - G3"
    "emSign Root CA - C1"
    "emSign ECC Root CA - C3"
  ];
  control = "ISRG Root X1";

  bundles = {
    flox-cli = flox-cli.envs.NIXPKGS_CACERT_BUNDLE_CRT;
    flox-activations = flox-activations.devEnvs.NIXPKGS_CACERT_BUNDLE_CRT;
    nef-lock-catalog = nef-lock-catalog.devEnvs.NIXPKGS_CACERT_BUNDLE_CRT;
    rust-internal-deps = rust-internal-deps.envs.NIXPKGS_CACERT_BUNDLE_CRT;
    crate2nix-flox-core = crate2nix-builds.crateEnvs.flox-core.NIXPKGS_CACERT_BUNDLE_CRT;
    installers = flox.passthru.pkgsFor.cacert.outPath + "/etc/ssl/certs/ca-bundle.crt";
  };
in
assert lib.assertMsg (lib.length (lib.unique (lib.attrValues bundles)) == 1) ''
  flox's packages compile in different CA bundles:
  ${lib.generators.toPretty { } bundles}'';
assert lib.assertMsg (cacert.outPath == stockCacert.outPath) ''
  The package set's cacert is not the stock nixpkgs one, so every package that depends on it is rebuilt:
    package set: ${cacert.outPath}
    nixpkgs:     ${stockCacert.outPath}'';
runCommand "ca-roots"
  {
    nativeBuildInputs = [ openssl ];
    bundle = bundles.flox-cli;
    stock = stockCacert.unbundled;
  }
  ''
    # List the fingerprints of the certificates that the bundle trusts for TLS
    # servers: plain certificates, and certificates with trust settings that
    # list TLS server authentication.
    mkdir certs
    awk '/^-----BEGIN/ { f = "certs/" ++n ".pem" } f { print > f }' "$bundle"
    for cert in certs/*.pem; do
      text=$(openssl x509 -in "$cert" -noout -text)
      if grep -q 'BEGIN CERTIFICATE' "$cert" || grep -A1 '^Trusted Uses:' <<<"$text" | grep -q 'TLS Web Server'; then
        openssl x509 -in "$cert" -noout -fingerprint -sha256
      fi
    done > trusted

    # trusts <label>: whether the bundle trusts the stock certificate with this
    # label. The stock unbundled output names each certificate after its label.
    trusts() {
      local files=("$stock/etc/ssl/certs/''${1// /_}:"*.crt)
      if [[ ''${#files[@]} -ne 1 || ! -f ''${files[0]} ]]; then
        echo "nixpkgs' cacert has no single certificate labeled '$1'." >&2
        echo "If it no longer ships it, remove it from the blacklist in flake.nix and from this check." >&2
        exit 1
      fi
      grep -qxF "$(openssl x509 -in "''${files[0]}" -noout -fingerprint -sha256)" trusted
    }

    status=0
    for label in ${lib.escapeShellArgs distrusted}; do
      if trusts "$label"; then
        echo "The bundle trusts '$label', but it should be distrusted." >&2
        status=1
      fi
    done
    if ! trusts ${lib.escapeShellArg control}; then
      echo "The bundle doesn't trust the control root '${control}'." >&2
      status=1
    fi
    [[ $status -eq 0 ]]

    touch "$out"
  ''
