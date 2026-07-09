# The nixpkgs-free context builder for the store-volume refresh fast path.
#
# Returns the store path of the activations-context JSON via `builtins.toFile`,
# which realises during evaluation (no derivation, no builder run). Because it
# imports the same `activate-ctx.nix` as a full bake, the JSON it writes is
# byte-identical to the one `mkContainer.nix` embeds for the same inputs.
#
# Args arrive as `--argstr` (plain strings, no string context), which is what
# lets `builtins.toFile` accept the interpolated JSON. Evaluating this is ~0.1s
# because it never touches nixpkgs — the caller passes the pre-resolved
# `bashPath` from the host binary-resolution cache.
{
  bashPath,
  environmentOutPath,
  interpreterPath,
  activationMode,
  containerName,
}:
builtins.toFile "activations-context" (
  builtins.toJSON (
    import ./activate-ctx.nix {
      inherit
        bashPath
        environmentOutPath
        interpreterPath
        activationMode
        containerName
        ;
    }
  )
)
