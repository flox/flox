# The single source of truth for the activation-context attrset a container
# guest reads at startup. Kept nixpkgs-free (builtins only) so it can be
# evaluated in ~0.1s without a `getFlake` of nixpkgs.
#
# `mkContainer.nix` imports this with the nixpkgs-derived `bashPath` resolved
# from `containerPkgs.bash`, so a full bake and a store-volume refresh emit a
# byte-identical context. See the store-volume refresh design for why the
# no-drift guarantee is load-bearing.
#
# For field definitions, see `ActivateCtx` in `flox-core`.
{
  # The absolute path to the baked bash, e.g. "${containerPkgs.bash}/bin/bash".
  # This is the only nixpkgs-derived value; resolving it is the expensive step
  # the refresh caches so the context can be rebuilt without nixpkgs.
  bashPath,
  # The environment closure store path (as a plain string).
  environmentOutPath,
  # The activation interpreter path baked into the context.
  interpreterPath,
  # The activation mode ("dev" or "run").
  activationMode,
  # The container name, used for the prompt and env descriptions.
  containerName,
}:
{
  mode = "${activationMode}";
  shell = {
    bash = "${bashPath}";
  };
  invocation_type = null;
  remove_after_reading = false;
  # The auto-activation hook (which calls back into the flox binary) is not
  # meaningful inside a container guest — no flox binary is present in the
  # image. Setting disable_hook prevents the generated rcfile from
  # registering the hook and avoids the "bash: : command not found" error
  # that occurs when the hook tries to invoke an empty flox_bin path.
  disable_hook = true;
  flox_activate_store_path = "${environmentOutPath}";
  activation_state_dir = "/run/flox/container-activations/${baseNameOf environmentOutPath}";
  attach_ctx = {
    env = "${environmentOutPath}"; # FIXME: Incorrect for containers.
    env_description = "${containerName}";
    env_cache = "/tmp";
    prompt_color_1 = "99";
    prompt_color_2 = "141";
    interpreter_path = "${interpreterPath}";
    flox_prompt_environments = "${containerName}";
    set_prompt = true;
    flox_env_cuda_detection = "0";
    flox_active_environments = "[]";
  };
  project_ctx = null;
}
