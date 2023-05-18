{
  # Packages
  # `version` is optional, otherwise the latest is used. Try `flox search`
  # packages.nixpkgs-flox.figlet = {};
  # packages.nixpkgs-flox.bat = { version = "0.22.1"; };

  # Activation Extensions
  # Provides an extensible `<env>/etc/profile` script.
  packages."github:flox/etc-profiles".profile-base = {};
  # Sets common environment variables such as `PKG_CONFIG_PATH' and `MANPATH'.
  packages."github:flox/etc-profiles".profile-common-paths = {};
  # Sets `PYTHONPATH' if `python3' is detected.
  packages."github:flox/etc-profiles".profile-python3 = {};
  # Sets `NODE_PATH' if `node' is detected.
  packages."github:flox/etc-profiles".profile-node = {};


  # Aliases available when environment is active
  # shell.aliases.cat = "bat";

  # Environment variables
  # environmentVariables.LANG = "en_US.UTF-8";

  # Script run upon environment activation
  # Warning: Be careful when using `${}` in shell hook.
  #   Due to conflicts with Nix language you have to escape it with ''
  #   (two single quotes)
  #   Example: ` ''${ENV_VARIABLE} `
  shell.hook = ''
    # Source `<env>/etc/profile` if it exists.
    [ -r "$FLOX_ENV/etc/profile" ] && . "$FLOX_ENV/etc/profile";
  '';
}
