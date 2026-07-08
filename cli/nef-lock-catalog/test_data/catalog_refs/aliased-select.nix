{ catalogs }:
let
  org = catalogs.myorg;
  toolkit = org.toolkit;
in
toolkit.readVersion
