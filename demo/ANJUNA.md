# Demo: `flox activate --sandbox` — the Anjuna Security (TEE) backend (prototype)

`cd` into a project and hand its baked environment to **Anjuna
Security's confidential-computing runtime**: Anjuna's
`anjuna-nitro-cli` *converts* a container image into an *enclave
image* (`.eif`) that runs hardware-isolated inside a Trusted
Execution Environment — AWS Nitro Enclaves, AMD SEV-SNP, or Intel
SGX. flox bakes the reproducible closure into an image, generates
the enclave-converter config + the `build-enclave` invocation, and
binds the *expected enclave attestation measurement* to the
lockfile hash. So a relying party can prove the enclave that runs is
the one flox's environment produced. This is the co-sell shape from
the Anjuna conversation (#flox-external-anjuna-2025, Stahnke notes
2025-09-25, Dor's floxification follow-up): **Flox is the
reproducible environment layer beneath Anjuna's confidential
runtime.**

**Bold** lines are what to *say*; fenced blocks are what to *type*.
The OCI-backend walkthrough is `demo/SCRIPT.md`; the OpenShell one
is `demo/OPENSHELL.md`; the Devin one is `demo/DEVIN.md`. They all
share `demo/setup.sh` and `demo/cleanup.sh`.

**The pitch:** flox already bakes each environment into an OCI
image. Anjuna does not *run* that image — it *converts* it into an
enclave image that runs inside a hardware TEE. So flox hands Anjuna
the baked image plus the converter config, and binds the enclave's
attestation measurement to the lockfile hash: flox brings the
reproducible, attestable environment; Anjuna brings the
hardware-isolated confidential runtime. Same manifest, one word
changed: `backend = "anjuna"`.

> **Honesty up front — this backend is Scaffolded, not
> Implemented.** Anjuna is a **commercially licensed** product: the
> `anjuna-nitro-cli` and runtime are **not open source** and are
> distributed through Anjuna's private repository, not a public
> download or the Flox catalog. And a TEE needs **hardware** — SGX/
> SEV-SNP silicon or an AWS Nitro parent instance, i.e. a Linux
> cloud instance. **macOS arm64 has neither the CLI nor the TEE
> capability.** So flox runs the honest *local* slice — preflight,
> bake, policy compilation, converter-config + build-invocation +
> attestation-binding generation — and stops at the launch boundary
> with a clear error naming both walls. Beats 2–7 describe what a
> completed enclave build + run looks like on a licensed,
> TEE-capable instance.

---

## 0 · Setup

### One-time host prerequisites

1. **Docker Desktop** (or Docker Engine ≥ 28) running — the image
   is baked into the local Docker store as the converter's input.
   This is the one genuinely required host tool.
2. **The presentation shell** — one command, in your presentation
   shell (export `FLOX_BIN` from the dev shell first):

   ```bash
   flox activate -r djsauble/anjuna-setup
   ```

   This is the demo's *outer layer* — one setup env per sandbox
   backend is the plan. Like the Devin env, it runs **no local
   service** and installs **no provider CLI**: Anjuna is
   license-gated and its CLI is presence-detected, not required. It
   configures the shell (feature flags and the planted
   `GITHUB_TOKEN` in `[vars]`, `FLOX_VERSION` plus a `flox` alias
   from `$FLOX_BIN` in `[profile]`), plants the `~/demo-secrets`
   fixture, and prints a note for each hand-off prerequisite that is
   missing. Deactivating removes the planted secret
   (`[profile.deactivate]`). Stay in this activation for the whole
   demo.

   > Details, caveats, and troubleshooting:
   > `demo/anjuna-setup/README.md`.

3. **An Anjuna commercial license** (**account beat** — required
   for the enclave build, beats 2+). The `anjuna-nitro-cli` and
   runtime are not open source; a warm partnership contact exists
   (#flox-external-anjuna-2025). A co-sell with Anjuna is the path
   to a backend-grade integration.

4. **TEE hardware** (**hardware beat** — required for the enclave
   build + run). SGX/SEV-SNP silicon or an AWS Nitro parent
   instance — a Linux cloud instance. macOS arm64 has none.

5. **A registry the Anjuna converter can pull from** (**registry
   beat** — for the substrate image). Point flox at it so the
   generated `build-enclave` `--docker-uri` references the right
   ref:

   ```bash
   export FLOX_SANDBOX_ANJUNA_REGISTRY=docker.io/<your-user>
   ```

### Demo environment

Run once from the dev shell:

```bash
BACKEND=anjuna bash demo/setup.sh
```

Same demo env as the other walkthroughs (git, curl, which,
python3, `flox/claude-code`, an auto-starting web service, seeded
`app.py` / `index.html`); the manifest declares `backend = "anjuna"`
plus network grants for the agent's API endpoints:

```toml
[[options.sandbox.network]]
endpoint = "api.anthropic.com:443"
binary   = "claude-code/.claude-wrapped"
# plus an identical grant for statsig.anthropic.com (agent telemetry)
```

flox compiles the **host** of each `:443` grant into the enclave
config's `egress.allowed_hosts` allowlist. Egress from a Nitro
enclave traverses the parent instance's `anjuna-nitro-netd`
vsock↔network proxy, which allowlists **per-host, not per-port**, so
the `:443` is dropped for the hosts it allows; the `binary`,
`access`, and `protocol` fields are recorded as comments but **do
not** constrain traffic through the converter-config contract — a
declared lossiness, honest about what the hand-off can express. A
grant on any port other than 443 is rejected at compile time rather
than silently widened.

The setup env already configured your shell — make sure the prompt
hook is in your shell's RC:

```bash
eval "$(flox hook-env --shell bash --shell-pid $$)"
```

**Pre-bake off-camera.** The first bake takes ~5–15 min on a
machine that compiles the pinned flox rev in-VM, or ~2–5 min if the
pin is cached. Later bakes reuse layers:

```bash
cd ~/sandbox-demo && FLOX_SANDBOX_OCI_AUTOBAKE=true flox activate -- true
```

The image lands in Docker as `sandbox-demo-anjuna:<hash12>` (the
Anjuna backend reuses the openshell compat-layer bake),
content-addressed to the lockfile — it rebakes only when the
environment actually changes.

---

## 1 · Auto-activate toward an Anjuna enclave

**"One `cd`, one `Y`, and flox bakes the image, compiles the
policy, and generates the Anjuna converter config + the
`build-enclave` invocation the enclave image is built from — with
the attestation measurement bound to the lockfile hash."**

```bash
cd /tmp && cd ~/sandbox-demo
```

```
Enter '/Users/you/sandbox-demo' (sandboxed via anjuna)? [Y/n]
```

Type `Y`. flox baked (or reused) the image, then generated the
converter-config hand-off.

**Without a license, TEE hardware, or registry** (this host), flox
stops at the launch boundary and tells you precisely what is
missing:

```
The 'anjuna' sandbox backend converts the baked environment into an
Anjuna enclave image, which requires prerequisites this host cannot
satisfy automatically:
  1. Push the baked image 'sandbox-demo-anjuna:<hash12>' to a
     registry the Anjuna converter can pull (set
     FLOX_SANDBOX_ANJUNA_REGISTRY=<registry-prefix> and re-run, then
     push '<prefix>/sandbox-demo-anjuna:<hash12>').
  2. An Anjuna commercial license: no 'anjuna-nitro-cli' was found
     on PATH (it is commercially licensed), and the anjuna-nitro-cli
     and runtime are not open source — obtain them through Anjuna (a
     warm partnership contact exists; see the demo notes).
  3. TEE hardware: the enclave needs SGX/SEV-SNP silicon or an AWS
     Nitro parent instance. macOS arm64 has no such capability, so
     the build + run must happen on a cloud Linux instance.
flox generated the Anjuna converter config + build invocation at:
  /Users/you/sandbox-demo/.flox/cache/anjuna/enclave-config.yaml
  /Users/you/sandbox-demo/.flox/cache/anjuna/build-enclave.sh
On a licensed, TEE-capable instance, push the image, then run the
build script to convert the enclave image and record its attestation
measurement against the lockfile hash '<hash12>'.
```

**"That is not a failure — that is the honest edge of what a laptop
can do for a hardware-TEE runtime with no public launch API and no
TEE silicon. flox did everything local: baked the image, compiled
the deny-by-default policy, and wrote the exact converter config +
build invocation Anjuna turns into an enclave image. The three
missing pieces — license, hardware, registry — are the co-sell
conversation, not flox's."**

Look at what flox generated:

```bash
cat ~/sandbox-demo/.flox/cache/anjuna/enclave-config.yaml
```

```yaml
# Generated by `flox activate --sandbox --sandbox-backend anjuna`.
# ... converter contract: Anjuna CONVERTS the image via build-enclave
# into an enclave image (.eif) ...
#
# ATTESTATION BINDING. The enclave's identity is its measurement
# (Nitro PCR0/PCR1/PCR2, or the SEV-SNP/SGX equivalent) ...
#   flox lockfile hash: <hash12>
#   policy: allowed: api.anthropic.com, statsig.anthropic.com
egress:
  deny_all_egress: false
  allowed_hosts: ["api.anthropic.com", "statsig.anthropic.com"]
```

```bash
cat ~/sandbox-demo/.flox/cache/anjuna/build-enclave.sh
```

```bash
#!/usr/bin/env bash
# ... run on a TEE-capable Linux instance with a licensed anjuna-nitro-cli ...
# ATTESTATION BINDING
#   flox lockfile hash: <hash12>
anjuna-nitro-cli build-enclave \
  --docker-uri "$IMAGE_URI" \
  --enclave-config-file "$CONFIG_FILE" \
  --output-file "$OUTPUT_EIF"
# ... then: anjuna-nitro-cli run-enclave --eif-path "$OUTPUT_EIF"
```

**"The manifest's `:443` grants became the enclave config's
`allowed_hosts` allowlist, deny-by-default. And the load-bearing
piece is the attestation binding: flox records the lockfile hash
next to the measurement `build-enclave` will emit. That is the
whole confidential-computing story — a relying party verifies the
enclave's measurement, and flox proves that measurement corresponds
to a specific, reproducible flox environment."**

---

## 2 · Push the image and build the enclave (license + hardware + registry beat)

**"With an Anjuna license, a TEE-capable instance, and a registry,
this is the whole remaining path."** On a credentialed operator's
Nitro-capable Linux instance, with `FLOX_SANDBOX_ANJUNA_REGISTRY`
set:

```bash
# Tag the local bake as the anjuna-namespaced registry ref and push:
docker tag sandbox-demo-anjuna:<hash12> \
  "$FLOX_SANDBOX_ANJUNA_REGISTRY/sandbox-demo-anjuna:<hash12>"
docker push \
  "$FLOX_SANDBOX_ANJUNA_REGISTRY/sandbox-demo-anjuna:<hash12>"

# Convert the image into an enclave image and capture the measurement:
bash ~/sandbox-demo/.flox/cache/anjuna/build-enclave.sh
```

`build-enclave` converts the container image into a `.eif` enclave
image and prints its PCR measurement. Record that measurement
alongside the lockfile hash the config carries — that is the
attestation binding an operator wires into their verification
policy.

> This beat requires a live Anjuna license, TEE hardware, and a
> reachable registry, none of which this host has tonight. The
> generated config + build invocation are exactly what Anjuna's
> converter consumes; nothing is faked.

---

## 3 · The enclave boots with the locked toolchain — and it's attestable

**"The whole pitch: the toolchain is present in the enclave, and the
enclave is *attestable*. No `apt install`, no version drift — the
image is the locked closure, content-addressed to the lockfile, and
its enclave measurement is bound to that same hash."**

Inside a running enclave (license + hardware beat), the locked tools
are already there — they were baked into the image the converter
consumed:

```bash
which python3 curl git       # all present, at the locked versions
flox list                    # the baked closure, exactly as declared
```

And the attestation report proves it: the enclave's measurement
matches the one bound to the lockfile hash. **"That binding is what
a TEE adds over every other backend on the roster — not just
isolation, but *provable* isolation of a *known* environment."**

---

## 4 · Prove the boundary — hardware isolation + deny-by-default egress

**"A TEE is the strongest boundary on the roster: the enclave's
memory is encrypted even against the host it runs on. And egress is
deny-by-default — only the manifest's `:443` hosts are in the
`allowed_hosts` allowlist flox compiled, which the parent's
anjuna-nitro-netd proxy enforces."**

Inside the enclave (license + hardware beat), a granted endpoint
works:

```bash
curl -sS https://api.anthropic.com/  # allowed: in allowed_hosts
```

An ungranted endpoint is blocked by the netd proxy:

```bash
curl -sS https://api.github.com/zen
# blocked — api.github.com is not in allowed_hosts
```

**"Anjuna's runtime governs egress; flox authored the allowlist from
the environment's own declared network needs. The threat model
inverts *further* than the other cloud backends: not only is the
host filesystem unreachable from the enclave, the enclave is
confidential *against the host operator itself*. But the code and
any injected secrets run in the enclave, so credentials belong in
Anjuna's attested secret-provisioning flow — keyed to the
attestation report — not your laptop's `.env`. That is the honest
tradeoff of a confidential runtime, and flox states it in the
backend capabilities."**

---

## 5 · Policy is fixed at enclave build — redemption is rebuild

**"Anjuna fixes the enclave's policy when `build-enclave` produces
the `.eif`. There is no live 'ask' — to widen egress, you edit the
manifest, regenerate the config, and rebuild the enclave image."**

Grant a new domain by editing the manifest and re-activating:

```toml
[[options.sandbox.network]]
endpoint = "api.github.com:443"
```

```bash
flox edit                       # add the grant
flox deactivate && cd ~/sandbox-demo   # regenerate the config
```

flox recompiles the allowlist and rewrites
`.flox/cache/anjuna/enclave-config.yaml`; rerunning `build-enclave`
(with license + hardware) yields an enclave image that allows
`api.github.com` — and a *new* measurement, which rebinds to the new
lockfile hash. **"Rebuild-as-redemption — the common path for
policy-at-build sandbox providers, and doubly honest here: a new
policy means a new enclave measurement, which is exactly what
attestation should reflect."**

---

## 6 · Run a coding agent, at full autonomy (license + hardware beat)

**"A coding agent with no permission prompts, running inside a
hardware enclave — the sandbox, not the agent, is the boundary, and
the boundary is a confidential runtime with its own egress proxy."**

The manifest already grants the agent's Anthropic endpoints, so
inside a running enclave:

```bash
claude --permission-mode auto
```

```
> add a docstring to greet() in app.py and commit the change
```

Claude's API traffic to `api.anthropic.com` is allowed; anything it
reaches for outside the allowlist is blocked by the netd proxy.
Because the runtime is a confidential enclave, the blast radius of
anything the agent does is a hardware-isolated, attestable sandbox.

> Agent auth (`CLAUDE_CODE_OAUTH_TOKEN`) must be injected through
> Anjuna's own attested secret-provisioning mechanism (keyed to the
> enclave's attestation report), not your laptop's `.env` — the
> enclave has no access to it, and that is the point. This is the
> credential-leaves-the-laptop tradeoff the inverted, confidential
> threat model names.

---

## 7 · Exit — the runtime is a hardware enclave

With license + hardware, the enclave is terminated through
`anjuna-nitro-cli terminate-enclave` (or the parent instance is torn
down) when you are done; nothing ran on your laptop.

On this host tonight, there is nothing to tear down — no enclave was
built and no session opened. The only local artifacts are the baked
image and the generated `.flox/cache/anjuna/` config + build script,
both removed by cleanup.

---

## 8 · Reset

```bash
bash demo/cleanup.sh
```

Removes the demo env, fixtures, the generated `.flox/cache/anjuna/`
artifacts, and the Docker-side `sandbox-demo-anjuna:*` images. (Any
enclave images or running enclaves on your TEE instance are governed
through Anjuna's tooling and are yours to terminate; images pushed to
your registry are yours to prune.)

> Integration notes for the Anjuna conversation (the converter — not
> image-launch — hand-off, the `allowed_hosts` netd-proxy egress
> vocabulary, the attestation-binding axis new to the seam, the
> license + hardware walls, the inverted confidential threat model):
> the backend module docs at
> `cli/flox/src/commands/sandbox_backends/anjuna.rs` and the backend
> contract at
> `slices/2026/06-sandboxed-activation-prototype/artifacts/backend-contract.md`.
