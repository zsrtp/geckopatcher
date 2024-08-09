use std::{fmt::Debug, io::{Read, Seek}, pin::Pin, rc::Rc, task::{Context, Poll}};

use async_std::{io, sync::Mutex};
use futures::{AsyncRead, AsyncSeek, AsyncWrite, TryFutureExt};
use js_sys::Uint8Array;
use wasm_bindgen_futures::JsFuture;
use web_sys::FileSystemWritableFileStream;


#[derive(Debug)]
struct WebFileState {
    handle: web_sys::FileSystemSyncAccessHandle,
    cursor: u64,
}

#[derive(Debug, Clone)]
pub struct WebFile {
    state: Rc<Mutex<WebFileState>>,
}

impl WebFile {
    pub fn new(handle: web_sys::FileSystemSyncAccessHandle) -> Self {
        Self {
            state: Rc::new(Mutex::new(WebFileState { handle, cursor: 0 })),
        }
    }
}

impl Read for WebFile {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let mut state = match self.state.try_lock() {
            Some(guard) => guard,
            None => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    "Failed to lock file state",
                ))
            }
        };
        let mut options = web_sys::FileSystemReadWriteOptions::new();
        options.at(state.cursor as f64);
        match state.handle.read_with_u8_array_and_options(buf, &options) {
            Ok(n) => {
                state.cursor += n as u64;
                Ok(n as usize)
            }
            Err(err) => Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("{err:?}"),
            )),
        }
    }
}

impl Seek for WebFile {
    fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
        let mut state = match self.state.try_lock() {
            Some(guard) => guard,
            None => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    "Failed to lock file state",
                ))
            }
        };
        let len = match state.handle.get_size() {
            Ok(size) => size as u64,
            Err(err) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("{err:?}"),
                ))
            }
        };
        match pos {
            std::io::SeekFrom::Start(pos) => {
                if pos > len {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        "Cursor past end of stream",
                    ));
                }
                state.cursor = pos;
            }
            std::io::SeekFrom::End(pos) => {
                let new_pos = len as i64 + pos;
                if !(0..=len as i64).contains(&new_pos) {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        "Cursor outside of stream range",
                    ));
                }
                state.cursor = new_pos as u64;
            }
            std::io::SeekFrom::Current(pos) => {
                let new_pos = state.cursor as i64 + pos;
                if !(0..=len as i64).contains(&new_pos) {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        "Cursor outside of stream range",
                    ));
                }
                state.cursor = new_pos as u64;
            }
        };
        Ok(state.cursor)
    }
}

impl AsyncRead for WebFile {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut [u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        let this = self.get_mut();
        let mut state = match this.state.try_lock() {
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

impl AsyncSeek for WebFile {
    fn poll_seek(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        pos: std::io::SeekFrom,
    ) -> std::task::Poll<std::io::Result<u64>> {
        let this = self.get_mut();
        let mut state = match this.state.try_lock() {
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

impl AsyncWrite for WebFile {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        let this = self.get_mut();
        let mut state = match this.state.try_lock() {
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
        let state = match this.state.try_lock() {
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
        let state = match this.state.try_lock() {
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

#[derive(Debug, Default)]
enum WebWritableStates {
    #[default]
    Init,
    StartSeek(u64),
    WaitForSeek(u64, JsFuture),

    StartWrite(Vec<u8>),
    WaitForWrite(JsFuture),

    StartClose,
    WaitForClose(JsFuture),
}

#[derive(Debug)]
struct WebWritableState {
    handle: web_sys::FileSystemWritableFileStream,
    state: WebWritableStates,
    cursor: u64,
}

#[derive(Debug, Clone)]
pub struct WebWritable {
    state: Rc<Mutex<WebWritableState>>,
}

impl WebWritable {
    pub fn new(handle: FileSystemWritableFileStream) -> Self {
        Self { state: Rc::new(Mutex::new(WebWritableState { handle, state: WebWritableStates::Init, cursor: 0 })) }
    }
}

impl AsyncSeek for WebWritable {
    fn poll_seek(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        pos: io::SeekFrom,
    ) -> Poll<io::Result<u64>> {
        let this = self.get_mut();
        let mut state = match this.state.try_lock() {
            None => {
                cx.waker().wake_by_ref();
                return Poll::Pending
            },
            Some(state) => state,
        };
        match std::mem::take(&mut state.state) {
            WebWritableStates::Init => {
                let new_cursor = match pos {
                    io::SeekFrom::Start(pos) => pos,
                    io::SeekFrom::End(_) => return Poll::Ready(Err(io::Error::new(io::ErrorKind::Other, "Seek from end is not supported for WebWritable"))),
                    io::SeekFrom::Current(pos) => state.cursor.checked_add_signed(pos).ok_or(io::Error::new(io::ErrorKind::Other, "Seek position out of bound"))?,
                };
                state.state = WebWritableStates::StartSeek(new_cursor);
                cx.waker().wake_by_ref();
                Poll::Pending
            },
            WebWritableStates::StartSeek(cursor) => {
                let promise = state.handle.seek_with_f64(cursor as f64).map_err(|err| io::Error::new(io::ErrorKind::Other, format!("{:?}", err)))?;
                // TODO Transform the promise to a future and pass it onto the next state to check on.
                state.state = WebWritableStates::WaitForSeek(cursor, JsFuture::from(promise));
                cx.waker().wake_by_ref();
                Poll::Pending
            },
            WebWritableStates::WaitForSeek(cursor, mut future) => {
                match future.try_poll_unpin(cx) {
                    Poll::Ready(Ok(_)) => {
                        state.cursor = cursor;
                        Poll::Ready(Ok(cursor))
                    }
                    Poll::Ready(Err(err)) => {
                        Poll::Ready(Err(io::Error::new(io::ErrorKind::Other, format!("{:?}", err))))
                    }
                    Poll::Pending => {
                        state.state = WebWritableStates::WaitForSeek(cursor, future);
                        Poll::Pending
                    },
                }
            },
            _ => Poll::Ready(Err(io::Error::new(io::ErrorKind::Other, "Invalid writer state"))),
        }
    }
}

impl AsyncWrite for WebWritable {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let mut state = match self.get_mut().state.try_lock() {
            None => {
                cx.waker().wake_by_ref();
                return Poll::Pending
            },
            Some(state) => state,
        };
        match std::mem::take(&mut state.state) {
            WebWritableStates::Init => {
                state.state = WebWritableStates::StartWrite(buf.to_vec());
                cx.waker().wake_by_ref();
                Poll::Pending
            }
            WebWritableStates::StartWrite(buffer) => {
                let array = Uint8Array::new_with_length(buffer.len() as u32);
                array.copy_from(buffer.as_slice());
                let promise = state.handle.write_with_buffer_source(&array).map_err(|err| io::Error::new(io::ErrorKind::Other, format!("{:?}", err)))?;
                state.state = WebWritableStates::WaitForWrite(JsFuture::from(promise));
                cx.waker().wake_by_ref();
                Poll::Pending
            }
            WebWritableStates::WaitForWrite(mut future) => {
                match future.try_poll_unpin(cx) {
                    Poll::Ready(Ok(_)) => {
                        state.cursor = state.cursor.checked_add(buf.len() as u64).ok_or(io::Error::new(io::ErrorKind::Other, "Write position out of bound"))?;
                        state.state = WebWritableStates::Init;
                        Poll::Ready(Ok(buf.len()))
                    }
                    Poll::Ready(Err(err)) => {
                        Poll::Ready(Err(io::Error::new(io::ErrorKind::Other, format!("{:?}", err))))
                    }
                    Poll::Pending => {
                        state.state = WebWritableStates::WaitForWrite(future);
                        Poll::Pending
                    },
                }
            }
            _ => Poll::Ready(Err(io::Error::new(io::ErrorKind::Other, "Invalid writer state")))
        }
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let mut state = match self.get_mut().state.try_lock() {
            None => {
                cx.waker().wake_by_ref();
                return Poll::Pending
            },
            Some(state) => state,
        };
        match std::mem::take(&mut state.state) {
            WebWritableStates::Init => {
                state.state = WebWritableStates::StartClose;
                cx.waker().wake_by_ref();
                Poll::Pending
            }
            WebWritableStates::StartClose => {
                let promise = state.handle.close();
                state.state = WebWritableStates::WaitForClose(JsFuture::from(promise));
                cx.waker().wake_by_ref();
                Poll::Pending
            }
            WebWritableStates::WaitForClose(mut future) => {
                match future.try_poll_unpin(cx) {
                    Poll::Ready(Ok(_)) => {
                        state.state = WebWritableStates::Init;
                        Poll::Ready(Ok(()))
                    }
                    Poll::Ready(Err(err)) => {
                        Poll::Ready(Err(io::Error::new(io::ErrorKind::Other, format!("{:?}", err))))
                    }
                    Poll::Pending => {
                        state.state = WebWritableStates::WaitForClose(future);
                        Poll::Pending
                    },
                }
            }
            _ => Poll::Ready(Err(io::Error::new(io::ErrorKind::Other, "Invalid writer state")))
        }
    }
}

#[derive(Debug, Default)]
enum WebReadableStates {
    #[default]
    Init,
    StartRead,
    WaitForRead(JsFuture),
}

#[derive(Debug)]
struct WebReadableState {
    handle: web_sys::File,
    state: WebReadableStates,
    cursor: u64,
}

#[derive(Debug, Clone)]
pub struct WebReadable {
    state: Rc<Mutex<WebReadableState>>,
}

impl WebReadable {
    pub fn new(handle: web_sys::File) -> Self {
        Self { state: Rc::new(Mutex::new(WebReadableState { handle, state: WebReadableStates::default() , cursor: 0 })) }
    }
}

impl AsyncSeek for WebReadable
{
    fn poll_seek(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        pos: std::io::SeekFrom,
    ) -> Poll<std::io::Result<u64>> {
        let this = self.get_mut();
        let mut state = match this.state.try_lock() {
            None => {
                cx.waker().wake_by_ref();
                return Poll::Pending
            },
            Some(state) => state,
        };
        let len = state.handle.size() as u64;
        match pos {
            std::io::SeekFrom::Start(pos) => {
                if pos > len {
                    return Poll::Ready(Err(std::io::Error::new(std::io::ErrorKind::Other, "Cursor past end of stream")));
                }
                state.cursor = pos;
            }
            std::io::SeekFrom::End(pos) => {
                let new_pos = len as i64 + pos;
                if !(0..=len as i64).contains(&new_pos) {
                    return Poll::Ready(Err(std::io::Error::new(std::io::ErrorKind::Other, "Cursor outside of stream range")));
                }
                state.cursor = new_pos as u64;
            }
            std::io::SeekFrom::Current(pos) => {
                let new_pos = state.cursor as i64 + pos;
                if !(0..=len as i64).contains(&new_pos) {
                    return Poll::Ready(Err(std::io::Error::new(std::io::ErrorKind::Other, "Cursor outside of stream range")));
                }
                state.cursor = new_pos as u64;
            }
        };
        Poll::Ready(Ok(state.cursor))
    }
}

impl AsyncRead for WebReadable
{
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<std::io::Result<usize>> {
        let this = self.get_mut();
        let mut state = match this.state.try_lock() {
            None => {
                cx.waker().wake_by_ref();
                return Poll::Pending
            },
            Some(state) => state,
        };
        let len = state.handle.size() as u64;
        if state.cursor + buf.len() as u64 > len {
            return Poll::Ready(Err(std::io::Error::new(std::io::ErrorKind::Other, "Cursor past end of stream")));
        }
        match std::mem::take(&mut state.state) {
            WebReadableStates::Init => {
                state.state = WebReadableStates::StartRead;
                cx.waker().wake_by_ref();
                Poll::Pending
            },
            WebReadableStates::StartRead => {
                let blob = state.handle.slice_with_f64_and_f64(state.cursor as f64, state.cursor as f64 + buf.len() as f64)
                    .map_err(|err| std::io::Error::new(std::io::ErrorKind::Other, format!("{:?}", err)))?;
                let fut = JsFuture::from(blob.array_buffer());
                state.state = WebReadableStates::WaitForRead(fut);
                cx.waker().wake_by_ref();
                Poll::Pending
            },
            WebReadableStates::WaitForRead(mut fut) => {
                match fut.try_poll_unpin(cx) {
                    Poll::Ready(Ok(array)) => {
                        let arr = Uint8Array::new(&array);
                        arr.copy_to(buf);
                        state.cursor += arr.length() as u64;
                        state.state = WebReadableStates::Init;
                        Poll::Ready(Ok(arr.length() as usize))
                    }
                    Poll::Ready(Err(err)) => {
                        Poll::Ready(Err(std::io::Error::new(std::io::ErrorKind::Other, format!("{:?}", err))))
                    }
                    Poll::Pending => {
                        state.state = WebReadableStates::WaitForRead(fut);
                        Poll::Pending
                    }
                }
            },
        }
    }
}
