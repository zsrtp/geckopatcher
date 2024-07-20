# Web Patcher

This is the web version of the GeckoPatcher.

## How to compile

The web patcher uses [trunk](https://trunkrs.dev) to build itself. It can be installed from cargo with `cargo install trunk`.

## How to run

The web patcher needs some specific HTTP headers in order to work properly on the client browser.

- Cross-Origin-Resource-Policy:same-site
- Cross-Origin-Opener-Policy:same-origin
- Cross-Origin-Embedder-Policy:require-corp
- Control:no-store, no-cache, max-age=0, must-revalidate, proxy-revalidate
