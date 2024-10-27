# Web Patcher

This is the web version of the GeckoPatcher.

This can be used to deploy your own mods for users to patch their game directly from their browser. It's a self contained web application which doesn't require any fancy server other than a static web server (no server computation required).

You can customize the patcher to use for your application by modifying in `assets/` the files `config.toml`, `patches/meta.json`, and `patches/mapping.json`, as well as providing your own SVG file to produce the icons.

- `config.toml` contains only the name and identifier for the Progressive Web Application (PWA) manifest file that will be generated when compiling the project.
- `patches/mapping.json` contains the mapping from the first 8 bytes of the game's dump to a version string used in `patches/meta.json` to filter the available patches to only show the user the ones corresponding to their dump.
- `patches/meta.json` contains all the available patch files produced from your mod. Each entry contains the name of the patch (usually the version of the mod), the path to the patch file, and the version of the game it can be applied to (from `patches/mapping.json`).

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
