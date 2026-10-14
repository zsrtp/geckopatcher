# GeckoPatcher

GeckoPatcher is a modern rebuild of the [romhack-compiler](https://github.com/zsrtp/romhack-compiler)
project. The previous project had accumulated issues which made it hard to maintain;
this project aims to remedy that situation.

It patches and creates patch files for **GameCube** and **Wii** game dumps.

## Summary

The project is split into one library and three applications:

- **`geckolib`**: the core library containing the business logic (file loading/saving,
  encryption/decryption, patching, assembly, linking).
- **`romhack`**: the command line application used to patch game backups and to create
  new patch files. This is the only application meant to *create* patches.
- **`gui/native`**: the desktop UI application (`gui-patcher`), the counterpart to the
  `romhack` CLI, which runs natively on the user's computer.
- **`gui/web`**: the UI application that runs in a web browser.

The GUI applications are meant only as a way to apply existing patch files to game
backups; only `romhack` can create new patches.

## CLI

```
romhack new <name>                  Create a new romhack project (scaffold a Rust crate)
romhack build [--patch]             Build the project described by RomHack.toml in the
                                    current directory (or package it into a .patch file
                                    with --patch)
romhack apply <patch> <iso> <out>   Apply a .patch file to a game dump
romhack diff <orig> <hacked> <out>  Create a patch from a diff of two game dumps
```

All commands accept `-P` (`--no-progress`) to hide the progress bar.

## Creating a patch

There are two ways to create a patch file:

1. **Project-based (recommended)**: describe your modifications in a `RomHack.toml`
   file, and let `romhack` build the patch from them.
2. **Diff-based**: compare a modified game dump against the original unmodified dump
   and let `romhack` figure out the differences.

### Project-based patches

The easiest way to start is to scaffold a new project:

```sh
$ romhack new my-mod
```

This creates a Cargo project (`my-mod/`) containing a `RomHack.toml`, a `#![no_std]`
Rust library (`src/lib.rs`), a panic handler, and a `src/patch.asm` file. The generated
`RomHack.toml` looks like this:

```toml
[info]
game-name = "my_mod"

[src]
iso = "game.iso" # Provide the path of the game's ISO
patch = "src/patch.asm"
# Optionally specify the game's symbol map
# map = "maps/framework.map"

[files]
# You may replace or add new files to the game here
# "path/to/file/in/iso" = "path/to/file/on/harddrive"

[build]
map = "target/framework.map"
iso = "target/my_mod.iso"

[link]
entries = ["init"] # Enter the exported function names here
base = "0x8040_1000" # Enter the start address of the Rom Hack's code here
```

To build the patched ISO from your project directory:

```sh
$ romhack build
```

To package the project into a distributable `.patch` file instead:

```sh
$ romhack build --patch
```

> The `--raw` flag on `build` is kept only for backwards compatibility and is ignored.

A `RomHack.toml` project does not have to be a Rust project. The scaffold from
`romhack new` is only a convenience for Rust users: everything the patcher needs is a
directory containing a `RomHack.toml`, a `patch.asm` and the other files they reference.
The [TPGZ](https://github.com/zsrtp/tpgz) project is a real example — it is written in
C++ (built with devkitPro), generates its `RomHack.toml` and `patch.asm` from CMake
templates into a `build/` directory, and drives `romhack` from there. See
[Injecting compiled code](#injecting-compiled-code-the-non-rust-way) below.

### The patch file format

A patch file is a **zip archive** containing a `RomHack.toml` manifest plus the data to
be injected into the game. It is the format distributed to end users, who apply it with
`romhack apply`, `gui-patcher`, or the web patcher.

### `RomHack.toml` reference

The manifest supports the following sections:

#### `[info]`

Optional metadata used for the game's banner and the web patcher's display:

| Key                   | Type   | Description                                    |
| --------------------- | ------ | ---------------------------------------------- |
| `game-name`           | string | Short game name (used in the banner)           |
| `full-game-name`      | string | Full display name                              |
| `developer-name`      | string | Short developer name                           |
| `full-developer-name` | string | Full developer name                            |
| `description`         | string | Patch description                              |
| `image`               | path   | Path to a banner image (stored as `banner.dat`)|
| `version`             | semver | Patch version                                  |

#### `[src]`

Describes the source the patch is built from:

| Key     | Type   | Description                                                      |
| ------- | ------ | ---------------------------------------------------------------- |
| `iso`   | path   | Path to the original, unmodified game ISO (required)             |
| `patch` | path   | Path to the assembly patch file (optional)                       |
| `map`   | path   | Path, inside the game, to the game's symbol map (e.g. `framework.map`), used to resolve game symbols. This differs per game version/region, so a project often ships one per target |

#### `[files]`

Maps files *inside the game* to files *on disk* (relative to the project). The file is
replaced if it exists in the game, and added otherwise:

```toml
[files]
"res/obj/someFileInGame.arc" = "someFileInPatch.ext"
"my_mod/a_texture.tex" = "res/someOtherFileInPatch.tex"
```

Directories may be used on the right hand side to add an entire tree. This is how
non-code assets are injected: textures, fonts, save files, and even runtime-loadable
REL modules (TPGZ, for example, injects its compiled RELs into `tpgz/rels` and lets the
game's loader pull them from the filesystem at runtime).

#### `[build]`

| Key   | Type   | Description                                        |
| ----- | ------ | -------------------------------------------------- |
| `iso` | path   | Path where the built ISO is written (required)     |
| `map` | path   | Optional path where the symbol map of the linked code is written          |

#### `[link]`

Embeds code compiled from Rust directly into the game's main DOL. This is the *Rust*
workflow — if your project is not written in Rust, see
[Injecting compiled code](#injecting-compiled-code-the-non-rust-way) below:

| Key       | Type       | Description                                                       |
| --------- | ---------- | ----------------------------------------------------------------- |
| `entries` | `[string]` | Global symbols (exported functions) to resolve and link in       |
| `base`    | string     | Start address of the romhack's code (e.g. `"0x8040_1000"`)       |
| `libs`    | `[path]`   | Static libraries (`*.a`) to link (see below)                     |

The romhack code is written as a `#![no_std]` Rust static library
(`crate-type = ["staticlib"]`), with `#[no_mangle] pub extern "C"` exported functions
that match the `entries`. It must be compiled for the PowerPC architecture, since the
linker resolves PowerPC relocations. List the resulting archive under `libs`; when
packaging with `romhack build --patch` it is stored in the patch as `libcompiled.a`.
The linker also links in a built-in basic library (`libbasic.a`) providing common
runtime symbols.

Game symbols referenced by your code can be resolved through the game's symbol map
(`[src] map`).

### Injecting compiled code (the non-Rust way)

Rust is not required to add code to the game. Because a patch is just a zip containing
`RomHack.toml`, a `patch.asm` and some files, any language that can be cross-compiled
to PowerPC works. The general approach — used by the TPGZ project — is:

1. Compile your code into a raw PowerPC binary blob (TPGZ compiles its C++ with
   devkitPro into a `res/bin/<platform>_<region>.bin`).
2. Convert the blob into one `u32 0x...` line per word and paste those lines into
   `patch.asm` under the address where the code should live. (TPGZ does this
   conversion in CMake, reading the file as hex and turning every 8 hex digits into a
   `u32 0x...` line.)
3. Patch the game to run it: overwrite a hook with a `u32` branch instruction that
   jumps into your code, and write `u32 0x60000000` (a nop) over any functions you
   want to disable.
4. Resolve game symbols through `[src] map`, and inject non-code assets (textures,
   fonts, save files, REL modules) through `[files]`.

A realistic example, taken from the generated `patch.asm` of a TPGZ GameCube NTSC-U
build:

```asm
; Injected loader code: the compiled C++ blob, one u32 per instruction
0x80004D18:
u32 0x3fe08000
u32 0x3bc00000
; ... (many more words, ending with the runtime REL path as string data)
u32 0x2f747067 ; "/tpg"
u32 0x7a2f7265 ; "z/re"
u32 0x6c732f62 ; "ls/b"
u32 0x6f6f742e ; "oot."
u32 0x72656c00 ; "rel\0"

; Hook: jump from the game's code into the injected loader
0x800063f8:
u32 0x4Bffe920

; Disable a buggy function (a nop)
0x80238F58:
u32 0x60000000

; Patch three instructions of setWaterDropEffect for custom tunic colors
0x801244a4:
u32 0x60000000
0x801244a8:
u32 0x60000000
0x801244ac:
u32 0x60000000

; Enable the debug crash screen
0x8000b8a4:
u32 0x48000014
```

Note how the string `"/tpgz/rels/boot.rel"` travels along with the loader code: the
loader resolves that path at runtime to load a REL module, which `romhack` merely
placed in the game's filesystem via `[files]`.

#### `[diffs]`

This section is normally **generated by `romhack diff`** (see below). It stores binary
changes as bzip2-compressed bsdiff patches:

| Key         | Type                    | Description                                                              |
| ----------- | ----------------------- | ------------------------------------------------------------------------ |
| `deletions` | `[path]`                | Paths in the game's filesystem to remove                                  |
| `changes`   | `{path: path}`          | Paths in the game to modify, mapped to `.bs.gz2` files in the archive    |
| `dol`       | path                    | bsdiff file for the main DOL (`start_dol.bs.gz2`)                        |
| `loader`    | path                    | bsdiff file for the AppLoader (`apploader.bs.gz2`)                       |
| `checksum`  | string (hex)            | Cumulative SHA3-256 checksum of the original game's files                 |

### Assembly patches

The `patch` file referenced by `[src] patch` is a simple assembly-like format applied
to the game's main DOL. It supports the following statements:

| Statement                  | Description                                   |
| -------------------------- | --------------------------------------------- |
| `0x80000000:`              | Set the current address for the following lines |
| `u32 0x12345678`           | Write a 32-bit value at the current address   |
| `bl 0x80012345` / `bl func`| Branch-and-link to an address or symbol       |
| `b 0x80012345` / `b func`  | Branch to an address or symbol                |
| `lis r3, 0x8000`           | Load immediate shifted into a register        |
| `nop`                      | No-op (0x60000000)                            |

Symbols are resolved from the linked romhack code (`[link]`) and from the game's
symbol map (`[src] map`). For example:

```asm
; patch.asm
0x80000000:
u32 0x12345678
u32 0x9ABCDEF0
```

replaces the two 32-bit words at `0x80000000` in the main DOL with the values
`0x12345678` and `0x9ABCDEF0`.

### Diff-based patches

The simplest way to make a patch is to compare a hacked version of a game, produced by
any means you like, with the original unmodified dump:

```sh
$ romhack diff ORIGINAL.iso HACKED.iso OUTPUT.patch
```

This compares the unmodified dump (`ORIGINAL.iso`) with the romhack you wish to share
(`HACKED.iso`) and produces a `OUTPUT.patch` file, which can be distributed for others
to apply to their own dump of the game.

## Applying a patch

To apply a `.patch` file to your own game dump:

```sh
$ romhack apply PATCH.patch ORIGINAL.iso OUTPUT.iso
```

Alternatively use the native GUI (`gui-patcher`) or the [web patcher](gui/web/README.md).

### Why not just share diffs?

Because of the complexity of the GameCube and Wii disc formats (compared to older
platforms like SNES or N64), sharing mods as whole-file diffs would be inefficient.
A GameCube or Wii game is essentially an archive: a header with metadata plus a
filesystem (Wii adds an extra layer of abstraction, with the filesystem contained
inside an encrypted partition). The patcher extracts the filesystem, applies the
modifications to the relevant files, and writes the filesystem back into a new dump —
usually without the padding that came from the original disc. This is why a patched
dump may be slightly smaller than the original, while still running correctly on an
emulator or a physical console.

## Building

GeckoPatcher can be built natively for all major platforms (Windows, macOS, Linux),
as well as a web application for applying patches in the browser. See the
[web Readme](gui/web/README.md) for the web build.

### Native

The only requirement to compile the native CLI and GUI is the
[Rust toolchain](https://www.rust-lang.org/). Then run:

```sh
$ cargo build
```

Add `--release` to build the release version. The binaries are written to
`target/debug/` (or `target/release/` for release builds) under the names `romhack`
(CLI) and `gui-patcher` (GUI) — `romhack.exe` and `gui-patcher.exe` on Windows.

> A Nix flake (`flake.nix`) is also provided, which gives a `nix develop` shell with
> the required toolchain (including `trunk` and `nodejs` for the web build).

#### Bundling

It is possible to produce OS-specific bundles (macOS apps, Linux AppImages, Windows
MSIs) using `cargo-bundle`. Install it with `cargo install cargo-bundle`, then build a
bundle for `gui-patcher`:

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

See [gui/web/README.md](gui/web/README.md).

## License

Licensed under the [MIT license](LICENSE).
