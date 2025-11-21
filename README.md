# Gecko Patcher

This project is a rebuild of the [romhack-compiler](https://github.com/zsrtp/romhack-compiler) project. The previous project has accumulated issues which made it hard to maintain. This project aims to remedy this situation.

## Summary

This project contains a few executables and libraries:

- __`geckolib`__: The main library containing the business code of the project (file loading/saving, encryption/decryption, patching).
- __`gui/native`__: The ui application which is the counterpart to the commandline `romhack` that runs natively on the user's computer.
- __`gui/web`__: The ui application that runs on a web browser.
- __`romhack`__: The commandline application which is used to patch game backups, and create new patch files.

The patcher is the only application which is meant to create new patches. The GUI interfaces are meant only as a way to use existing patch files to patch game backups.

## Usage

There are two ways to to make a patch file: a **diff** between an unmodified game and a romhacked version, or through the **legacy** method.

### Diff

The simplest way to make a patch file is to compare a Hacked version of a game you make through any mean you want with the original, unmodifed dump of the game. The command is simple:

```sh
$ romhack diff ORIGINAL.iso HACKED.iso OUTPUT.patch
```

This command will compare the unmodified dump of the game (`ORIGINAL.iso`) with the romhack you wish to share (`HACKED.iso`) and produce a patch file `OUTPUT.patch` which can be distributed for others to apply to their own dump of their game.

### Legacy

This goal of this tool is to allow modification of GameCube and Wii games. This is accomplished through the mean of a patch file. Due to the complexity of the format of both GameCube and Wii games (compared to older platforms like snes or N64), it would be less efficient to share mods as diffs of the original and modded version of the game. Instead GeckoPatcher uses a custom patch format which is a simple zip file containing metadata files about the mod, as well as the data to be injected into the game.

A minimal Patch file (zip archive) would contain a single `RomHack.toml` file with the following content:

```Toml
[src]
iso = ""

[build]
iso = ""
```

This valid patch file would result in nothing being modified in the game. Although, the file that the patcher would output could be different when comparing it to the original file byte to bytes, even though it could be run by an emulator or a physical console. Wii game would typically see the size of the file be reduced a bit. This is due to the fact that GameCube and Wii games are essentially equivalent to an archive containing a header with metadata and a FileSystem (with Wii having an additional layer of abstraction with the filesystem being contained inside a partition). The patcher extracts the FileSystem to then apply the modification to the relevant files, and finally put the FileSystem back in a new dump file, usually without the padding that usually comes from the disc on which the game was stored.

A more useful patch file would be a zip archive with the following structure:

```
|
+- RomHack.toml
+- patch.asm
```

With the files content being like so:

```Toml
# RomHack.toml
[info]
game-name = "MyMod"
full-game-name = "My Awesome Mod"
developer-name = "The Author"
full-developer-name = "John Doe, The Author"

[src]
iso = ""
patch = "patch.asm"

[build]
iso = ""
```
```asm
; patch.asm
0x80000000:
u32 0x12345678
u32 0x9ABCDEF0
```

Which would replace the two 32 bits words at the address 0x80000000 with the values 0x12345678 and 0x9ABCDEF0 in the main DOL file of the game.

An other possible way to modify the game would be to replace some of the game's files:

```
|
+- RomHack.toml
+- someFileInPatch.ext
+- res
   |
   + someOtherFileInPatch.tex
```
```Toml
# RomHack.toml
[info]
game-name = "MyMod"
full-game-name = "My Awesome Mod"
developer-name = "The Author"
full-developer-name = "John Doe, The Author"

[src]
iso = ""

[build]
iso = ""

[files]
"res/obj/someFileInGame.arc" = "someFileInPatch.ext"
"my_mod/a_texture.tex" = "res/someOtherFileInPatch.tex"
```

This would replace an existing file from `res/obj/someFileInGame.arc` in the game with the file `someFileInPatch.ext` contained in the zip archive (the patch file), as well as adding a new file to the game under `my_mod/a_texture.tex`.

## Building

GeckoPatcher can be built natively for all major distributions (Windows, MacOS, Linux), as well as a web application for applying a patch file provided by the user, or ones provided for your own application. You can find more details in the [web Readme](gui/web/README.md).

### Native

The requirements to compile the native gui and cli are as follow:

- **Rust**: You need to have the [rust compiler toolchain](https://www.rust-lang.org/) installed.

That's all. You then just need to run `cargo build` at the root of the repository. You can optionally add the `--release` flag to build the release version.

This will result in the binary for the system you built it on to be put in `target/debug/` (for a release build, `target/release/`), under the names `romhack` and `gui-patcher` for the CLI and GUI verisons of the application respectively (`romhack.exe` and `gui-patcher.exe` for Windows).

#### Bundling

It is possible to produce OS-specific bundles (MacOS apps, Linux AppImage, Windows msi) using `cargo-bundle`. Install it through `cargo install cargo-bundle`, then use the following commands to produce a bundle for the `gui-patcher`:

```sh
$ cargo bundle -p gui-patcher --release --target x86_64-apple-darwin -f osx
```
```sh
$ cargo bundle -p gui-patcher --release --target x86_64-unknown-linux-gnu -f appimage
```
```sh
$ cargo bundle -p gui-patcher --release --target x86_64-pc-windows-gnu -f msi
```

### Web

See [gui/web/README.md](gui/web/README.md)