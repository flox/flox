{
  # Packages
  # "version" is optional, otherwise the latest is used. Try `flox search`
  # packages.nixpkgs-flox.figlet = {};
  # packages.nixpkgs-flox.bat = { version = "0.22.1"; };

  # Aliases available when environment is active
  # shell.aliases.cat = "bat";

  # Script run upon environment activation
  # Warning: Be careful when using `${}` in shell hook.
  #          Due to conflicts with Nix language you have to escape it with '' (two single quotes)
  #          Example: ` ''${ENV_VARIABLE} `
  # shell.hook = ''
  #   echo Flox Environment | figlet
  # '';

  # Environment variables
  # environmentVariables.LANG = "en_US.UTF-8";
}
