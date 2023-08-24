# Nix packages rely on "system" files that are found in different
# locations on different operating systems and distros, and many of
# these packages employ environment variables for overriding the
# default locations for such files.
#
# This file provides a place for defining global environment variables
# and borrows liberally from the set of default environment variables
# set by NixOS, the principal proving ground for Nixpkgs itself.
export SSL_CERT_FILE="${SSL_CERT_FILE:-@cacert@/etc/ssl/certs/ca-bundle.crt}"
export NIX_SSL_CERT_FILE="${NIX_SSL_CERT_FILE:-$SSL_CERT_FILE}"
