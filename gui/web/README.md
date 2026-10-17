# Web Patcher

The web version of GeckoPatcher. It is a self-contained static web application which
requires no server-side computation — only a static web server (or any HTTPS host) to
serve it. Users patch their game directly from the browser.

The user interface is written in plain HTML, CSS and JavaScript; the only Rust is a
single WebAssembly module (a Web Worker) which does the patching. This keeps the build
simple: one `trunk build` produces everything.

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
$ trunk build --release
```

This compiles and packages the web application into the `dist/` folder. A `pre_build`
hook (configured in `Trunk.toml`) runs the `xtask` Cargo package to generate the PWA
manifest and icons before each build.

The build is single-threaded and requires no special flags or configuration.

## Generic mode

If the `patches/` directory is omitted from a deployment, the application detects that
`patches/meta.json` is missing and switches to *generic* mode automatically: instead of
a drop-down of bundled patches, the user picks a `.patch` file from their computer. No
separate build is needed for this — the same `dist/` output serves both modes.

## Running

Once built, serve the `dist/` folder with any static server over a secure HTTPS
connection. The patcher requires a secure context, cross-origin isolation, and the
[File System Access API](https://developer.mozilla.org/en-US/docs/Web/API/File_System_Access_API)
(used to stream the multi-gigabyte game dump without loading it into memory).

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
