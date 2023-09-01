<h1 align="center">
  <a href="https://flox.dev" target="_blank">
    <picture>
      <source media="(prefers-color-scheme: dark)"  srcset="img/flox_orange_small.png" />
      <source media="(prefers-color-scheme: light)" srcset="img/flox_blue_small.png" />
      <img src="img/flox_blue_small.png" alt="flox logo" />
    </picture>
  </a>
</h1>

<h2 align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)"  srcset="img/harness_the_power_of_nix_dark.svg" />
    <source media="(prefers-color-scheme: light)" srcset="img/harness_the_power_of_nix_light.svg" />
    <img height="24" src="img/harness_the_power_of_nix_light.svg" alt="Harness the Power of Nix" />
  </picture>
</h2>

<!-- TODO: here comes the graphic
 show immediate value proposition
 a short demo of basics would be good for now
 a bold statement: Free yourself from container walls.
-->

<h3 align="center">
   &emsp;
   <a href="https://discourse.flox.dev"><b>Discourse</b></a>
   &emsp; | &emsp; 
   <a href="https://flox.dev/docs"><b>Documentation</b></a>
   &emsp; | &emsp; 
   <a href="https://flox.dev/blog"><b>Blog</b></a>
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
your environments**. flox builds on top of the powerful ideas of [Nix][nix] as
well as making them accessible to everybody.

Install packages from [the biggest open source repository
(nixpkgs)][post-nixpkgs] that contains **more that 80.000 packages**.

With `flox` you can:<br/>
&rarr; [Create composable environments](https://flox.dev/docs/tutorials/projects)<br/>
&rarr; [Share your environments with others](https://flox.dev/docs/cookbook/managed-environments/#share-an-environment)<br/>
&rarr; [Build container images](https://flox.dev/docs/tutorials/build-container-images)<br/>
&rarr; [... and much more][docs]<br/>

<div align="center">
  <a href="https://flox.dev/docs/#install-flox">
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

``` text
$ flox search hello           # <- Search for a package 🚀.

$ flox install -e demo hello  # <- Install packages into an environment 🔨.

$ flox activate -e demo       # <- Enter the environment shell 🎆.

flox [demo default] $ hello   # <- Have fun 🎉.
Hello world!

flox [demo default] $ exit    # <- Exit environment 💃.
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

## 🪪 License

The flox CLI is licensed under the GPLv2. See [LICENSE](./LICENSE).


[website]: https://flox.dev
[discourse]: https://discourse.flox.dev
[nix]: https://nixos.org
[basics]:https://flox.dev/docs/basics
[share-envs]: https://flox.dev/docs/share-environments
[images]: https://flox.dev/docs/build-container-images
[docs]: https://flox.dev/docs
[twitter]: https://twitter.com/floxdevelopment
[matrix]: https://matrix.to/#/#flox:matrix.org
[discord]: https://discord.gg/5H7hN57eQR
[new-issue]: https://github.com/flox/flox/issues/new/choose
[post-nixpkgs]: https://flox.dev/blog/nixpkgs
