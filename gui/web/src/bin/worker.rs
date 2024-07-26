use std::sync::Arc;

use async_std::io::prelude::{ReadExt, SeekExt};
use async_std::io::{Read as AsyncRead, Seek as AsyncSeek, Write as AsyncWrite};
use async_std::sync::Mutex;
use futures::AsyncWriteExt;
use geckolib::iso::disc::DiscType;
use geckolib::iso::read::DiscReader;
use geckolib::iso::write::DiscWriter;
use geckolib::vfs::GeckoFS;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsValue;
use web_gui_patcher::io::WebFile;

#[cfg(feature = "debug_alloc")]
#[global_allocator]
static ALLOC: wasm_tracing_allocator::WasmTracingAllocator<std::alloc::System> =
    wasm_tracing_allocator::WasmTracingAllocator(std::alloc::System);

async fn reproc<R: AsyncRead + AsyncSeek + Unpin + Clone + 'static, W: AsyncSeek + AsyncWrite + Unpin + Clone>(
    file: R,
    save: W,
) -> Result<(), eyre::Error> {
    let mut f = DiscReader::new(file).await?;
    {
        f.seek(std::io::SeekFrom::Start(0)).await?;
        let mut buf = vec![0u8; 0x60];
        f.read(&mut buf).await?;
        log::info!(
            "[{}] Game Title: {:02X?}",
            String::from_utf8_lossy(&buf[..6]),
            String::from_utf8_lossy(&buf[0x20..0x60])
                .split_terminator('\0')
                .find(|s| !s.is_empty())
                .expect("This game has no title")
        );
    }
    let mut out = {
        DiscWriter::new(save, f.get_disc_info())
    };

    log::info!("Loading virtual FileSystem...");
    if let DiscWriter::Wii(wii_out) = out.clone() {
        std::pin::pin!(wii_out).init().await?;
    }
    let fs = Arc::new(Mutex::new(GeckoFS::parse(f).await?));
    {
        let is_wii = out.get_type() == DiscType::Wii;
        let mut fs_guard = fs.lock_arc().await;
        log::info!("Writing VFS to output file...");
        fs_guard.serialize(&mut out, is_wii).await?;
        if is_wii {
            log::info!("Encrypting the ISO");
        }
        out.flush().await?;
        out.close().await?;
        log::info!("ISO writing done");
    }
    <eyre::Result<()>>::Ok(())
}

#[wasm_bindgen]
pub async extern "C" fn run_patch(
    patch: &JsValue,
    file: &JsValue,
    save: &JsValue,
) -> Result<(), JsValue> {
    let patch_handle: web_sys::FileSystemFileHandle = patch.clone().dyn_into()?;
    let patch_access: web_sys::FileSystemSyncAccessHandle =
        wasm_bindgen_futures::JsFuture::from(patch_handle.create_sync_access_handle())
            .await?
            .dyn_into()?;
    let file_handle: web_sys::FileSystemFileHandle = file.clone().dyn_into()?;
    let file_access: web_sys::FileSystemSyncAccessHandle =
        wasm_bindgen_futures::JsFuture::from(file_handle.create_sync_access_handle())
            .await?
            .dyn_into()?;
    let save_handle: web_sys::FileSystemFileHandle = save.clone().dyn_into()?;
    log::info!("getting handle...");
    let save_access: web_sys::FileSystemSyncAccessHandle =
        wasm_bindgen_futures::JsFuture::from(save_handle.create_sync_access_handle())
            .await?
            .dyn_into()?;
    log::debug!(
        "Patching game... {:?} {:?} {:?}",
        patch_access,
        file_access,
        save_access
    );

    if let Err(err) = reproc(
        WebFile::new(file_access.clone()),
        WebFile::new(save_access.clone()),
    )
    .await
    {
        web_sys::console::error_1(&format!("{err:?}").into());
        return Err(format!("{err:?}").into());
    }

    log::info!("Reproc finished");

    let _ = patch_access.flush();
    let _ = file_access.flush();
    // let _ = save_access.flush();

    patch_access.close();
    file_access.close();
    // save_access.close();

    Ok(())
}

fn main() {
    #[cfg(debug_assertions)]
    console_log::init_with_level(log::Level::Debug)
        .expect("could not initialize worker's console_log");
    #[cfg(not(debug_assertions))]
    console_log::init_with_level(log::Level::Info)
        .expect("could not initialize worker's console_log");
    web_gui_patcher::progress::init_web_progress();
    log::debug!("Starting Worker Thread");
}
