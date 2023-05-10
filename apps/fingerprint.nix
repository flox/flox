{
  writeShellApplication,
  jq,
  coreutils,
  name ? "fingerprint",
  ...
}: {
  type = "app";
  program =
    (writeShellApplication {
      inherit name;
      runtimeInputs = [jq coreutils];
      text = ''
        outPath=$(nix flake metadata  --json | jq .path -r)
        self=$(echo "$outPath" | cut -d/ -f4)
        rev=$(nix flake metadata  --json | jq '.revCount // 0' -r)
        lastMod=$(nix flake metadata  --json | jq ' .lastModified // 0' -r)
        echo "$self" >&2
        set -x
        hash=$(printf "%s;%s;%d;%d;%s" "$self" "" "$rev" "$lastMod" "$(cat ./flake.lock || echo '{
          "nodes": {
            "root": {}
          },
          "root": "root",
          "version": 7
        }
        ')" | sha256sum | cut -d' ' -f1)
        printf "$HOME/.cache/nix/eval-cache-v4/%s.sqlite\n" "$hash"
      '';
    })
    + "/bin/${name}";
}
