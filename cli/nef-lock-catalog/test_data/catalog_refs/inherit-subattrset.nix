# Pattern: `inherit (catalogs.myorg) toolkit;` — inheriting the toolkit
# attrset itself from the top level of a catalog, then calling a method on it.
# The inherit yields the exact ref `catalogs.myorg.toolkit`; the use site adds
# the deeper `catalogs.myorg.toolkit.buildGoModule`, which the server
# canonicalizes to the same package.
{
  catalogs,
  lib,
  pandoc,
}:

let
  src = ../../..;
  inherit (catalogs.myorg) toolkit;

in
toolkit.buildGoModule {
  pname = "queue-daemon";
  inherit src;
  version = lib.fileContents "${src}/VERSION";

  subPackages = [ "cmd/queued" ];

  nativeBuildInputs = [ pandoc ];

  postInstall = ''
    mkdir -p $out/share/man/man1
    cat cmd/queued/README.md | pandoc -s -f markdown -w man \
      | gzip -c9 > $out/share/man/man1/queued.1.gz
  '';

  doCheck = false;

  passthru.src = src;

  meta = {
    description = "Async task queue daemon";
  };
}
