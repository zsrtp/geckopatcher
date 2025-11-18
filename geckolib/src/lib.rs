#[cfg(feature = "log")]
extern crate log;
#[cfg(feature = "parallel")]
extern crate rayon;
extern crate thiserror;
#[macro_use]
extern crate lazy_static;
extern crate cbc;
extern crate eyre;
extern crate num;
extern crate serde;
extern crate sha1_smol;
#[macro_use]
extern crate static_assertions;
extern crate indextree;
extern crate regex;
extern crate syn;

pub mod config;
pub mod crypto;
pub mod diff;
pub mod iso;
pub(crate) mod logs;
pub mod patch;
#[cfg(feature = "progress")]
pub mod update;
pub mod vfs;

#[cfg(not(target_arch = "wasm32"))]
use std::collections::HashMap;
#[cfg(not(target_arch = "wasm32"))]
use std::fs::{File, OpenOptions};
use std::io::Read;
#[cfg(not(target_arch = "wasm32"))]
use std::io::{Seek, Write};
#[cfg(not(target_arch = "wasm32"))]
use std::process::Command;

use config::Config;
#[cfg(not(target_arch = "wasm32"))]
use eyre::Context;
use futures::AsyncWrite;
use futures::{AsyncRead, AsyncSeek};
#[cfg(not(target_arch = "wasm32"))]
use indextree::NodeId;
use iso::builder::IsoBuilder;
use iso::read::DiscReader;
#[cfg(not(target_arch = "wasm32"))]
use std::path::PathBuf;
use vfs::tree::GeckoFS;
use zip::ZipArchive;

#[cfg(feature = "progress")]
lazy_static! {
    /// Progress updater
    pub static ref UPDATER: std::sync::Arc<std::sync::Mutex<update::Updater<eyre::Report, usize>>> =
        std::sync::Arc::new(std::sync::Mutex::new(update::Updater::default()));
}

pub fn parse_config<R: Read>(config_stream: R) -> eyre::Result<Config> {
    Ok(toml::from_str(&std::io::read_to_string(config_stream)?)?)
}

/// Open a config from a patch file
pub async fn open_config_from_patch<RConfig, RDisc, W>(
    patch_reader: RConfig,
    iso_reader: RDisc,
    writer: W,
) -> eyre::Result<IsoBuilder<RConfig, RDisc, W>>
where
    RConfig: std::io::Read + std::io::Seek,
    RDisc: AsyncRead + AsyncSeek + Clone + Unpin + 'static,
    W: AsyncWrite + Clone + Unpin,
{
    let mut zip: ZipArchive<RConfig> = ZipArchive::new(patch_reader)?;

    let mut config: Config = parse_config(zip.by_name("RomHack.toml")?)?;

    if let Some(link) = &mut config.link {
        link.libs.insert(0, "libcompiled.a".into());
    };

    let disc_reader = DiscReader::new(iso_reader).await?;
    let wii_disc = disc_reader.get_disc_info();
    Ok(IsoBuilder::new_with_zip(
        config,
        zip,
        GeckoFS::parse(disc_reader).await?,
        wii_disc,
        writer,
    ))
}

#[cfg(not(target_arch = "wasm32"))]
/// Open a config from a file on the FileSystem to return an IsoBuilder
pub async fn open_config_from_fs_iso<
    R: AsyncRead + AsyncSeek + Unpin + Clone + 'static,
    W: AsyncWrite,
>(
    config: Config,
    input: R,
    output: W,
) -> eyre::Result<IsoBuilder<File, R, W>> {
    #[cfg(feature = "progress")]
    if let Ok(mut updater) = UPDATER.lock() {
        updater.set_message("Parsing RomHack.toml...")?;
    }

    let disc_reader = DiscReader::new(input).await?;
    let wii_disc = disc_reader.get_disc_info();
    let gfs = GeckoFS::parse(disc_reader).await?;
    Ok(IsoBuilder::new_with_fs(
        config,
        PathBuf::new(),
        gfs,
        wii_disc,
        output,
    ))
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug)]
struct Changes {
    additions: Vec<PathBuf>,
    deletions: Vec<PathBuf>,
    changes: HashMap<PathBuf, Vec<u8>>,
}

#[cfg(not(target_arch = "wasm32"))]
async fn extract_changes<R, R2>(
    original: &mut GeckoFS<R>,
    original_root: NodeId,
    patched: &mut GeckoFS<R2>,
    patched_root: NodeId,
) -> Result<Changes, eyre::Report>
where
    R: AsyncRead + AsyncSeek + Unpin,
    R2: AsyncRead + AsyncSeek + Unpin,
{
    let mut orig_files: Vec<_> = original
        .iter_path_dfs(original_root)
        .filter(|p| {
            original
                .get_node_ref(original_root, p)
                .ok()
                .is_some_and(|node| node.is_file())
        })
        .collect();
    let mut patch_files: Vec<_> = patched
        .iter_path_dfs(patched_root)
        .filter(|p| {
            patched
                .get_node_ref(patched_root, p)
                .ok()
                .is_some_and(|node| node.is_file())
        })
        .collect();
    orig_files.sort();
    patch_files.sort();

    let mut deletions: Vec<PathBuf> = Vec::new();
    let mut changes: HashMap<PathBuf, Vec<u8>> = HashMap::new();

    for orig_path in orig_files.iter() {
        if patch_files.contains(orig_path) {
            #[cfg(feature = "progress")]
            if let Ok(mut updater) = UPDATER.try_lock() {
                updater.set_message(format!(
                    "Checking {}",
                    orig_path
                        .iter()
                        .next_back()
                        .and_then(|s| s.to_str())
                        .unwrap_or("<unk>")
                ))?;
            }
            // File from original found in patched, check if content differs
            use futures::{AsyncReadExt, AsyncSeekExt};
            let mut orig_buf = Vec::new();
            let orig_file = original
                .get_file_mut(original_root, orig_path)
                .expect("path is a file");
            orig_file.seek(std::io::SeekFrom::Start(0)).await?;
            orig_file.read_to_end(&mut orig_buf).await?;
            let mut patch_buf = Vec::new();
            let patch_file = patched
                .get_file_mut(patched_root, orig_path)
                .expect("path is a file");
            patch_file.seek(std::io::SeekFrom::Start(0)).await?;
            patch_file.read_to_end(&mut patch_buf).await?;
            let diff = diff::diff(orig_buf, patch_buf)?;
            if let Some(diff) = diff {
                // There is a difference between the files, save it in the changes.
                changes.insert(orig_path.clone(), diff);
            }
        } else {
            // File from original not found in patched, add to delete list
            deletions.push(orig_path.clone());
        }
    }

    let mut additions: Vec<PathBuf> = Vec::new();

    for patch_path in patch_files
        .iter()
        .filter(|path| !orig_files.contains(path))
        .cloned()
    {
        #[cfg(feature = "progress")]
        if let Ok(mut updater) = UPDATER.try_lock() {
            updater.set_message(format!(
                "Adding {}",
                patch_path
                    .iter()
                    .next_back()
                    .and_then(|s| s.to_str())
                    .unwrap_or("<unk>")
            ))?;
        }
        // The file isn't in original, include it in the the additions
        additions.push(patch_path);
    }

    Ok(Changes {
        additions,
        deletions,
        changes,
    })
}

#[cfg(not(target_arch = "wasm32"))]
pub async fn generate_from_diff<R, R2, RConfig>(
    mut original: GeckoFS<R>,
    mut patched: GeckoFS<R2>,
    output: RConfig,
) -> Result<(), eyre::Report>
where
    R: AsyncRead + AsyncSeek + Unpin,
    R2: AsyncRead + AsyncSeek + Unpin,
    RConfig: Write + Seek,
{
    use std::str::FromStr;

    use zip::ZipWriter;

    #[cfg(feature = "progress")]
    if let Ok(mut updater) = UPDATER.try_lock() {
        updater.set_message("")?;
        updater.set_title("Extracting FileSystem changes...")?;
    }

    let (original_root, patched_root) = (original.root, patched.root);
    let root_changes =
        extract_changes(&mut original, original_root, &mut patched, patched_root).await?;

    #[cfg(feature = "progress")]
    if let Ok(mut updater) = UPDATER.try_lock() {
        updater.set_message("")?;
        updater.set_title("Extracting SystemData changes...")?;
    }

    let (original_sys, patched_sys) = (original.sys, patched.sys);
    let sys_changes =
        extract_changes(&mut original, original_sys, &mut patched, patched_sys).await?;

    #[cfg(feature = "progress")]
    if let Ok(mut updater) = UPDATER.try_lock() {
        updater.set_message("")?;
        updater.set_title("Generating Patch File...")?;
    }

    let mut out_zip = ZipWriter::new(output);
    let mut start_dol_change = None;
    if let Some(dol_change) = sys_changes.changes.get(&PathBuf::from_str("Start.dol")?) {
        out_zip.start_file("start_dol.bs.gz2", zip::write::FileOptions::<()>::default())?;
        out_zip.write_all(dol_change)?;
        start_dol_change = Some(PathBuf::from_str("start_dol.bs.gz2")?);
    }

    let mut apploader_change = None;
    if let Some(ldr_change) = sys_changes
        .changes
        .get(&PathBuf::from_str("AppLoader.ldr")?)
    {
        out_zip.start_file("apploader.bs.gz2", zip::write::FileOptions::<()>::default())?;
        out_zip.write_all(ldr_change)?;
        apploader_change = Some(PathBuf::from_str("apploader.bs.gz2")?);
    }

    let mut changes = HashMap::new();
    for (i, (path, diff)) in root_changes.changes.iter().enumerate() {
        let name = format!("change{}.bs.gz2", i);
        out_zip.start_file(&name, zip::write::FileOptions::<()>::default())?;
        out_zip.write_all(diff)?;
        changes.insert(path.to_owned(), PathBuf::from_str(&name)?);
    }

    let mut additions = HashMap::new();
    for (i, file) in root_changes.additions.iter().enumerate() {
        let name = format!("replace{}.dat", i);
        if let Some(file_reader) = patched.get_file_mut(patched.root, file) {
            use futures::{AsyncReadExt, AsyncSeekExt};

            let mut buf = Vec::new();
            file_reader.seek(std::io::SeekFrom::Start(0)).await?;
            file_reader.read_to_end(&mut buf).await?;
            out_zip.start_file(&name, zip::write::FileOptions::<()>::default())?;
            out_zip.write_all(&buf)?;
            additions.insert(
                itertools::Itertools::intersperse(
                    file.iter().map(|c| c.to_string_lossy().to_string()),
                    "/".into(),
                )
                .collect(),
                PathBuf::from_str(&name)?,
            );
        }
    }

    let config = Config {
        info: config::Info {
            ..Default::default()
        },
        src: config::Src {
            iso: "".into(),
            ..Default::default()
        },
        build: config::Build {
            iso: "".into(),
            ..Default::default()
        },
        files: additions,
        diffs: Some(config::Diffs {
            deletions: root_changes.deletions,
            changes,
            dol: start_dol_change,
            loader: apploader_change,
        }),
        ..Default::default()
    };
    let config_data = toml::to_string(&config)?;
    out_zip.start_file("RomHack.toml", zip::write::FileOptions::<()>::default())?;
    out_zip.write_all(config_data.as_bytes())?;
    out_zip.finish()?;

    #[cfg(feature = "progress")]
    if let Ok(mut updater) = UPDATER.lock() {
        updater.set_title("Finished")?;
        updater.finish()?;
    }

    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
pub fn new(name: &str) -> eyre::Result<()> {
    use std::io::Write;

    let exit_code = Command::new("cargo")
        .args(["new", "--lib", name])
        .spawn()
        .context("Couldn't create the cargo project")?
        .wait()?;

    assert!(exit_code.success(), "Couldn't create the cargo project");

    let mut file = File::create(format!("{}/RomHack.toml", name))
        .context("Couldn't create the RomHack.toml")?;
    write!(
        file,
        r#"[info]
game-name = "{0}"

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
iso = "target/{0}.iso"

[link]
entries = ["init"] # Enter the exported function names here
base = "0x8040_1000" # Enter the start address of the Rom Hack's code here
"#,
        name.replace('-', "_"),
    )
    .context("Couldn't write the RomHack.toml")?;

    let mut file = File::create(format!("{}/src/lib.rs", name))
        .context("Couldn't create the lib.rs source file")?;
    write!(
        file,
        r#"#![no_std]

pub mod panic;

#[no_mangle]
pub extern "C" fn init() {{}}
"#
    )
    .context("Couldn't write the lib.rs source file")?;

    let mut file = File::create(format!("{}/src/panic.rs", name))
        .context("Couldn't create the panic.rs source file")?;
    write!(
        file,
        r#"#[cfg(any(target_arch = "powerpc", target_arch = "wasm32"))]
#[panic_handler]
pub fn panic(_info: &::core::panic::PanicInfo) -> ! {{
    loop {{}}
}}
"#
    )
    .context("Couldn't write the panic.rs source file")?;

    let mut file = File::create(format!("{}/src/patch.asm", name))
        .context("Couldn't create the default patch file")?;
    writeln!(
        file,
        r#"; You can use this to patch the game's code to call into the Rom Hack's code"#
    )
    .context("Couldn't write the default patch file")?;

    let mut file = OpenOptions::new()
        .append(true)
        .open(format!("{}/Cargo.toml", name))
        .context("Couldn't open the Cargo.toml")?;
    writeln!(
        file,
        r#"# Comment this in if you want to use the gcn crate in your rom hack.
# It requires the operating system symbols to be resolved via a map.
# gcn = {{ git = "https://github.com/CryZe/gcn", features = ["panic"] }}

[lib]
crate-type = ["staticlib"]

[profile.dev]
panic = "abort"
opt-level = 1

[profile.release]
panic = "abort"
lto = true"#
    )
    .context("Couldn't write into the Cargo.toml")?;

    let mut file = File::create(format!("{}/.gitignore", name))
        .context("Couldn't create the gitignore file")?;
    write!(
        file,
        r#"/target
**/*.rs.bk
"#
    )
    .context("Couldn't write the gitignore file")?;

    Ok(())
}
