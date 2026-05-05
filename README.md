<h1 align="center">
  <a href="https://flox.dev" target="_blank">
    <picture>
      <source media="(prefers-color-scheme: dark)"  srcset="img/flox-logo-white-on-black.png" />
      <source media="(prefers-color-scheme: light)" srcset="img/flox-logo-black-on-white.png" />
      <img src="img/flox-logo-black-on-white.png" alt="flox logo" />
    </picture>
  </a>
</h1>

<h3 align="center">
  Reproducible developer environments for any stack — install 120,000+ packages, share environments with your team, and build container images. Powered by Nix, but you don't have to know Nix.
</h3>

<p align="center">
  <a href="https://github.com/flox/flox/releases">
    <img alt="Latest Release" src="https://img.shields.io/github/v/release/flox/flox?style=flat-square" />
  </a>
  <a href="https://github.com/flox/flox/blob/main/LICENSE">
    <img alt="License: GPL v2" src="https://img.shields.io/badge/License-GPL%20v2-blue.svg?style=flat-square" />
  </a>
<a href="https://discourse.flox.dev">
    <img alt="Discourse" src="https://img.shields.io/badge/community-Discourse-blue?style=flat-square" />
  </a>
  <a href="https://go.flox.dev/slack">
    <img alt="Slack" src="https://img.shields.io/badge/community-Slack-purple?style=flat-square&logo=slack" />
  </a>
</p>

<p align="center">
  <img src="img/demo.gif" alt="Flox introductory demo" />
</p>

---

## Why Flox?

- **Works on macOS and Linux** — no containers, no VMs, no configuration. Just install and go.
- **Perfectly reproducible** — share an environment and it reproduces exactly on any machine, every time.
- **120,000+ packages** — powered by [Nixpkgs][post-nixpkgs], the largest open source package repository, updated continuously.

## Install

Flox provides native packages for each supported platform:

- **macOS** — `pkg` installer or Homebrew
- **Linux** — `.deb` or `.rpm`

See the [installation guide](https://flox.dev/docs/install-flox/install/) for step-by-step instructions.

## Quick Start

```text
$ flox init                        # Create an environment in your project
✨ Created environment my-project

$ flox install python3 nodejs      # Install packages — any combination of tools
✅ python3 installed
✅ nodejs installed

$ flox activate                    # Enter the environment
flox [my-project] $ python3 --version
Python 3.12.4
flox [my-project] $ node --version
v20.15.0

flox [my-project] $ exit           # Leave — and the tools are gone
$ python3 --version
python3: command not found
```

That's the core idea: **tools appear when you activate, and disappear when you leave.** Your global system stays clean. Your project stays reproducible.

📖 Ready to dive deeper? Check out the [full documentation](https://flox.dev/docs) for tutorials and guides.

⭐ If Flox looks useful, [give us a star](https://github.com/flox/flox) — it helps others discover the project. ⭐

---

## What You Can Do with Flox

| | Capability | What it means |
|---|---|---|
| **Create** | `flox init` | Declarative environments that live alongside your code and activate automatically |
| **Search** | `flox search` | Find any package from 120,000+ in Nixpkgs instantly |
| **Share** | `flox push` / `flox pull` | Push to [FloxHub](https://hub.flox.dev); anyone on your team pulls an identical environment |
| **Containerize** | `flox containerize` | Generate OCI-compatible images directly from your Flox environment — no Dockerfile required |
| **Services** | `flox services start` | Run databases, queues, and background processes as part of your environment — they start on activate and stop on exit |
| **Configure** | `manifest.toml` | Define environment variables, shell hooks, and activation scripts declaratively alongside your code |
| **Editor support** | [Flox extension](https://marketplace.visualstudio.com/items?itemName=flox.flox) | Available for VS Code and any VS Code-compatible editor — manage environments without leaving your IDE |
| **AI-ready** | Works with Claude Code, Cursor, Copilot, and Codex | Prompt any AI coding assistant to create or modify a Flox environment and it Just Works — see [flox-agentic](https://github.com/flox/flox-agentic) |

> **Origin:** Flox emerged from one of the largest enterprise Nix deployments in the world, at the D.E. Shaw group, where it made Nix accessible at scale across a large engineering organization.

### [Already using Nix?](https://flox.dev/docs/install-flox/install/#__tabbed_1_5)

Flox is a higher-level interface that adds environment sharing, activation hooks, and FloxHub without changing how you use nixpkgs. **Not using Nix?** You don't need to learn it — Flox handles everything.

---

## Community & Support

- [Documentation](https://flox.dev/docs) — tutorials, reference, and guides
- [FloxHub](https://hub.flox.dev) — discover and share environments
- [Discourse](https://discourse.flox.dev) — questions, discussions, and announcements
- [Blog](https://flox.dev/blog) — deep dives and workflows
- [Slack](https://go.flox.dev/slack) — real-time chat with the team
- [YouTube](https://www.youtube.com/@floxdev) — demos and walkthroughs
- [GitHub Issues](https://github.com/flox/flox/issues/new/choose) — bug reports and feature requests

**Questions? Want help getting started?** [Join us on Slack](https://go.flox.dev/slack) — we'll walk you through it.

## Security

We encourage responsible disclosure of potential security issues.
For any security-related inquiry, please contact us at: **security@flox.dev**

## Contributing

We welcome contributions! Please read the [Contributor guide](./CONTRIBUTING.md) first.

## Star History

⭐ If you find Flox useful, star the repo — it helps others discover the project and keeps us going.

<a href="https://star-history.com/#flox/flox&Date">
 <picture>
   <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/svg?repos=flox/flox&type=Date&theme=dark" />
   <source media="(prefers-color-scheme: light)" srcset="https://api.star-history.com/svg?repos=flox/flox&type=Date" />
   <img alt="Star History Chart" src="https://api.star-history.com/svg?repos=flox/flox&type=Date" width="600" />
 </picture>
</a>

## License

The Flox CLI is licensed under the GPLv2. See [LICENSE](./LICENSE).

<img referrerpolicy="no-referrer-when-downgrade" src="https://static.scarf.sh/a.png?x-pxid=199c01a0-67c0-4d95-a4c1-5c2d78aa7743" />

[website]: https://flox.dev
[discourse]: https://discourse.flox.dev
[nix]: https://nixos.org
[docs]: https://flox.dev/docs
[new-issue]: https://github.com/flox/flox/issues/new/choose
[post-nixpkgs]: https://flox.dev/blog/nixpkgs
