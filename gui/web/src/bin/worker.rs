use async_std::io::prelude::ReadExt;
use js_sys::Array;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsValue;

// When the `wee_alloc` feature is enabled, use `wee_alloc` as the global
// allocator.
#[cfg(feature = "wee_alloc")]
#[global_allocator]
static ALLOC: wee_alloc::WeeAlloc<'_> = wee_alloc::WeeAlloc::INIT;

#[derive(Debug)]
struct WebFile {
    handle: web_sys::FileSystemSyncAccessHandle,
    cursor: u64,
}

impl async_std::io::Read for WebFile {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
        buf: &mut [u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        let mut options = web_sys::FileSystemReadWriteOptions::new();
        options.at(self.cursor as f64);
        match self.handle.read_with_u8_array_and_options(buf, &options) {
            Ok(n) => std::task::Poll::Ready(Ok(n as usize)),
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
        self: std::pin::Pin<&mut Self>,
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
            Ok(n) => std::task::Poll::Ready(Ok(n as usize)),
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

#[wasm_bindgen]
pub async extern "C" fn run_patch(
    patch: &JsValue,
    file: &JsValue,
    save: &JsValue,
) -> Result<Array, JsValue> {
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
    log::debug!("Patching game... {:?} {:?} {:?}", patch_access, file_access, save_access);

    save_access.truncate_with_u32(0)?;
    let mut f = WebFile {
        handle: save_access.clone(),
        cursor: 0,
    };
    let mut buf = [0u8; 6];
    let _ = f.read_exact(&mut buf).await;
    log::info!("{buf:X?}");

    let mut f2 = WebFile {
        handle: file_access.clone(),
        cursor: 0,
    };
    let _ = f2.read_exact(&mut buf).await;
    log::info!("{buf:X?}");

    // TODO Start the process...

    let _ = patch_access.flush();
    let _ = file_access.flush();
    let _ = save_access.flush();

    patch_access.close();
    file_access.close();
    save_access.close();

    Ok(Array::of3(patch, file, save))
}

fn main() {
    console_log::init_with_level(log::Level::Debug)
        .expect("could not initialize worker's console_log");
    log::debug!("Starting Worker Thread");
}
