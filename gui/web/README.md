# Web Patcher

This is the web version of the GeckoPatcher.

## How to compile

The web patcher uses [trunk](https://trunkrs.dev) to build itself. It can be installed from cargo with `cargo install trunk`.
Once that is installed, you can build the project by running `trunk build`. This will compile and package the web application
to the `dist` folder. The option `--release` is available to build a release version.

By default, the application will look for patch files provided in the `patches` folder, but it can be built with the `generic_patch`
feature flag (`--feature generic_patch`) to have a version of the application which accepts a patch provided by the user.

## How to run

Once built, the application can be provided to uses by any server which supports both secure connection through SSL/TLS (HTTPS) and
providing specific headers.

The web patcher needs some specific HTTP headers in order to work properly on the client browser.

- Cross-Origin-Resource-Policy: same-site
- Cross-Origin-Opener-Policy: same-origin
- Cross-Origin-Embedder-Policy:r equire-corp
- Control: no-store, no-cache, max-age=0, must-revalidate, proxy-revalidate

The last header is optional and is only useful during development to avoid the client caching the application.
