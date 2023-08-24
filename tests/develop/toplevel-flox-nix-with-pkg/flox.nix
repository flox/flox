{
  packages.nixpkgs-flox.curl = {};
  environmentVariables = {
    floxEnvActivated = "true";
  };
  shell = {
    hook = ''
      echo "activating floxEnv"
    '';
  };
}
