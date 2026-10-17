use std::io::{Read, Seek, SeekFrom, Write};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use futures::io::{AsyncRead, AsyncSeek, AsyncWrite};

/// A blocking file wrapped in the `futures` async I/O traits.
///
/// Intended for native use only, where patching already runs on a dedicated
/// thread (or under a simple `block_on`), so blocking file I/O is fine and
/// avoids pulling a full async runtime into the dependency tree.
#[derive(Clone, Debug)]
pub struct AsyncFile {
    inner: Arc<Mutex<std::fs::File>>,
}

impl AsyncFile {
    pub fn new(file: std::fs::File) -> Self {
        Self {
            inner: Arc::new(Mutex::new(file)),
        }
    }
}

impl AsyncRead for AsyncFile {
    fn poll_read(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<std::io::Result<usize>> {
        let mut file = self.inner.lock().expect("AsyncFile lock poisoned");
        Poll::Ready(file.read(buf))
    }
}

impl AsyncSeek for AsyncFile {
    fn poll_seek(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        pos: SeekFrom,
    ) -> Poll<std::io::Result<u64>> {
        let mut file = self.inner.lock().expect("AsyncFile lock poisoned");
        Poll::Ready(file.seek(pos))
    }
}

impl AsyncWrite for AsyncFile {
    fn poll_write(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        let mut file = self.inner.lock().expect("AsyncFile lock poisoned");
        Poll::Ready(file.write(buf))
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        let mut file = self.inner.lock().expect("AsyncFile lock poisoned");
        Poll::Ready(file.flush())
    }

    fn poll_close(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}
