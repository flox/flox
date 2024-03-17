<h1 align="center">
  <a href="https://flox.dev" target="_blank">
    <picture>
      <source media="(prefers-color-scheme: dark)"  srcset="img/flox-logo-white-on-black.png" />
      <source media="(prefers-color-scheme: light)" srcset="img/flox-logo-black-on-white.png" />
      <img src="img/flox-logo-black-on-white.png" alt="flox logo" />
    </picture>
  </a>
</h1>

<h2 align="center">
  Developer environments you can take with you
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
  <a href="https://github.com/flox/flox/blob/main/LICENSE"> 
    <img alt="GitHub" src="https://img.shields.io/github/license/flox/flox?style=flat-square">
  </a>
  <a href="https://github.com/flox/flox/blob/main/CONTRIBUTING.md">
    <img alt="PRs Welcome" src="https://img.shields.io/badge/PRs-welcome-brightgreen.svg?style=flat-square"/>
  </a>
  <a href="https://github.com/flox/flox/releases">
    <img alt="flox version" src="https://img.shields.io/badge/version-beta-brightgreen?style=flat-square"/>
    <!-- <img alt="GitHub tag (latest by date)" src="https://img.shields.io/github/v/tag/flox/flox?label=Version&style=flat-square"> -->
  </a>
</p>

[Flox][website] is a virtual environment and package manager all in one. With Flox you 
create environments that layer and replace dependencies just where
it matters, making them portable across the full software lifecycle.

Install packages from [the biggest open source repository
(nixpkgs)][post-nixpkgs] that contains **more that 80.000 packages**.

With `flox` you can:<br/>
&rarr; [Create environments](https://flox.dev/docs/tutorials/creating-environments)<br/>
&rarr; [Share your environments with others](https://flox.dev/docs/tutorials/sharing-environments)<br/>
&rarr; [Build container images](https://flox.dev/docs/tutorials/sharing-environments/#sharing-with-containers)<br/>
&rarr; [... and much more][docs]<br/>

<div align="center">
  <a href="https://flox.dev/docs/#install-flox">
    <img alt="install flox" src="https://img.shields.io/badge/Install-flox-brightgreen?style=for-the-badge"/>
  </a>
</div>

<!-- TODO: Compare with others
- [Flox vs Docker]
- [Flox vs Homebrew]
- [Flox vs Bazel]
- .. more (point to the website)
-->

## ⚡️ Quick start

``` text
$ flox init           # <- Create an environment in the current directory ✨.

$ flox search hello   # <- Search for a package 🚀.

$ flox install hello  # <- Install packages into current directory's environment 🔨.

$ flox activate       # <- Enter current directory's environment 🎆.

flox [my-project] $ hello   # <- Have fun 🎉.
Hello world!

flox [my-project] $ exit    # <- Exit environment 💃.
```

## ❓ Why

We all build software on top of a dynamic set of tools,
frameworks and packages, allowing us to move quickly and only
build what’s necessary. However, each new wave of dev tooling
innovation results in an entirely new set of dependencies that
need to be managed. What starts as a simple app or microservice
quickly grows complex, and
turns into a expanding and fragmented supply
chain. Flox brings reproducibility and consistency to complex
software development lifecycles.

## 📘 Origins

Flox began its life during the deployment of Nix at
the D. E. Shaw group, where it quickly proved invaluable
by making Nix easier for newcomers and offering centralized
control over packages. As a result, their successful project
became one of the largest, most impactful enterprise deployments
of Nix.

## 📫 Have a question? Want to chat? Ran into a problem?

We are happy to welcome you to our [Discourse forum][discourse] and answer your
questions! You can always reach out to us directly via the [Flox twitter
account][twitter] or chat to us directly on [Slack][slack] or
[Discord][discord].

## 🤝 Found a bug? Missing a specific feature?

Feel free to [file a new issue][new-issue] with a respective title and
description on the the `flox/flox` repository. If you already found a solution
to your problem, we would love to review your pull request!

## ⭐️ Contribute

We welcome contributions to this project. Please read the [Contributor
guide](./CONTRIBUTING.md) first.

## 🪪 License

The Flox CLI is licensed under the GPLv2. See [LICENSE](./LICENSE).


[website]: https://flox.dev
[discourse]: https://discourse.flox.dev
[nix]: https://nixos.org
[basics]:https://flox.dev/docs
[share-envs]: https://flox.dev/docs/share-environments
[images]: docs/tutorials/sharing-environments/#sharing-with-containers
[docs]: https://flox.dev/docs
[twitter]: https://twitter.com/floxdevelopment
[slack]: https://go.flox.dev/slack
[discord]: https://discord.gg/5H7hN57eQR
[new-issue]: https://github.com/flox/flox/issues/new/choose
[post-nixpkgs]: https://flox.dev/blog/nixpkgs
