# Error

```
# Writing manifest to image destination
# time="2024-12-17T13:17:59-07:00" level=debug msg="DoRequest Method: GET URI: http://d/v5.2.3/libpod/images/ghcr.io%2Fflox%2Fflox:v1.3.8/json"
# time="2024-12-17T13:17:59-07:00" level=debug msg="DoRequest Method: POST URI: http://d/v5.2.3/libpod/containers/create"
# Error: statfs /tmp/home.D4hkdr: no such file or directory
# time="2024-12-17T13:17:59-07:00" level=debug msg="Shutting down engines"
# Error: unable to load image: payload does not match any of the supported image formats:
#  * oci: open /var/tmp/libpod-images-load.tar2221354042/index.json: not a directory
#  * oci-archive: loading index: open /var/tmp/container_images_oci2006874804/index.json: no such file or directory
#  * docker-archive: loading tar component "manifest.json": file does not exist
#  * dir: open /var/tmp/libpod-images-load.tar2221354042/manifest.json: not a directory
# ❌ ERROR: Writing to runtime was unsuccessful
```

Matthew says that he sees this same error when `flox` fails to build.

- I verified that `flox` builds my itself in this container.
- I verified that the tmp home directory `/tmp/home.XXXXXX` exists outside
  the container.

Confirmed with Matthew that it's just the second part of the error that he sees,
which suggests that the home directory isn't being mounted properly.

Stopped the podman machine outside of the tests so that tests will restart it.
Same issue.

Full command:
```
/nix/store/p3lz44rirshy6z1f3mzdgk2py9mqssjx-podman-5.2.3/bin/podman run --rm --userns keep-id --log-level debug --mount type=bind,source=/private/tmp/nix-shell.tvmXfz/bats-run-wzT5Ib/test/1/test,target=/flox_env --env HOME=/flox_home --mount type=bind,source=/tmp/home.SoDC1j,target=/flox_home --env FLOX_DISABLE_METRICS=true ghcr.io/flox/flox:v1.3.8 nix --extra-experimental-features nix-command flakes run github:flox/flox/v1.3.8 -- containerize -vvv --dir /flox_env --tag latest --file -
```

Debugging command:
```
/nix/store/p3lz44rirshy6z1f3mzdgk2py9mqssjx-podman-5.2.3/bin/podman run --rm --userns keep-id --log-level debug --mount type=bind,source=/Users/zmitchell/src/flox/macos-containerize-ci/my_env,target=/flox_env --env HOME=/flox_home --mount type=bind,source=/tmp/home.SoDC1j,target=/flox_home --env FLOX_DISABLE_METRICS=true ghcr.io/flox/flox:v1.3.8 nix --extra-experimental-features nix-command flakes run github:flox/flox/v1.3.8 -- containerize -vvv --dir /flox_env --tag latest --file img.tar.gz
```

I'm now at an error regarding users and permissions:
```
DEBU[0029] DoRequest Method: POST URI: http://d/v5.2.3/libpod/containers/6154375310ac7a6dac31174cfd60524e52cbd80f44f5ed2a2f4b01b5dc49e802/start
error: could not set permissions on '/nix/var/nix/profiles/per-user' to 755: Operation not permitted
```

There are these different possible users:
- user on my host machine
- user for the `podman` invocation in the test
- user in the VM
- user in the container

Running on macOS via `just integ-tests` all four of those seem to be `zmitchell`.
I tried setting the user in the `podman` wrapper to `zmitchell`, but that didn't do
anything (as expected) because the user was already set to `zmitchell` everywhere.

I ran this command to put me inside the container:
```
USER=podman-user /nix/store/p3lz44rirshy6z1f3mzdgk2py9mqssjx-podman-5.2.3/bin/podman run -it --rm --userns keep-id --log-level debug --mount type=bind,source=/Users/zmitchell/src/flox/macos-containerize-ci/my_env,target=/flox_env --env HOME=/flox_home --mount type=bind,source=/tmp/home.SoDC1j,target=/flox_home --env FLOX_DISABLE_METRICS=true ghcr.io/flox/flox:v1.3.8 bash
```

Observations:
- `USER` inside the container is `root`.
- `id` does not say that my user ID is the same as `root`
  - `uid=501(core) gid=1000(core) groups=1000(core)`
- `/nix` and everything under it is owned by root.

Noticing that my user ID was the same, but the user itself wasn't, I decided
to change the `--userns` mapping from `keep-id` to `''`, which makes the user
inside the container `root`.
I had to delete the `/flox_home/.cache/nix` directory because it's a symlink
to the host machine that I don't think gets propagated to the container.

`flox containerize` fails with no other errors other than "could not build container".
I switched from `github:flox/flox/v1.3.8` to `github:flox/flox` and I can now
run `flox containerize`.

# Running one machine for the whole file

Doesn't work right now, probably because sockets aren't where podman expects.

Looks like `nix develop` does this at startup:
- Create a temporary directory with template `nix-shell.XXXXXX` under the
  default temporary directory (`/tmp`).
- Sets `"TMP", "TMPDIR", "TEMP", "TEMPDIR"` to point to this directory.

That explains why you see a bunch of paths that start with `/tmp/nix-shell.XXXXXX`
when debugging things.
`podman` appears to create a temporary directory to put runtime data in,
rooted at `TEMPDIR/podman`.

`podman system connection list` reports sockets at `~/.local/share/containers/podman/machine/machine`.
That means we probably need to symlink `XDG_DATA_HOME` into the temporary home
directory.

The current failure is 
```
# Error: unable to connect to Podman socket: Get "http://d/v5.2.3/libpod/_ping":
dial unix /tmp/nix-shell.tvmXfz/storage-run-501/podman/podman.sock: connect: no such file or directory
```

It looks like `skopeo` creates the `storage-run-*` directory:
- If `XDG_RUNTIME_DIR` is set, put it there.
- If not, put it under `TEMPDIR` (which is set under `/tmp/nix-shell.XXXXXX`)
  for us.
