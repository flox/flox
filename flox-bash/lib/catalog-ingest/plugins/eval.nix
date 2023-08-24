# capacitor provided arguments, local to the defining flake
{
  lib,
  self,
  ...
}:
# attributes for user level configuration when adding to `config.plugins`
{
  build ? {},
  attributePath ? ["analysis"],
  target,
}:
# context of the flake the plugin is used in
{...}: let
  analysis = self.lib.analysis {inherit target;};
  projects = lib.mapAttrsToList (_: child: child.__reflect.context.closures "packages") target.__reflect.config.projects or {};
  own = target.__reflect.context.closures "packages";
in
  lib.setAttrByPath
  attributePath
  (analysis.analysisGen build (lib.flatten (projects ++ [own])))
