# Web Patcher

This is the web version of the GeckoPatcher.

## How to compile

The web patcher uses [trunk](https://trunkrs.dev) to build itself. It can be installed from cargo with `cargo install trunk`.
Once that is installed, you can build the project by running `trunk build`. This will compile and package the web application
to the `dist` folder. The option `--release` is available to build a release version.

By default, the application will look for patch files provided in the `patches` folder, but it can be built with the `generic_patch`
feature flag (`--features generic_patch`) to have a version of the application which accepts a patch provided by the user.

## How to run

Once built, the application can be provided to users by any server which supports secure connection through SSL/TLS (HTTPS).

For testing, you can run `trunk serve` to launch a dev server and go to https://localho.st:8080/ to see the patcher run and
update on changes in real time. The server configuration can be changed in `Trunk.toml`.
