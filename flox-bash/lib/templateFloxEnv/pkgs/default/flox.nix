{
  # Packages
  # "version" is optional, otherwise the latest is used. Try `flox search`
  # packages.nixpkgs-flox.figlet = {};
  # packages.nixpkgs-flox.bat = { version = "0.22.1"; };

  # Activation Extensions
  # Provides an extensible `<env>/etc/profile` script.
  # see 'man flox-activate' for more information on Language Packs.
  # packages.flox.etc-profiles = {
  #   # All language packs are installed by default, but you can also
  #   # select individual packs by uncommenting the line below.
  #   # Invoke `flox search -l -c flox etc-profiles` to see
  #   # a list of all supported language pack outputs.
  #   # Please note that all/most language packs depend on including
  #   # the "base" and "common_paths" output.
  #   meta.outputsToInstall = [ "base" "common_paths" "python3" ];
  # };

  # Aliases available when environment is active
  # shell.aliases.cat = "bat";

  # Script run upon environment activation
  # Warning: Be careful when using `${}` in shell hook.
  #          Due to conflicts with Nix language you have to
  #          first escape it with '' (two single quotes).
  #          Example: ` ''${ENV_VARIABLE} `
  shell.hook = ''
    # Source `<env>/etc/profile` if it exists.
    [ -r "$FLOX_ENV/etc/profile" ] && . "$FLOX_ENV/etc/profile";
    # echo Flox Environment | figlet
  '';

  # Environment variables
  # environmentVariables.LANG = "en_US.UTF-8";
}
