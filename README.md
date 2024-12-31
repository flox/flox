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
   &emsp; | &emsp; 
   <a href="https://go.flox.dev/slack"><b>Slack</b></a>
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
    <img alt="flox version" src="https://img.shields.io/github/v/release/flox/flox?style=flat-square"/>
    <!-- <img alt="GitHub tag (latest by date)" src="https://img.shields.io/github/v/tag/flox/flox?label=Version&style=flat-square"> -->
  </a>
</p>

[Flox][website] is a virtual environment and package manager all in one. With Flox you 
create environments that layer and replace dependencies just where
it matters, making them portable across the full software lifecycle.

Install packages from [the biggest open source repository
(nixpkgs)][post-nixpkgs] that contains **more than 80.000 packages**.

With `flox` you can:<br/>
&rarr; [Create environments](https://flox.dev/docs/tutorials/creating-environments)<br/>
&rarr; [Share your environments with others](https://flox.dev/docs/tutorials/sharing-environments)<br/>
&rarr; [Build container images](https://flox.dev/docs/tutorials/sharing-environments/#sharing-with-containers)<br/>
&rarr; [... and much more][docs]<br/>

<div align="center">
  <a href="https://flox.dev/docs/install-flox">
    <img alt="install flox" src="https://img.shields.io/badge/Install-flox-brightgreen?style=for-the-badge"/>
  </a>
</div>

<!-- TODO: Compare with others
- [Flox vs Docker]
- [Flox vs Homebrew]
- [Flox vs Bazel]
- .. more (point to the website)
-->

# Flox vs Docker:
Before demonstrating the difference, let's clarify what Docker is. Historically, the fundamental operation of Docker is based on `chroot`, a system call introduced in Version 7 Unix. This system call changes the root directory for the current process, making the specified directory appear as the root (`/`) for any process running within it. Later, Linux adopted this feature, which became known as Linux Containers. In 2008, Docker was built based on this concept. Simply put, a container is a running process that uses virtualization-based mechanisms, but each of these containers perceives its own environment as if it were the host system. Given this, you can see Flox is different than Docker.

# Flox vs Homebrew:

This document compares **Homebrew** and **Flox**, two popular package managers used in modern development environments. Each has its own strengths and is suited to different use cases. Here's a side-by-side comparison of their key features:

## Installation and Package Management

- **Homebrew** is a package manager primarily for macOS. It allows users to easily install, update, and manage software packages with a simple command-line interface. Packages are installed in standard directories, making it user-friendly for macOS users.
  
- **Flox** is a cross-platform, purely functional package manager available for multiple operating systems. It isolates packages in their own environments, ensuring reproducibility and conflict-free installations. Flox’s approach is more controlled, allowing precise environment management.

## Package Availability

- **Homebrew** provides a large number of packages through its main repository, **Homebrew Core**. Users can also tap into additional repositories, known as **taps**, for even more software.
  
- **Flox** comes with **Nixpkgs**, a curated repository of packages. However, Flox also allows users to define and share custom package sets. This flexibility makes it highly adaptable for specialized workflows and environments.

## Version Management

- **Homebrew** offers easy version management, allowing users to switch between package versions using the `brew switch` command. This is useful when certain packages depend on specific versions.
  
- **Flox** handles version management differently. It allows multiple versions of a package to be installed and used concurrently, each in its own isolated environment. This eliminates version conflicts and makes it simple to switch between different package versions.

## Reproducibility

- **Homebrew** focuses on convenience and ease of use. However, it does not guarantee full reproducibility of package installations across systems. This can lead to inconsistencies when trying to replicate environments.
  
- **Flox** is designed with **reproducibility** at its core. It ensures that package installations are fully declarative and can be reproduced exactly across different systems, making it ideal for environments where consistency is critical (e.g., scientific computing, large-scale deployments).

## Rollbacks and Atomic Upgrades

- **Homebrew** does not natively support rollbacks or atomic upgrades. If an installation or update fails, it can leave the system in an inconsistent state.
  
- **Flox** supports **atomic upgrades** and **rollbacks**. It uses a transactional approach to package installations, ensuring that changes are applied in a consistent and reliable manner. If something goes wrong, the system can easily be rolled back to its previous state.

## Customizability and Extensibility

- **Homebrew** is simple and easy to use, making it ideal for casual users and those who need a straightforward package management experience. Its primary focus is on usability and quick installations.
  
- **Flox** is highly **customizable** and **extensible**, offering users the ability to define custom package sets, build configurations, and even entire environments. Flox is based on Nix, a functional package manager, which gives advanced users full control over their package management workflow.

## ⚡️ Quick start

``` text
$ flox init           # <- Create an environment in current directory ✨.

$ flox search hello   # <- Search for a package 🚀.

$ flox install hello  # <- Install packages into current directory's environment 🔨.

$ flox activate       # <- Enter the current directory's environment 🎆.

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
turns into an expanding and fragmented supply
chain. Flox brings reproducibility and consistency to complex
software development life-cycles.

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
account][twitter] or chat with us directly on [Slack][slack].

## 🤝 Found a bug? Missing a specific feature?

Feel free to [file a new issue][new-issue] with a respective title and
description on the `flox/flox` repository. If you already found a solution
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
[new-issue]: https://github.com/flox/flox/issues/new/choose
[post-nixpkgs]: https://flox.dev/blog/nixpkgs
