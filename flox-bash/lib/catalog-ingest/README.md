# Flox Catalog Data Ingestion

## Usage

To perform an analysis of a (*capacitated*^[Usage on other flakes to be evaluated]) flake run:

```
nix eval --override-input target <target flakeref> github:flox/catalog-ingest#analysis.<analysis>

<target flakeref>: location of the flake to analyze
<analysis>: <eval|build>.packages.<attrPath>


<eval>: get information about the package without building it
<build>: inspect the build output (to dermine the presence of `/local/{bin,man}` in the flake).

<attrPath>: equal to the `attrPath` used when acchessing packages of the target flake.
```

## Example

```
nix eval --json \
  --override-input target github:flox/floxpkgs \
  github:flox/catalog-ingest#analysis.eval.packages.x86_64-linux.default
```

<details>
<summary style="font-size:14px">Output</summary>
<p>

```json
{
  "flox": {
    "build": {},
    "element": {
      "active": true,
      "attrPath": [
        "x86_64-linux",
        "default",
        "flox"
      ],
      "originalUrl": null,
      "storePaths": [
        "/nix/store/ija147pgghwabsp2iz0l5lb8w581jy5v-flox-0.0.2-rdirty"
      ],
      "url": null
    },
    "eval": {
      "attrPath": [
        "x86_64-linux",
        "default",
        "flox"
      ],
      "drvPath": "/nix/store/qxk8ja9n502byr205wij6vr6m6hbdnvn-flox-0.0.2-rdirty.drv",
      "flake": {
        "locked": {
          "lastModified": 1657728995,
          "lastModifiedDate": "20220713161635",
          "narHash": "sha256-SNZn1JBP32CJB6Yetf/MUh7MU3tiLS1/0Gs30oR7e8k=",
          "rev": "d11320fb8efb964bde06012bf8c33110b76dd777",
          "shortRev": "d11320f"
        }
      },
      "meta": {
        "available": true,
        "broken": false,
        "insecure": false,
        "name": "flox-0.0.2-rdirty",
        "outputsToInstall": [
          "out"
        ],
        "position": "/nix/store/jvdiiqzmhy6d5m0yq7irbfjfrxjqhfk2-source/default.nix:66",
        "unfree": false,
        "unsupported": false
      },
      "name": "flox-0.0.2-rdirty",
      "outputs": {
        "out": "/nix/store/ija147pgghwabsp2iz0l5lb8w581jy5v-flox-0.0.2-rdirty"
      },
      "pname": "flox",
      "system": "x86_64-linux",
      "version": "0.0.2-rdirty"
    }
  }
}
```
</p></details>
