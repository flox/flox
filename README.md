<h1 align="center">
  <a href="https://floxdev.com" target="_blank">
    <img src="img/flox_blue_small.png" alt="flox logo" />
  </a>
</h1>

<h2 align="center">
  <img style="height:1em;" src="img/harness_the_power_of_nix.svg" alt="Harness the Power of Nix" />
</h2>

<!-- TODO: here comes the graphic
 show immediate value proposition
 a short demo of basics would be good for now
 a bold statement: Free yourself from container walls.
-->

<h3 align="center">
   &emsp;
   <a href="https://discourse.floxdev.com"><b>Discourse</b></a>
   &emsp; | &emsp; 
   <a href="https://floxdev.com/docs"><b>Documentation</b></a>
   &emsp; | &emsp; 
   <a href="https://floxdev.com/blog"><b>Blog</b></a>
   &emsp; | &emsp;  
   <a href="https://twitter.com/floxdevelopment"><b>Twitter</b></a>
   &emsp;
</h3>

<p align="center">
  <a href="https://github.com/flox/flox/blog/main/LICENSE">
    <img alt="GitHub" src="https://img.shields.io/github/license/flox/flox?style=flat-square">
  </a>
  <a href="https://github.com/flox/flox/blog/main/CONTRIBUTING.md">
    <img alt="PRs Welcome" src="https://img.shields.io/badge/PRs-welcome-brightgreen.svg?style=flat-square"/>
  </a>
  <a href="https://github.com/flox/flox/releases">
    <img alt="flox version" src="https://img.shields.io/badge/version-beta-brightgreen?style=flat-square"/>
    <!-- <img alt="GitHub tag (latest by date)" src="https://img.shields.io/github/v/tag/flox/flox?label=Version&style=flat-square"> -->
  </a>
</p>

[flox][website] is a command line tool that helps you **manage
your environments**. flox builds on top of a powerfull ideas of [Nix][nix] as
well as making them accessible to everybody.

With `flox` you can:<br/>
&rarr; [Create composable environments][basics]<br/>
&rarr; [Share your environments with others][share-envs]<br/>
&rarr; [Build container images][images]<br/>
&rarr; [... and much more][docs]<br/>

<div align="center">
  <a href="https://floxdev.com/docs/#install-flox">
    <img alt="install flox" src="https://img.shields.io/badge/Install-flox-brightgreen?style=for-the-badge"/>
  </a>
</div>

<!-- TODO: Compare with others
- [flox vs Docker]
- [flox vs Homebrew]
- [flox vs Bazel]
- .. more (point to the website)
-->


## ⚡️ Quick start

``` console
$ flox search hello           # <- to search for package
$ flox install -e demo hello  # <- to install packages into an environment
$ flox activate -e demo       # <- to enter an environment shell

flox [demo default] $ hello   # <- have fun!
Hello world!
flox [demo default] $ exit    # <- exit environment
$
```

## 📫 Have a question? Want to chat? Ran into a problem?

We are happy to welcome you to our [Discourse forum][discourse] and answer your
questions! You can always reach out to us directly via the [flox twitter
account][twitter] or chat to us directly on [Matrix][matrix] or
[Discord][discord].

## 🤝 Found a bug? Missing a specific feature?

Feel free to [file a new issue][new-issue] with a respective title and
description on the the `flox/flox` repository. If you already found a solution
to your problem, we would love to review your pull request!

## ⭐️ Contribute

We welcome contributions to this project. Please read the [Contributor
guide](./CONTRIBUTING.md) first.

## ⚠️ License

The flox CLI is licensed under the GPLv2. See [LICENSE](./LICENSE).


[website]: https://floxdev.com
[discourse]: https://discourse.floxdev.com
[nix]: https://nixos.org
[basics]:https://floxdev.com/docs/basics
[share-envs]: https://floxdev.com/docs/share-environments
[images]: https://floxdev.com/docs/build-container-images
[docs]: https://floxdev.com/docs
[twitter]: https://twitter.com/floxdevelopment
[matrix]: https://matrix.to/#/#flox:matrix.org
[discord]: https://discord.gg/mxUgrRGP
[new-issue]: https://github.com/flox/flox-private/issues/new/choose
