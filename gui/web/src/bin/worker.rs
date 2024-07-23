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

#[cfg(feature = "debug_alloc")]
#[global_allocator]
static ALLOC: wasm_tracing_allocator::WasmTracingAllocator<std::alloc::System> =
    wasm_tracing_allocator::WasmTracingAllocator(std::alloc::System);

#[derive(Debug)]
struct WebFileState {
    handle: web_sys::FileSystemSyncAccessHandle,
    cursor: u64,
}

#[derive(Debug, Clone)]
struct WebFile {
    state: Arc<Mutex<WebFileState>>,
}

impl WebFile {
    fn new(handle: web_sys::FileSystemSyncAccessHandle) -> Self {
        Self {
            state: Arc::new(Mutex::new(WebFileState { handle, cursor: 0 })),
        }
    }
}

impl async_std::io::Read for WebFile {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut [u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        let this = self.get_mut();
        let mut state = match this.state.try_lock_arc() {
            Some(guard) => guard,
            None => {
                cx.waker().wake_by_ref();
                return std::task::Poll::Pending;
            }
        };
        if let Err(err) = state.handle.flush() {
            return std::task::Poll::Ready(Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("{err:?}"),
            )));
        };
        let mut options = web_sys::FileSystemReadWriteOptions::new();
        options.at(state.cursor as f64);
        match state.handle.read_with_u8_array_and_options(buf, &options) {
            Ok(n) => {
                state.cursor += n as u64;
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
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        pos: std::io::SeekFrom,
    ) -> std::task::Poll<std::io::Result<u64>> {
        let this = self.get_mut();
        let mut state = match this.state.try_lock_arc() {
            Some(guard) => guard,
            None => {
                cx.waker().wake_by_ref();
                return std::task::Poll::Pending;
            }
        };
        if let Err(err) = state.handle.flush() {
            return std::task::Poll::Ready(Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("{err:?}"),
            )));
        };
        let len = match state.handle.get_size() {
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
                state.cursor = pos;
            }
            std::io::SeekFrom::End(pos) => {
                let new_pos = len as i64 + pos;
                if !(0..=len as i64).contains(&new_pos) {
                    return std::task::Poll::Ready(Err(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        "Cursor outside of stream range",
                    )));
                }
                state.cursor = new_pos as u64;
            }
            std::io::SeekFrom::Current(pos) => {
                let new_pos = state.cursor as i64 + pos;
                if !(0..=len as i64).contains(&new_pos) {
                    return std::task::Poll::Ready(Err(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        "Cursor outside of stream range",
                    )));
                }
                state.cursor = new_pos as u64;
            }
        };
        std::task::Poll::Ready(Ok(state.cursor))
    }
}

impl async_std::io::Write for WebFile {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        let this = self.get_mut();
        let mut state = match this.state.try_lock_arc() {
            Some(guard) => guard,
            None => {
                cx.waker().wake_by_ref();
                return std::task::Poll::Pending;
            }
        };
        if let Err(err) = state.handle.flush() {
            return std::task::Poll::Ready(Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("{err:?}"),
            )));
        };
        let mut options = web_sys::FileSystemReadWriteOptions::new();
        options.at(state.cursor as f64);
        match state
            .handle
            .write_with_u8_array_and_options(buf, &options)
        {
            Ok(n) => {
                state.cursor += n as u64;
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
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let this = self.get_mut();
        let state = match this.state.try_lock_arc() {
            Some(guard) => guard,
            None => {
                cx.waker().wake_by_ref();
                return std::task::Poll::Pending;
            }
        };
        match state.handle.flush() {
            Ok(_) => std::task::Poll::Ready(Ok(())),
            Err(err) => std::task::Poll::Ready(Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("{err:?}"),
            ))),
        }
    }

    fn poll_close(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let this = self.get_mut();
        let state = match this.state.try_lock_arc() {
            Some(guard) => guard,
            None => {
                cx.waker().wake_by_ref();
                return std::task::Poll::Pending;
            }
        };
        state.handle.close();
        std::task::Poll::Ready(Ok(()))
    }
}

async fn reproc<R: AsyncRead + AsyncSeek + 'static, W: AsyncSeek + AsyncWrite + Unpin + Clone>(
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
