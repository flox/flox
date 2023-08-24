---
title: FLOX-CONTAINERIZE
section: 1
header: "flox User Manuals"
...


# NAME

flox-containerize - export environment as a container image

# SYNOPSIS

flox [ `<general-options>` ] containerize [ `<options>` ]

# DESCRIPTION

Export flox environment as a container image. The image is dumped to stdout and
should be piped to `docker load`.

# OPTIONS

```{.include}
./include/general-options.md
./include/environment-options.md
```
