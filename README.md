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
  The software environment platform: one manifest, every environment, from laptop to production. All the power of Nix, none of the learning curve.
</h2>

<p align="center">
  <a href="https://github.com/flox/flox/stargazers">
    <img alt="GitHub stars" src="https://img.shields.io/github/stars/flox/flox?style=flat-square" />
  </a>
  <a href="https://github.com/flox/flox/releases">
    <img alt="Latest Release" src="https://img.shields.io/github/v/release/flox/flox?style=flat-square" />
  </a>
  <a href="https://github.com/flox/flox/blob/main/LICENSE">
    <img alt="License: GPL v2" src="https://img.shields.io/badge/License-GPL%20v2-blue.svg?style=flat-square" />
  </a>
  <a href="https://builtwithnix.org">
    <img alt="Built with Nix" src="https://builtwithnix.org/badge.svg" height="20" />
  </a>
  <a href="https://go.flox.dev/slack">
    <img alt="Slack" src="https://img.shields.io/badge/community-Slack-purple?style=flat-square&logo=slack" />
  </a>
  <a href="https://discourse.flox.dev">
    <img alt="Discourse" src="https://img.shields.io/badge/community-Discourse-blue?style=flat-square" />
  </a>
</p>

<p align="center">
  <img src="img/demo.gif" alt="Flox introductory demo" />
</p>

---

**Flox is a software environment platform for engineering teams** that delivers cryptographically pinned, reproducible environments, identical from a developer's laptop through CI to production. Built on [Nix](https://nixos.org/), with Nix knowledge optional, Flox eliminates environment drift, strengthens software supply chain security, and gives [AI coding agents](https://github.com/flox/flox-agentic) a deterministic foundation to build on.

It's more than a package manager: where traditional package managers work on a single machine, Flox manages the lifecycle of packages and environments across your entire organization.

- **Declarative.** One file describes every tool, environment variable, and service your project needs.
- **Reproducible.** The same definition produces the same environment on any supported system.
- **Composable.** Layer environments per project, per team, and per pipeline.

<table>
  <thead>
    <tr>
      <th width="25%">🚀 Try it</th>
      <th width="25%">👥 Standardize</th>
      <th width="25%">✅ Kill drift</th>
      <th width="25%">🔁 Match CI</th>
    </tr>
  </thead>
  <tbody>
    <tr>
      <td valign="top">
        <a href="https://flox.dev/docs/flox-5-minutes?utm_source=github&utm_medium=readme&utm_campaign=flox_readme&utm_content=top_5min">Get Flox running in 5 minutes</a><br>
        <small>One command. A working environment.</small>
      </td>
      <td valign="top">
        <a href="https://flox.dev/docs/tutorials/sharing-environments?utm_source=github&utm_medium=readme&utm_campaign=flox_readme&utm_content=top_sharing">Share one environment</a><br>
        <small>Same packages, every teammate.</small>
      </td>
      <td valign="top">
        <a href="https://flox.dev/case-studies/how-resolve-ai-eliminated-works-on-my-machine/?utm_source=github&utm_medium=readme&utm_campaign=flox_readme&utm_content=top_resolve">How Resolve AI did it</a><br>
        <small>A real team's before-and-after.</small>
      </td>
      <td valign="top">
        <a href="https://flox.dev/docs/tutorials/ci-cd?utm_source=github&utm_medium=readme&utm_campaign=flox_readme&utm_content=top_cicd">Reproducible CI builds</a><br>
        <small>Same environment, dev to CI.</small>
      </td>
    </tr>
  </tbody>
</table>

## Who Flox is for

- **Platform & DevX teams** standardizing toolchains across the org, cutting onboarding from days to minutes, and extending golden base environments while keeping standards intact, without forcing everyone to learn Nix.
- **Security and AppSec teams** who need SBOMs, fast CVE remediation, dependency provenance, and reproducible builds across every service.
- **Developers** who want one reproducible, per-project environment for any stack on macOS, Linux, or Windows (WSL2). More like a virtual environment than a container or VM, with nothing to spin up before you code.
- **AI coding agents** that need a deterministic, reproducible environment to build and run generated code the same way on every run.

## Why Flox?

- **Reproducible by construction.** Every environment is defined in a [declarative manifest](https://flox.dev/docs/tutorials/creating-environments?utm_source=github&utm_medium=readme&utm_campaign=flox_readme&utm_content=why_manifest) and locked to cryptographically pinned, content-hashed inputs. The same lockfile resolves to the same packages on every supported system, so environments stay identical across machines. No drift.
- **One definition, laptop to production.** The same environment runs on a developer's laptop, in an AI agent's sandbox, through CI, and in production. [Tutorial](https://flox.dev/docs/tutorials/ci-cd?utm_source=github&utm_medium=readme&utm_campaign=flox_readme&utm_content=why_cicd).
- **120,000+ packages.** Pull from [Nixpkgs](https://flox.dev/blog/nixpkgs?utm_source=github&utm_medium=readme&utm_campaign=flox_readme&utm_content=why_nixpkgs), the largest open source package repository, updated continuously.
- **Build and package your own software.** Build custom software from source into reproducible packages and publish them for your whole team, alongside everything from Nixpkgs. [Learn more](https://flox.dev/blog/introducing-flox-build-and-publish/?utm_source=github&utm_medium=readme&utm_campaign=flox_readme&utm_content=why_build_publish).
- **A software supply chain you can defend.** SBOMs, automated vulnerability and CVE patching, software composition analysis (SCA), and clean, auditable builds fall out of reproducibility instead of being bolted on afterward. [Learn more](https://flox.dev/blog/cves-are-now-being-exploited-much-faster-than-you-can-respond/?utm_source=github&utm_medium=readme&utm_campaign=flox_readme&utm_content=why_supply_chain).

## Install

The Flox CLI installs natively on macOS, Linux, and Windows (WSL2):

- [macOS](https://flox.dev/docs/install-flox/install/?utm_source=github&utm_medium=readme&utm_campaign=flox_readme&utm_content=install_macos): `brew install flox`, or the `.pkg` installer
- [Linux](https://flox.dev/docs/install-flox/install/?utm_source=github&utm_medium=readme&utm_campaign=flox_readme&utm_content=install_linux): `.deb` (Debian/Ubuntu) or `.rpm` (Fedora/RHEL) packages
- [Windows](https://flox.dev/docs/install-flox/install/?utm_source=github&utm_medium=readme&utm_campaign=flox_readme&utm_content=install_windows): via WSL2, using the Linux packages

See the [installation guide](https://flox.dev/docs/install-flox/install/?utm_source=github&utm_medium=readme&utm_campaign=flox_readme&utm_content=install_guide) for step-by-step instructions on every platform.

## Quick Start

```text
$ flox init                        # Create an environment in your project
⚡︎ Created environment 'my-project' (aarch64-darwin)

$ flox install python3 nodejs      # Install packages (any combination of tools)
✔ 'python3', 'nodejs' installed to environment 'my-project'

$ flox activate                    # Enter the environment
flox [my-project] $ python3 --version
Python 3.13.13
flox [my-project] $ node --version
v24.15.0

flox [my-project] $ exit           # Leave, and the tools are gone
$ python3 --version
python3: command not found
```

📖 New here? Follow [Flox in 5 minutes](https://flox.dev/docs/flox-5-minutes?utm_source=github&utm_medium=readme&utm_campaign=flox_readme&utm_content=quickstart_5min) to walk through it step by step.

That's the core idea: **tools appear when you activate, and disappear when you leave.** Each environment is isolated, so projects never collide and your system stays clean. And because it lives in a file, the same environment reproduces everywhere: laptop, CI, and production.

⭐ If Flox looks useful, [give us a star](https://github.com/flox/flox). It helps others discover the project. ⭐

---

## What You Can Do with Flox

| | Capability | What it means |
|---|---|---|
| **Create** | `flox init` | Declarative environments that live alongside your code and activate automatically. [Tutorial →](https://flox.dev/docs/tutorials/creating-environments?utm_source=github&utm_medium=readme&utm_campaign=flox_readme&utm_content=cap_create) |
| **Search** | `flox search` | Find any package from 120,000+ in Nixpkgs instantly. [Reference →](https://flox.dev/docs/man/flox-search?utm_source=github&utm_medium=readme&utm_campaign=flox_readme&utm_content=cap_search) |
| **Share** | `flox push` / `flox pull` | Every teammate pulls an identical environment from a single source of truth on [FloxHub](https://hub.flox.dev/?utm_source=github&utm_medium=readme&utm_campaign=flox_readme&utm_content=cap_floxhub) (our hosted environment registry). [Tutorial →](https://flox.dev/docs/tutorials/sharing-environments?utm_source=github&utm_medium=readme&utm_campaign=flox_readme&utm_content=cap_share) |
| **Containerize** | `flox containerize` | Ship any Flox environment into your existing container workflow as an OCI image, no Dockerfile required. [Reference →](https://flox.dev/docs/man/flox-containerize?utm_source=github&utm_medium=readme&utm_campaign=flox_readme&utm_content=cap_containerize) |
| **Build & publish** | `flox build` / `flox publish` | Build your own software from source into reproducible packages, and publish them for your team to install. [Blog →](https://flox.dev/blog/introducing-flox-build-and-publish/?utm_source=github&utm_medium=readme&utm_campaign=flox_readme&utm_content=cap_build) |
| **Services** | `flox services start` | Run databases, queues, and background processes as part of your environment (a lightweight alternative to Docker Compose for local development); they start on activate and stop on exit. [Concepts →](https://flox.dev/docs/concepts/services?utm_source=github&utm_medium=readme&utm_campaign=flox_readme&utm_content=cap_services) |
| **Configure** | `manifest.toml` | Define environment variables, shell hooks, and activation scripts declaratively alongside your code. [Manifest reference →](https://flox.dev/docs/concepts/manifest?utm_source=github&utm_medium=readme&utm_campaign=flox_readme&utm_content=cap_configure) |
| **AI-ready** | Works with Claude Code, Cursor, Copilot, and Codex | Give AI coding agents a deterministic, reproducible environment so generated code builds and runs the same on every run. [flox-agentic →](https://github.com/flox/flox-agentic) |

> **Origin:** Flox emerged from one of the largest enterprise Nix deployments in the world, at the D.E. Shaw group, where it made Nix accessible at scale across a large engineering organization.

---

## For platform teams

Real teams using Flox: [Fellow.ai ships faster](https://flox.dev/case-studies/how-fellow-ai-ships-faster-with-flox/?utm_source=github&utm_medium=readme&utm_campaign=flox_readme&utm_content=platform_fellow) · [Resolve AI eliminated works-on-my-machine](https://flox.dev/case-studies/how-resolve-ai-eliminated-works-on-my-machine/?utm_source=github&utm_medium=readme&utm_campaign=flox_readme&utm_content=platform_resolve).

Rolling this out across your organization (private registry, SSO, self-hosting, support)? [Talk to us](https://flox.dev/contact/?utm_source=github&utm_medium=readme&utm_campaign=flox_readme&utm_content=platform_talk_to_us).

---

## FAQ

<details>
<summary><b>Is Flox a package manager?</b></summary>

Flox does manage packages, so in part yes, but it's more than a package manager. Traditional package managers work on a single machine; Flox manages the lifecycle of packages and environments across your whole organization, delivering complete, reproducible, shareable environments, not just installed packages. That makes it a software environment platform.
</details>

<details>
<summary><b>How is Flox different from Docker?</b></summary>

Flox is not a container technology, and it is not a Docker replacement. The difference: Flox believes software packaging should be distinct from the chosen type of isolation. With Docker, packaging and container isolation are often mixed together. Flox environments work the same on bare metal, in VMs, and in containers. The `flox containerize` command creates OCI images with the software environment baked in, and works great with Docker, Kubernetes, and other container runtimes.
</details>

<details>
<summary><b>I already use Nix. What does Flox add?</b></summary>

Flox is additive to Nix, not a replacement. It adds a centralized service, FloxHub, for collaborative environment and package sharing, making it easy to find, use, publish, and share environments across an organization. Flox environments also bundle activation hooks, services, and shell profiles in a single declarative TOML file.
</details>

<details>
<summary><b>How does Flox help with software supply chain security?</b></summary>

Flox environments are pinned and reproducible, so SBOM generation, software composition analysis (SCA), automated vulnerability and CVE patching, dependency provenance, and clean, auditable builds all become tractable.
</details>

<details>
<summary><b>Can Flox give AI coding agents a deterministic environment?</b></summary>

Yes. Flox gives AI coding agents (Claude Code, Cursor, Copilot, Codex) a deterministic, reproducible environment so generated code builds and runs against identical dependencies on every run.
</details>

<details>
<summary><b>Do I need to know Nix?</b></summary>

You don't have to. Flox is powered by Nix but designed so you create, share, and run environments, and even some builds, without writing Nix. If you do know Nix, you can drop into it directly when you need to.
</details>

---

## Community & Support

- [Documentation](https://flox.dev/docs?utm_source=github&utm_medium=readme&utm_campaign=flox_readme&utm_content=footer_docs): tutorials, reference, and guides
- [FloxHub](https://hub.flox.dev/?utm_source=github&utm_medium=readme&utm_campaign=flox_readme&utm_content=footer_floxhub): discover and share environments
- [Discourse](https://discourse.flox.dev/?utm_source=github&utm_medium=readme&utm_campaign=flox_readme&utm_content=footer_discourse): questions, discussions, and announcements
- [Blog](https://flox.dev/blog?utm_source=github&utm_medium=readme&utm_campaign=flox_readme&utm_content=footer_blog): deep dives and workflows
- [Slack](https://go.flox.dev/slack): real-time chat with the team
- [YouTube](https://www.youtube.com/@floxdev): demos and walkthroughs
- [GitHub Issues](https://github.com/flox/flox/issues/new/choose): bug reports and feature requests
- [VS Code extension](https://marketplace.visualstudio.com/items?itemName=flox.flox): manage Flox environments from your editor

## Security

We encourage responsible disclosure of potential security issues. For any security-related inquiry, please contact us at: **security@flox.dev**

## Contributing

We welcome contributions! Please read the [Contributor guide](./CONTRIBUTING.md) first.

## License

The Flox CLI is licensed under the GPLv2. See [LICENSE](./LICENSE).

<img referrerpolicy="no-referrer-when-downgrade" src="https://static.scarf.sh/a.png?x-pxid=199c01a0-67c0-4d95-a4c1-5c2d78aa7743" />
