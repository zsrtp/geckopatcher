# Web Patcher

The web version of GeckoPatcher. It is a self-contained static web application which
requires no server-side computation — only a static web server (or any HTTPS host) to
serve it. Users patch their game directly from the browser.

You can deploy it with your own mods by customizing the files in `assets/`:

- `config.toml` — the name, short name, and icon used for the Progressive Web
  Application (PWA) manifest generated when the project is compiled.
- `patches/mapping.json` — maps the first 8 bytes of a game dump (the game's
  code + region, e.g. `GZ2E01\0\0`) to a version string such as `gcn_ntscu` or
  `wii_ntscu_10`. The version strings encode the platform and region (GameCube
  NTSC-U, Wii NTSC-U 1.0, etc.) and match the per-version/region patch files
  produced by projects like [TPGZ](https://github.com/zsrtp/tpgz)
  (e.g. `1.0.0-gcn-ntscu.patch`).
- `patches/meta.json` — lists all the available patch files. Each entry contains the
  name of the patch (usually the mod version), the path to the `.patch` file, and the
  game version (from `patches/mapping.json`) it applies to. The patcher uses this to
  only show the patches that match the user's dump.
- An SVG file (e.g. `Gecko2.svg`) used to produce the icons.

## Compiling

The web patcher is built with [trunk](https://trunkrs.dev). Install it with
`cargo install trunk`, then build:

```sh
$ trunk build
```

This compiles and packages the web application into the `dist/` folder. Add `--release`
for a release build. A `pre_build` hook (configured in `Trunk.toml`) runs the `xtask`
Cargo package to generate the PWA manifest before each build.

By default the application ships with the patch files found in `assets/patches/`.
It can also be built with the `generic_patch` feature flag to produce a version which
accepts a user-provided patch file instead:

```sh
$ trunk build --features generic_patch
```

## Running

Once built, serve the `dist/` folder with any static server over a secure HTTPS
connection. The patcher requires a secure context.

For local development, `trunk serve` starts a dev server which rebuilds and reloads on
changes. The server is configured in `Trunk.toml` to serve on
`https://localho.st:8081` using the self-signed `key.pem` and `cert.pem` certificates
shipped in this directory, and to set the cross-origin isolation headers required by the
application (`Cross-Origin-Resource-Policy: same-site`,
`Cross-Origin-Opener-Policy: same-origin`, `Cross-Origin-Embedder-Policy: require-corp`).

```sh
$ trunk serve
```

There is also a `run_dev_server.sh` script which serves an existing `dist/` build with
the same cross-origin headers, which is useful for testing a production build locally.
