---
title: FLOX-CONTAINERIZE
section: 1
header: "Flox User Manuals"
...

```{.include}
./include/experimental-warning.md
```

# NAME

flox-containerize - export an environment as a container image

# SYNOPSIS

```
flox [<general-options>] containerize
     [-d=<path> | -r=<owner/name>]
     [-o=<path>]
     [--tag=<tag>]
     [--load-into-registry=backend]
```

# DESCRIPTION

Export a Flox environment as a container image.
The image is written to `<path>`.
Then use `docker load -i <path>` to load the image into docker.
When `<path>` is `-`, the image is written to `stdout`,
and can be piped into `docker load` directly.

Running the container will behave like running `flox activate`.
Running the container interactively with `docker run -it <container id>`,
will launch a bash subshell in the container
with all your packages and variables set after running the activation hook.
This is akin to `flox activate`

Running the container non-interactively with `docker run <container id>`
allows you to run a command within the container without launching a subshell,
similar to `flox activate --`


**Note**:
The `containerize` command is currently **only available on Linux**.
The produced container however can also run on macOS.

# OPTIONS

`-o`, `--output`
:   Write the container to `<path>`
    (default: `./<environment-name>-container.tar`)
    If `<path>` is `-`, writes to `stdout`.

`-t`, `--tag`
:   Tag the container with `<tag>`
    (default: `latest`)

`-l`, `--load-into-registry`
:   Loads the container into `<backend>` without having to pipe
    Has to be one of `docker` or `podman`.

```{.include}
./include/environment-options.md
./include/general-options.md
```

# EXAMPLES

Create a container image file and load it into Docker:

```
$ flox containerize -o ./mycontainer.tar
$ docker load -i ./mycontainer.tar
```

Pipe the image into Docker directly:

```
$ flox containerize -o - | docker load
```

Run the container interactively:

```
$ flox init
$ flox install hello
$ flox containerize -o - | docker load
$ docker run --rm -it <container id>
[floxenv] $ hello
Hello, world!
```

Run a specific command from within the container,
but do not launch a subshell.

```
$ flox init
$ flox install hello
$ flox containerize -o - | docker load
$ docker run <container id> hello
Hello, world
```

Create a container with a specific tag:

```
$ flox init
$ flox install hello
$ flox containerize --tag 'v1' -o - | docker load
$ docker run --rm -it <container name>:v1
[floxenv] $ hello
Hello, world!
```

Create a container, and load it into Docker's local registry directly:

```
$ flox init
$ flox install hello
$ flox containerize --load-into-registry=docker
$ docker run --rm -it <container name>:latest
[floxenv] $ hello
Hello, world!
```

# SEE ALSO

[`flox-activate(1)`](./flox-activate.md)
[`docker-load(1)`]
