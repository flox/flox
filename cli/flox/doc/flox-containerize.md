---
title: FLOX-CONTAINERIZE
section: 1
header: "Flox User Manuals"
...

# NAME

flox-containerize - export an environment as a container image

# SYNOPSIS

```
flox [<general-options>] containerize
     [-d=<path> | -r=<owner/name>]
     [-f=<file> | --runtime=<runtime>]
     [--tag=<tag>]
```

# DESCRIPTION

Export a Flox environment as a container image.
The image is written to the specified output target.
With `--file|-f <file>` a tarball is writtent to the specified file.
When `-` is passed as `<file>` the image is instead written to stdout.
The `--runtime <runtime>` flag supports `docker` and `podman`,
and expects the selected runtime to be found in PATH.

When neither option is provided,
the container is loaded into a supported runtime,
`docker` or `podman`, whichever is found first on the PATH.
If no supported runtime is found,
the container is written to `./<env name>-container.tar` instead.

Running the container will behave like running `flox activate`.
Running the container interactively with `docker run -it <container id>`,
will launch a bash subshell in the container
with all your packages and variables set after running the activation hook.
This is akin to `flox activate`.

Running the container non-interactively with `docker run <container id>`
allows you to run a command within the container without launching a subshell,
similar to `flox activate --`.

**Note**:
The `containerize` command is currently **only available on Linux**.
The produced container however can also run on macOS.

# OPTIONS

`-f`, `--file`
:   Write the container image to `<file>`.
    If `<output target>` is `-`, writes to `stdout`.

`--runtime`
:   Load the image into the specified `<runtime>`.
    `<runtime>` may bei either `docker` or `podman`.
    The specified binary must be found in `PATH`.

```{.include}
./include/environment-options.md
./include/general-options.md
```

# EXAMPLES

Create a container image file and load it into Docker:

```
$ flox containerize -f ./mycontainer.tar
$ docker load -i ./mycontainer.tar
```

Load the image into Docker:

```
$ flox containerize --runtime docker

# or through stdout e.g. if `docker` is not in `PATH`:

$ flox containerize -f - | /path/to/docker
```

Run the container interactively:

```
$ flox init
$ flox install hello
$ flox containerize -f - | docker load
$ docker run --rm -it <container id>
[floxenv] $ hello
Hello, world!
```

Run a specific command from within the container,
but do not launch a subshell.

```
$ flox init
$ flox install hello
$ flox containerize -f - | docker load
$ docker run <container id> hello
Hello, world
```

Create a container with a specific tag:

```
$ flox init
$ flox install hello
$ flox containerize --tag 'v1' -f - | docker load
$ docker run --rm -it <container name>:v1
[floxenv] $ hello
Hello, world!
```

# SEE ALSO

[`flox-activate(1)`](./flox-activate.md)
[`docker-load(1)`]
