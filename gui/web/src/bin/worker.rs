use std::sync::Arc;

use async_std::io::prelude::{ReadExt, SeekExt};
use async_std::io::{Read as AsyncRead, Seek as AsyncSeek, Write as AsyncWrite};
use async_std::sync::Mutex;
use geckolib::iso::disc::DiscType;
use geckolib::iso::read::DiscReader;
use geckolib::iso::write::DiscWriter;
use geckolib::vfs::GeckoFS;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsValue;

#[cfg(feature = "debug_alloc")]
#[global_allocator]
static ALLOC: wasm_tracing_allocator::WasmTracingAllocator<std::alloc::System> =
    wasm_tracing_allocator::WasmTracingAllocator(std::alloc::System);

#[derive(Debug)]
struct WebFile {
    handle: web_sys::FileSystemSyncAccessHandle,
    cursor: u64,
}

impl async_std::io::Read for WebFile {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
        buf: &mut [u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        let mut options = web_sys::FileSystemReadWriteOptions::new();
        options.at(self.cursor as f64);
        match self.handle.read_with_u8_array_and_options(buf, &options) {
            Ok(n) => {
                self.cursor += n as u64;
                std::task::Poll::Ready(Ok(n as usize))
            }
            Err(err) => std::task::Poll::Ready(Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("{err:?}"),
            ))),
        }
    }
}

impl async_std::io::Seek for WebFile {
    fn poll_seek(
        mut self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
        pos: std::io::SeekFrom,
    ) -> std::task::Poll<std::io::Result<u64>> {
        let len = match self.handle.get_size() {
            Ok(size) => size as u64,
            Err(err) => {
                return std::task::Poll::Ready(Err(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("{err:?}"),
                )))
            }
        };
        match pos {
            std::io::SeekFrom::Start(pos) => {
                if pos > len {
                    return std::task::Poll::Ready(Err(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        "Cursor past end of stream",
                    )));
                }
                self.cursor = pos;
            }
            std::io::SeekFrom::End(pos) => {
                let new_pos = len as i64 + pos;
                if !(0..=len as i64).contains(&new_pos) {
                    return std::task::Poll::Ready(Err(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        "Cursor outside of stream range",
                    )));
                }
                self.cursor = new_pos as u64;
            }
            std::io::SeekFrom::Current(pos) => {
                let new_pos = self.cursor as i64 + pos;
                if !(0..=len as i64).contains(&new_pos) {
                    return std::task::Poll::Ready(Err(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        "Cursor outside of stream range",
                    )));
                }
                self.cursor = new_pos as u64;
            }
        };
        std::task::Poll::Ready(Ok(self.cursor))
    }
}

impl async_std::io::Write for WebFile {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        let mut options = web_sys::FileSystemReadWriteOptions::new();
        options.at(self.cursor as f64);
        let mut b = buf.to_vec();
        match self
            .handle
            .write_with_u8_array_and_options(&mut b, &options)
        {
            Ok(n) => {
                self.cursor += n as u64;
                std::task::Poll::Ready(Ok(n as usize))
            }
            Err(err) => std::task::Poll::Ready(Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("{err:?}"),
            ))),
        }
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        match self.handle.flush() {
            Ok(_) => std::task::Poll::Ready(Ok(())),
            Err(err) => std::task::Poll::Ready(Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("{err:?}"),
            ))),
        }
    }

    fn poll_close(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        self.handle.close();
        std::task::Poll::Ready(Ok(()))
    }
}

async fn reproc<R: AsyncRead + AsyncSeek + 'static, W: AsyncRead + AsyncSeek + AsyncWrite>(
    file: R,
    save: W,
) -> Result<(), eyre::Error> {
    let f = Arc::new(Mutex::new(DiscReader::new(file).await?));
    {
        let mut guard = f.lock_arc().await;
        guard.seek(std::io::SeekFrom::Start(0)).await?;
        let mut buf = vec![0u8; 0x60];
        guard.read(&mut buf).await?;
        log::info!(
            "[{}] Game Title: {:02X?}",
            String::from_utf8_lossy(&buf[..6]),
            String::from_utf8_lossy(&buf[0x20..0x60])
                .split_terminator('\0')
                .find(|s| !s.is_empty())
                .expect("This game has no title")
        );
    }
    let out = {
        let guard = f.lock_arc().await;
        DiscWriter::new(save, guard.get_disc_info()).await?
    };

    let mut out = std::pin::pin!(out);
    let fs = GeckoFS::parse(f).await?;
    {
        let mut fs_guard = fs.lock_arc().await;
        let is_wii = out.get_type() == DiscType::Wii;
        fs_guard.serialize(&mut out, is_wii).await?;
        if is_wii {
            log::info!("Encrypting the ISO");
        }
        out.finalize().await?;
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
        WebFile {
            cursor: 0,
            handle: file_access.clone(),
        },
        WebFile {
            cursor: 0,
            handle: save_access.clone(),
        },
    )
    .await
    {
        web_sys::console::error_1(&format!("{err:?}").into());
        return Err(format!("{err:?}").into());
    }

    log::info!("Reproc finished");

    let _ = patch_access.flush();
    let _ = file_access.flush();
    let _ = save_access.flush();

    patch_access.close();
    file_access.close();
    save_access.close();

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
