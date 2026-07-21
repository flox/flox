# Helper receiving the catalog namespace under the name `cats`.
{ cats }:
{
  result = cats.myorg.toolkit.readVersion;
}
