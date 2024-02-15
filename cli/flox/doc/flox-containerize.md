---
title: FLOX-CONTAINERIZE
section: 1
header: "Flox User Manuals"
...


# NAME

flox-containerize - export an environment as a container image

# SYNOPSIS

flox [ `<general-options>` ] containerize [ `<options>` ]

# DESCRIPTION

Export a Flox environment as a container image.
The image is dumped to stdout and should be piped to `docker load`.

# OPTIONS

```{.include}
./include/environment-options.md
./include/general-options.md
```
