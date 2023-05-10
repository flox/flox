{
  lib,
  nixpkgs,
  self,
}: {
  context,
  namespace,
  modules,
  system,
}:
(lib.evalModules {
  modules =
    [
      {
        _module.args = {
          inherit context namespace system;
        };
      }
      (self + "/modules")
    ]
    ++ modules;
})
.config
.toplevel
