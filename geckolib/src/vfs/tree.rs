#[cfg(feature = "progress")]
use crate::{UPDATER, iso::FstNodeType};
use crate::{
    crypto::Unpackable,
    iso::{
        FstEntry, FstEntryError, FstNode,
        consts::{self, *},
        disc::{DiscType, align_addr},
        read::DiscReader,
        write::DiscWriter,
    },
    vfs::data_source::FileDataSource,
};
use byteorder::{BE, ByteOrder};
use futures::{
    AsyncRead, AsyncReadExt, AsyncSeek, AsyncSeekExt, AsyncWrite, AsyncWriteExt as _, io,
};
use indextree::{Arena, NodeId};
use num::ToPrimitive as _;
#[cfg(feature = "parallel")]
use rayon::{iter::ParallelIterator, slice::ParallelSlice};
#[cfg(feature = "progress")]
use std::sync::TryLockError;
use std::{
    borrow::{Borrow, BorrowMut},
    io::{ErrorKind, Seek, SeekFrom},
    ops::{Deref, DerefMut},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    task::{Context, Poll},
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum FsFileError {
    #[error("Failed to lock the file status")]
    StatusLockError,
    #[error("Failed to lock the file data")]
    DataLockError,
}

impl From<FsFileError> for io::Error {
    fn from(value: FsFileError) -> Self {
        io::Error::other(value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub(super) enum FileState {
    #[default]
    Seeking,
    Reading,
}

#[derive(Debug, Clone, Copy, Default)]
pub(super) struct FileStatus {
    cursor: u64,
    state: FileState,
}

#[derive(Debug, Clone)]
pub struct FsFile<R> {
    data: Arc<Mutex<FileDataSource<R>>>,
    status: Arc<Mutex<FileStatus>>,
}

impl<R> FsFile<R> {
    pub fn new(data: FileDataSource<R>) -> Self {
        Self {
            data: Arc::new(Mutex::new(data)),
            status: Default::default(),
        }
    }
}

impl<R> FsFile<R> {
    pub fn set_data(&mut self, data: Box<[u8]>) -> Result<(), FsFileError> {
        match self.data.lock() {
            Ok(mut data_source) => {
                let name = data_source.name();
                *data_source = FileDataSource::Box { data, name };
                Ok(())
            }
            Err(_) => Err(FsFileError::StatusLockError),
        }
    }

    pub fn name(&self) -> String {
        self.data.lock().map(|data| data.name()).unwrap_or_default()
    }

    pub fn len(&self) -> Result<usize, FsFileError> {
        self.data
            .lock()
            .map(|data| data.len())
            .map_err(|_| FsFileError::DataLockError)
    }

    pub fn is_empty(&self) -> Result<bool, FsFileError> {
        Ok(self.len()? == 0)
    }
}

impl<R> AsyncSeek for FsFile<R>
where
    R: AsyncSeek + Unpin,
{
    fn poll_seek(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut Context<'_>,
        pos: SeekFrom,
    ) -> Poll<std::io::Result<u64>> {
        crate::trace!("Seeking \"{0}\" to {1:?} ({1:016X?})", self.name(), pos);
        let mut status = match self.status.try_lock() {
            Ok(data) => data,
            Err(std::sync::TryLockError::WouldBlock) => return Poll::Pending,
            Err(std::sync::TryLockError::Poisoned(_)) => {
                return Poll::Ready(Err(FsFileError::StatusLockError.into()));
            }
        };
        let mut data = match self.data.try_lock() {
            Ok(data) => data,
            Err(std::sync::TryLockError::WouldBlock) => return Poll::Pending,
            Err(std::sync::TryLockError::Poisoned(_)) => {
                return Poll::Ready(Err(FsFileError::DataLockError.into()));
            }
        };
        let pos = match pos {
            SeekFrom::Start(pos) => {
                if pos > data.len() as u64 {
                    return Poll::Ready(Err(io::Error::new(
                        ErrorKind::InvalidInput,
                        "Index out of range",
                    )));
                }
                SeekFrom::Start(pos)
            }
            SeekFrom::End(pos) => {
                let new_pos = self.len().map_err(io::Error::other)? as i64 + pos;
                if new_pos < 0 || pos > 0 {
                    return Poll::Ready(Err(io::Error::new(
                        ErrorKind::InvalidInput,
                        "Index out of range",
                    )));
                }
                SeekFrom::End(pos)
            }
            SeekFrom::Current(pos) => {
                let new_pos = status.cursor as i64 + pos;
                if new_pos < 0 || new_pos > self.len().map_err(io::Error::other)? as i64 {
                    return Poll::Ready(Err(io::Error::new(
                        ErrorKind::InvalidInput,
                        "Index out of range",
                    )));
                }
                SeekFrom::Current(pos)
            }
        };
        match &mut *data {
            FileDataSource::Reader { fst, .. } => match pos {
                SeekFrom::Start(pos) => {
                    status.cursor = pos;
                    Poll::Ready(Ok(status.cursor))
                }
                SeekFrom::End(pos) => {
                    status.cursor = (fst.get_file_size().unwrap() as i64 + pos) as u64;
                    Poll::Ready(Ok(status.cursor))
                }
                SeekFrom::Current(pos) => {
                    status.cursor = (status.cursor as i64 + pos) as u64;
                    Poll::Ready(Ok(status.cursor))
                }
            },
            FileDataSource::Box { data, .. } => match pos {
                SeekFrom::Start(pos) => {
                    status.cursor = pos;
                    Poll::Ready(Ok(status.cursor))
                }
                SeekFrom::End(pos) => {
                    status.cursor = (data.len() as i64 + pos) as u64;
                    Poll::Ready(Ok(status.cursor))
                }
                SeekFrom::Current(pos) => {
                    status.cursor = (status.cursor as i64 + pos) as u64;
                    Poll::Ready(Ok(status.cursor))
                }
            },
        }
    }
}

impl<R> std::io::Seek for FsFile<R>
where
    R: std::io::Seek,
{
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        crate::trace!("Seeking \"{0}\" to {1:?} ({1:016X?})", self.name(), pos);
        let mut status = self
            .status
            .lock()
            .map_err(|_| FsFileError::StatusLockError)?;
        let mut data = self.data.lock().map_err(|_| FsFileError::DataLockError)?;
        let pos = match pos {
            SeekFrom::Start(pos) => {
                if pos > data.len() as u64 {
                    return Err(io::Error::new(
                        ErrorKind::InvalidInput,
                        "Index out of range",
                    ));
                }
                SeekFrom::Start(pos)
            }
            SeekFrom::End(pos) => {
                let new_pos = self.len().map_err(io::Error::other)? as i64 + pos;
                if new_pos < 0 || pos > 0 {
                    return Err(io::Error::new(
                        ErrorKind::InvalidInput,
                        "Index out of range",
                    ));
                }
                SeekFrom::End(pos)
            }
            SeekFrom::Current(pos) => {
                let new_pos = status.cursor as i64 + pos;
                if new_pos < 0 || new_pos > self.len().map_err(io::Error::other)? as i64 {
                    return Err(io::Error::new(
                        ErrorKind::InvalidInput,
                        "Index out of range",
                    ));
                }
                SeekFrom::Current(pos)
            }
        };
        match &mut *data {
            FileDataSource::Reader { fst, .. } => match pos {
                SeekFrom::Start(pos) => {
                    status.cursor = pos;
                    Ok(status.cursor)
                }
                SeekFrom::End(pos) => {
                    status.cursor = (fst.get_file_size().unwrap() as i64 + pos) as u64;
                    Ok(status.cursor)
                }
                SeekFrom::Current(pos) => {
                    status.cursor = (status.cursor as i64 + pos) as u64;
                    Ok(status.cursor)
                }
            },
            FileDataSource::Box { data, .. } => match pos {
                SeekFrom::Start(pos) => {
                    status.cursor = pos;
                    Ok(status.cursor)
                }
                SeekFrom::End(pos) => {
                    status.cursor = (data.len() as i64 + pos) as u64;
                    Ok(status.cursor)
                }
                SeekFrom::Current(pos) => {
                    status.cursor = (status.cursor as i64 + pos) as u64;
                    Ok(status.cursor)
                }
            },
        }
    }
}

impl<R> AsyncRead for FsFile<R>
where
    R: AsyncRead + AsyncSeek + Unpin,
{
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut [u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        crate::trace!(
            "Reading \"{}\" for 0x{:08X} byte(s)",
            self.name(),
            buf.len()
        );
        let mut status = match self.status.try_lock() {
            Ok(data) => data,
            Err(std::sync::TryLockError::WouldBlock) => return Poll::Pending,
            Err(std::sync::TryLockError::Poisoned(_)) => {
                return Poll::Ready(Err(FsFileError::StatusLockError.into()));
            }
        };
        let mut data = match self.data.try_lock() {
            Ok(data) => data,
            Err(std::sync::TryLockError::WouldBlock) => return Poll::Pending,
            Err(std::sync::TryLockError::Poisoned(_)) => {
                return Poll::Ready(Err(FsFileError::DataLockError.into()));
            }
        };
        let cursor = status.cursor;
        let end = std::cmp::min(buf.len(), (data.len() as i64 - cursor as i64) as usize);
        match status.state {
            FileState::Seeking => match *data {
                FileDataSource::Reader {
                    ref mut reader,
                    ref fst,
                } => {
                    let guard_pin = std::pin::pin!(reader);
                    match guard_pin
                        .poll_seek(cx, SeekFrom::Start(fst.get_file_offset().unwrap() + cursor))
                    {
                        Poll::Ready(Ok(_)) => {
                            status.state = FileState::Reading;
                            cx.waker().wake_by_ref();
                            Poll::Pending
                        }
                        Poll::Ready(Err(err)) => {
                            status.state = FileState::Seeking;
                            Poll::Ready(Err(err))
                        }
                        Poll::Pending => Poll::Pending,
                    }
                }
                FileDataSource::Box { ref data, .. } => {
                    if cursor > data.len() as u64 {
                        Poll::Ready(Err(io::Error::from(io::ErrorKind::InvalidInput)))
                    } else {
                        status.state = FileState::Reading;
                        cx.waker().wake_by_ref();
                        Poll::Pending
                    }
                }
            },
            FileState::Reading => match *data {
                FileDataSource::Reader { ref mut reader, .. } => {
                    match std::pin::pin!(reader).poll_read(cx, &mut buf[..end]) {
                        Poll::Ready(Ok(num_read)) => {
                            status.cursor += num_read as u64;
                            status.state = FileState::Seeking;
                            Poll::Ready(Ok(num_read))
                        }
                        Poll::Ready(Err(err)) => {
                            status.state = FileState::Seeking;
                            Poll::Ready(Err(err))
                        }
                        Poll::Pending => Poll::Pending,
                    }
                }
                FileDataSource::Box { ref data, .. } => {
                    let num_read = std::cmp::min(buf.len(), (data.len() as u64 - cursor) as usize);
                    buf[..num_read].copy_from_slice(&data[cursor as usize..][..num_read]);
                    status.cursor += num_read as u64;
                    status.state = FileState::Seeking;
                    Poll::Ready(Ok(num_read))
                }
            },
        }
    }
}

impl<R> std::io::Read for FsFile<R>
where
    R: std::io::Read + std::io::Seek,
{
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        crate::trace!(
            "Reading \"{}\" for 0x{:08X} byte(s)",
            self.name(),
            buf.len()
        );
        let mut status = self
            .status
            .lock()
            .map_err(|_| FsFileError::StatusLockError)?;
        let mut data = self.data.lock().map_err(|_| FsFileError::DataLockError)?;
        let cursor = status.cursor;
        let end = std::cmp::min(buf.len(), (data.len() as i64 - cursor as i64) as usize);
        // Seek to the cursor
        match *data {
            FileDataSource::Reader {
                ref mut reader,
                ref fst,
            } => match reader.seek(SeekFrom::Start(fst.get_file_offset().unwrap() + cursor)) {
                Ok(_) => {}
                Err(err) => {
                    status.state = FileState::Seeking;
                    return Err(err);
                }
            },
            FileDataSource::Box { ref data, .. } => {
                if cursor > data.len() as u64 {
                    return Err(io::Error::from(io::ErrorKind::InvalidInput));
                }
            }
        }
        // Then read the data
        match *data {
            FileDataSource::Reader { ref mut reader, .. } => match reader.read(&mut buf[..end]) {
                Ok(num_read) => {
                    status.cursor += num_read as u64;
                    status.state = FileState::Seeking;
                    Ok(num_read)
                }
                Err(err) => {
                    status.state = FileState::Seeking;
                    Err(err)
                }
            },
            FileDataSource::Box { ref data, .. } => {
                let num_read = std::cmp::min(buf.len(), (data.len() as u64 - cursor) as usize);
                buf[..num_read].copy_from_slice(&data[cursor as usize..][..num_read]);
                status.cursor += num_read as u64;
                status.state = FileState::Seeking;
                Ok(num_read)
            }
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum FsNodeError {
    #[error("FileSystem node {0:?} is not a directory: {1:?}")]
    NodeIsNotADirectory(String, PathBuf),
    #[error("FileSystem node is not a file: {0:?}")]
    NodeIsNotAFile(PathBuf),
    #[error(transparent)]
    FsFileError(#[from] FsFileError),
}

#[derive(Debug)]
pub enum FsNode<R> {
    Directory { name: String },
    File { file: FsFile<R> },
}

impl<R> FsNode<R> {
    fn assert_dir<A: Deref<Target = Arena<FsNode<R>>>>(
        node: NodeId,
        arena: A,
    ) -> Result<(), FsNodeError> {
        // First, check the parent is a directory
        if let Some(n) = arena.get(node)
            && let FsNode::File { .. } = n.get()
        {
            let path = node
                .ancestors(arena.borrow())
                .filter_map(|n| arena.get(n))
                .map(|n| n.get().name())
                .collect::<Vec<_>>()
                .iter()
                .rev()
                .skip(1)
                .collect();
            return Err(FsNodeError::NodeIsNotADirectory(n.get().name(), path));
        }
        Ok(())
    }

    pub fn new_file<A: DerefMut<Target = Arena<FsNode<R>>>>(
        parent: NodeId,
        data: FileDataSource<R>,
        mut arena: A,
    ) -> Result<NodeId, FsNodeError> {
        // First, check the parent is a directory
        Self::assert_dir(parent, arena.deref())?;
        let id = parent.append_value(
            Self::File {
                file: FsFile::new(data),
            },
            arena.borrow_mut(),
        );
        Ok(id)
    }

    pub fn new_directory<S: Into<String>, A: DerefMut<Target = Arena<FsNode<R>>>>(
        parent: NodeId,
        name: S,
        mut arena: A,
    ) -> Result<NodeId, FsNodeError> {
        // First, check the parent is a directory
        Self::assert_dir(parent, arena.deref())?;
        let id = parent.append_value(Self::Directory { name: name.into() }, arena.borrow_mut());
        Ok(id)
    }

    pub fn is_dir(&self) -> bool {
        match self {
            FsNode::Directory { .. } => true,
            FsNode::File { .. } => false,
        }
    }

    pub fn is_file(&self) -> bool {
        match self {
            Self::Directory { .. } => false,
            Self::File { .. } => true,
        }
    }

    pub fn as_file(self) -> Option<FsFile<R>> {
        match self {
            Self::Directory { .. } => None,
            Self::File { file, .. } => Some(file),
        }
    }

    pub fn as_file_ref(&self) -> Option<&FsFile<R>> {
        match self {
            Self::Directory { .. } => None,
            Self::File { file, .. } => Some(file),
        }
    }

    pub fn as_file_mut(&mut self) -> Option<&mut FsFile<R>> {
        match self {
            Self::Directory { .. } => None,
            Self::File { file, .. } => Some(file),
        }
    }

    pub fn name(&self) -> String {
        match self {
            Self::Directory { name, .. } => name.into(),
            Self::File { file, .. } => file.name(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum GeckoFSError {
    #[error("Error while reading the disc {0}")]
    ReadError(#[from] std::io::Error),
    #[error(transparent)]
    FsNodeError(#[from] FsNodeError),
    #[error(transparent)]
    FsFileError(#[from] FsFileError),
    #[error("File {0:?} not found")]
    FileNotFount(PathBuf),
    #[error("An invalid node was provided")]
    InvalidNode,
    #[error("The buffer too large, it will exceed the u64 limit of the cursor")]
    BufferTooLarge,
    #[error("The provided path {0:?} is not valid")]
    InvalidPath(PathBuf),
    #[error("Node {0:?} not found")]
    NodeNotFound(PathBuf),
    #[error(transparent)]
    FstEntryError(#[from] FstEntryError),
    #[error("Directory stack underflowed while serializing")]
    DirStackUnderflow,
    #[error(transparent)]
    UpdaterError(#[from] eyre::Report),
    #[error("The provided path contains a file {0}")]
    NodeIsAFile(PathBuf),
    #[error(transparent)]
    PoisonError(eyre::Report),
}

pub struct GeckoFS<R> {
    pub root: NodeId,
    pub sys: NodeId,
    arena: Arena<FsNode<R>>,
}

impl<R> GeckoFS<R> {
    pub fn new_directory<S: Into<String>>(
        &mut self,
        parent: NodeId,
        name: S,
    ) -> Result<NodeId, GeckoFSError> {
        FsNode::new_directory(parent, name, &mut self.arena).map_err(Into::into)
    }

    pub fn new_file(
        &mut self,
        parent: NodeId,
        data: FileDataSource<R>,
    ) -> Result<NodeId, GeckoFSError> {
        FsNode::new_file(parent, data, &mut self.arena).map_err(Into::into)
    }

    pub fn get_node_ref<P: AsRef<Path>>(
        &self,
        root: NodeId,
        path: P,
    ) -> Result<&FsNode<R>, GeckoFSError> {
        self.get_nodeid(root, path.as_ref())
            .ok()
            .and_then(|n| self.arena.get(n))
            .map(indextree::Node::get)
            .ok_or(GeckoFSError::NodeNotFound(path.as_ref().into()))
    }

    pub fn get_node_mut<P: AsRef<Path>>(
        &mut self,
        root: NodeId,
        path: P,
    ) -> Result<&mut FsNode<R>, GeckoFSError> {
        self.get_nodeid(root, path.as_ref())
            .ok()
            .and_then(|n| self.arena.get_mut(n))
            .map(indextree::Node::get_mut)
            .ok_or(GeckoFSError::NodeNotFound(path.as_ref().into()))
    }

    pub fn get_file_ref<P: AsRef<Path>>(&self, root: NodeId, path: P) -> Option<&FsFile<R>> {
        self.get_node_ref(root, path)
            .ok()
            .and_then(|f| f.as_file_ref())
    }

    pub fn get_file_mut<P: AsRef<Path>>(
        &mut self,
        root: NodeId,
        path: P,
    ) -> Option<&mut FsFile<R>> {
        self.get_node_mut(root, path)
            .ok()
            .filter(|n| n.is_file())
            .and_then(|f| f.as_file_mut())
    }

    pub fn get_dir_ref<P: AsRef<Path>>(&self, root: NodeId, path: P) -> Option<&FsNode<R>> {
        self.get_node_ref(root, path).ok().filter(|n| n.is_dir())
    }

    pub fn get_dir_mut<P: AsRef<Path>>(&mut self, root: NodeId, path: P) -> Option<&mut FsNode<R>> {
        self.get_node_mut(root, path).ok().filter(|n| n.is_dir())
    }

    pub fn get_path(&self, node: NodeId) -> Option<PathBuf> {
        if node
            .ancestors(&self.arena)
            .any(|a| self.arena.get(a).is_none())
        {
            return None;
        }
        Some(
            node.ancestors(&self.arena)
                .filter_map(|a| self.arena.get(a).map(|a| a.get()))
                .map(|a| a.name())
                .collect::<Vec<_>>()
                .iter()
                .rev()
                .skip(1)
                .collect(),
        )
    }

    pub fn get_nodeid<P: AsRef<Path>>(
        &self,
        root: NodeId,
        path: P,
    ) -> Result<NodeId, GeckoFSError> {
        if path.as_ref().iter().any(|c| c.to_str().is_none()) {
            return Err(GeckoFSError::InvalidPath(path.as_ref().into()));
        }
        path.as_ref()
            .iter()
            .filter_map(|c| c.to_str())
            .try_fold(root, |node, c| {
                node.children(&self.arena).find(|ch| {
                    self.arena
                        .get(*ch)
                        .map(|n| n.get())
                        .is_some_and(|n| n.name() == c)
                })
            })
            .ok_or(GeckoFSError::NodeNotFound(path.as_ref().into()))
    }

    #[doc = "Iterates through all entries under the provided root node using Depth First Search"]
    pub fn iter_dfs(&self, root: NodeId) -> impl Iterator<Item = PathBuf> {
        root.descendants(&self.arena).skip(1).map(move |d| {
            d.ancestors(&self.arena)
                .filter_map(|a| self.arena.get(a))
                .map(|a| a.get().name())
                .collect::<Vec<_>>()
                .iter()
                .rev()
                .skip(1)
                .collect()
        })
    }

    pub fn iter_nodes_dfs(&self, root: NodeId) -> impl Iterator<Item = &FsNode<R>> {
        root.descendants(&self.arena)
            .skip(1)
            .filter_map(|n| self.arena.get(n).map(|node| node.get()))
    }

    pub fn enumerate_nodes_dfs(&self, root: NodeId) -> impl Iterator<Item = (PathBuf, &FsNode<R>)> {
        root.descendants(&self.arena).skip(1).filter_map(move |n| {
            let path = n
                .ancestors(&self.arena)
                .filter_map(|a| self.arena.get(a))
                .map(|a| a.get().name())
                .collect::<Vec<_>>()
                .iter()
                .rev()
                .skip(1)
                .collect();
            self.arena.get(n).map(|node| (path, node.get()))
        })
    }

    pub fn iter_path_dfs(&self, root: NodeId) -> impl Iterator<Item = PathBuf> {
        root.descendants(&self.arena)
            .skip(1)
            .filter_map(|node| self.get_path(node))
    }

    fn get_file_len<P: AsRef<Path>>(&self, root: NodeId, file: P) -> Result<usize, GeckoFSError> {
        self.get_node_ref(root, file.as_ref())
            .and_then(|n| {
                n.as_file_ref()
                    .ok_or(GeckoFSError::FileNotFount(file.as_ref().into()))
            })
            .and_then(|f| f.len().map_err(Into::into))
    }

    pub fn mkdirs<P: AsRef<Path>>(
        &mut self,
        root: NodeId,
        path: P,
    ) -> Result<NodeId, GeckoFSError> {
        if path.as_ref().iter().any(|c| c.to_str().is_none()) {
            return Err(GeckoFSError::InvalidPath(path.as_ref().into()));
        }
        let mut current_node = root;
        for component in path.as_ref().iter().filter_map(|c| c.to_str()) {
            // Check if a node exists with `component` name in the current_node
            current_node = match current_node.children(&self.arena).find(|n| {
                self.arena
                    .get(*n)
                    .map(|n| n.get())
                    .is_some_and(|n| n.name() == component)
            }) {
                Some(node) => {
                    // We found a child with `component` name. Check if it is a directory
                    if self.arena.get(node).filter(|n| n.get().is_dir()).is_none() {
                        return Err(GeckoFSError::NodeIsAFile(path.as_ref().into()));
                    } else {
                        node
                    }
                }
                None => {
                    // We didn't find any children with `component` name. Create a new one
                    FsNode::new_directory(current_node, component, &mut self.arena)?
                }
            }
        }
        Ok(current_node)
    }

    pub fn rm<P>(&mut self, root: NodeId, path: P) -> Result<(), GeckoFSError>
    where
        P: AsRef<Path>,
    {
        self.get_nodeid(root, path)
            .map(|n| n.remove_subtree(&mut self.arena))
    }
}

impl<R: Clone> GeckoFS<R> {
    fn get_dir_structure_recursive<A: DerefMut<Target = Arena<FsNode<R>>>>(
        cur_index: &mut usize,
        fst: &Vec<FstNode>,
        parent_dir: NodeId,
        mut arena: A,
        reader: &DiscReader<R>,
    ) -> Result<(), GeckoFSError> {
        let entry = &fst[*cur_index];

        match entry.clone() {
            FstNode::Directory {
                relative_file_name,
                parent_dir: _,
                next_dir_index,
            } => {
                let dir = FsNode::new_directory(
                    parent_dir,
                    relative_file_name.clone(),
                    arena.deref_mut(),
                )?;

                while *cur_index < next_dir_index - 1 {
                    *cur_index += 1;
                    GeckoFS::get_dir_structure_recursive(
                        cur_index,
                        fst,
                        dir,
                        arena.deref_mut(),
                        reader,
                    )?;
                }
                Ok(())
            }
            FstNode::File {
                relative_file_name,
                file_offset,
                file_size,
            } => {
                let _ = FsNode::new_file(
                    parent_dir,
                    FileDataSource::Reader {
                        reader: reader.clone(),
                        fst: FstNode::File {
                            relative_file_name,
                            file_offset,
                            file_size,
                        },
                    },
                    arena,
                );
                Ok(())
            }
        }
    }
}

impl<R> GeckoFS<R> {
    /// Visits the directory tree to calculate the length of the FST table
    fn visitor_fst_len(&self, mut acc: u64, node: NodeId) -> Result<u64, GeckoFSError> {
        match self.arena.get(node).ok_or(GeckoFSError::InvalidNode)?.get() {
            FsNode::Directory { name } => {
                acc += 12 + name.len() as u64 + 1;

                for child in node.children(&self.arena) {
                    acc = self.visitor_fst_len(acc, child)?;
                }
            }
            FsNode::File { file } => {
                acc += 12 + file.name().len() as u64 + 1;
            }
        };
        Ok(acc)
    }
}

impl<R> GeckoFS<R>
where
    R: AsyncRead + AsyncSeek + Unpin,
{
    async fn read_file_to_end<P: AsRef<Path>>(
        &mut self,
        root: NodeId,
        file: P,
        buf: &mut Vec<u8>,
    ) -> Result<usize, GeckoFSError> {
        let f = self
            .get_node_mut(root, file.as_ref())?
            .as_file_mut()
            .ok_or(FsNodeError::NodeIsNotAFile(file.as_ref().into()))?;
        f.seek(SeekFrom::Start(0)).await?;
        Ok(f.read_to_end(buf).await?)
    }
}

#[doc = r"Utility function to read the disc."]
async fn read_exact_async<R: AsyncRead + AsyncSeek + Unpin>(
    reader: &mut DiscReader<R>,
    pos: SeekFrom,
    buf: &mut [u8],
) -> Result<(), GeckoFSError> {
    reader.seek(pos).await?;
    Ok(reader.read_exact(buf).await?)
}

impl<R> GeckoFS<R>
where
    R: AsyncRead + AsyncSeek + Clone + Unpin + 'static,
{
    pub async fn parse(mut reader: DiscReader<R>) -> Result<Self, GeckoFSError> {
        let mut arena: Arena<FsNode<R>> = Arena::new();
        let root = arena.new_node(FsNode::Directory { name: "".into() });
        let sys = arena.new_node(FsNode::Directory {
            name: "&&systemdata".into(),
        });
        {
            let is_wii = reader.get_type() == DiscType::Wii;
            crate::debug!(
                "{}",
                if is_wii {
                    "The disc is a Wii game"
                } else {
                    "The disc is NOT a Wii game"
                }
            );
            let mut buf = [0u8; 4];
            read_exact_async(
                &mut reader,
                SeekFrom::Start(consts::OFFSET_FST_OFFSET as u64),
                &mut buf,
            )
            .await?;
            let fst_offset = (BE::read_u32(&buf[..]) as u64) << (if is_wii { 2 } else { 0 });
            read_exact_async(&mut reader, SeekFrom::Start(fst_offset + 8), &mut buf).await?;
            let num_entries = BE::read_u32(&buf[..]) as usize;
            let mut fst_list_buf = vec![0u8; num_entries * FstEntry::BLOCK_SIZE];
            read_exact_async(&mut reader, SeekFrom::Start(fst_offset), &mut fst_list_buf).await?;
            let string_table_offset = num_entries as u64 * FstEntry::BLOCK_SIZE as u64;

            read_exact_async(
                &mut reader,
                SeekFrom::Start(consts::OFFSET_FST_SIZE as u64),
                &mut buf,
            )
            .await?;
            let fst_size = (BE::read_u32(&buf) as u64) << (if is_wii { 2 } else { 0 });
            let mut str_tbl_buf = vec![0u8; (fst_size - string_table_offset) as usize];
            read_exact_async(
                &mut reader,
                SeekFrom::Start(string_table_offset + fst_offset),
                &mut str_tbl_buf,
            )
            .await?;

            crate::debug!(
                "#fst enties: {}; #names: {}",
                num_entries,
                str_tbl_buf.split(|b| *b == 0).count()
            );

            let fst_entries: Vec<FstNode> = {
                #[cfg(feature = "parallel")]
                let chunks = fst_list_buf.par_chunks_exact(FstEntry::BLOCK_SIZE);
                #[cfg(not(feature = "parallel"))]
                let chunks = fst_list_buf.chunks_exact(FstEntry::BLOCK_SIZE);
                chunks
            }
            .map(|entry_buf| {
                let entry = FstEntry::try_from(entry_buf).unwrap();
                let mut node = FstNode::from_fstnode(&entry, &str_tbl_buf).unwrap();

                if is_wii && let FstNode::File { file_offset, .. } = &mut node {
                    *file_offset <<= 2;
                }

                node
            })
            .collect();

            read_exact_async(
                &mut reader,
                SeekFrom::Start(consts::OFFSET_DOL_OFFSET as u64),
                &mut buf,
            )
            .await?;
            let dol_offset = (BE::read_u32(&buf) as u64) << (if is_wii { 2 } else { 0 });
            crate::debug!(
                "fst_size: 0x{:08X}; fst entries list size: 0x{:08X}",
                fst_size,
                num_entries * FstEntry::BLOCK_SIZE
            );

            let _ = FsNode::new_file(
                sys,
                FileDataSource::Reader {
                    reader: reader.clone(),
                    fst: FstNode::File {
                        relative_file_name: "iso.hdr".to_owned(),
                        file_offset: 0,
                        file_size: consts::HEADER_LENGTH,
                    },
                },
                &mut arena,
            );
            let _ = FsNode::new_file(
                sys,
                FileDataSource::Reader {
                    reader: reader.clone(),
                    fst: FstNode::File {
                        relative_file_name: "AppLoader.ldr".to_owned(),
                        file_offset: consts::HEADER_LENGTH as u64,
                        file_size: (dol_offset - consts::HEADER_LENGTH as u64) as usize,
                    },
                },
                &mut arena,
            );
            let _ = FsNode::new_file(
                sys,
                FileDataSource::Reader {
                    reader: reader.clone(),
                    fst: FstNode::File {
                        relative_file_name: "Start.dol".to_owned(),
                        file_offset: dol_offset,
                        file_size: (fst_offset - dol_offset) as usize,
                    },
                },
                &mut arena,
            );
            let _ = FsNode::new_file(
                sys,
                FileDataSource::Reader {
                    reader: reader.clone(),
                    fst: FstNode::File {
                        relative_file_name: "Game.toc".to_owned(),
                        file_offset: fst_offset,
                        file_size: fst_size as usize,
                    },
                },
                &mut arena,
            );

            let mut count = 1;
            while count < num_entries {
                let _ = GeckoFS::get_dir_structure_recursive(
                    &mut count,
                    &fst_entries,
                    root,
                    &mut arena,
                    &reader,
                );
                count += 1;
            }
        }
        crate::debug!("{} children", root.children(&arena).count());
        Ok(Self { root, sys, arena })
    }

    pub async fn serialize<W>(&mut self, writer: &mut DiscWriter<W>) -> Result<(), GeckoFSError>
    where
        W: AsyncWrite + AsyncSeek + Unpin,
    {
        crate::debug!("Serializing the FileSystem");
        let is_wii = writer.get_type() == DiscType::Wii;
        let mut pos: u64 = 0;
        let header_size = self.get_file_len(self.sys, "iso.hdr")? as u64;
        let apploader_size = self.get_file_len(self.sys, "AppLoader.ldr")? as u64;

        // Calculate dynamic offsets
        let dol_offset_raw = header_size + apploader_size;
        let dol_offset = align_addr(dol_offset_raw, consts::DOL_ALIGNMENT_BIT);
        let dol_padding_size = dol_offset - dol_offset_raw;
        let dol_size = self.get_file_len(self.sys, "Start.dol")? as u64;

        let fst_list_offset_raw = dol_offset + dol_size;
        let fst_list_offset = align_addr(fst_list_offset_raw, consts::FST_ALIGNMENT_BIT);
        let fst_list_padding_size = fst_list_offset - fst_list_offset_raw;

        let fst_len = self.visitor_fst_len(0, self.root)? - 1;

        // Wii FST sizes are stored as 32-bit words (see encode_fst_size). For the
        // size field to round-trip without Dolphin over/under-reading the FST, the
        // FST's length must be a multiple of 4. We align it here and pad the written
        // table below to match.
        let fst_len_aligned = (fst_len + 3) & !3u64;

        let d = [
            (dol_offset >> if is_wii { 2u8 } else { 0u8 }) as u32,
            (fst_list_offset >> if is_wii { 2u8 } else { 0u8 }) as u32,
            super::encode_fst_size(fst_len_aligned, is_wii),
            super::encode_fst_size(fst_len_aligned, is_wii),
        ];
        let mut b = vec![0u8; 0x10];
        BE::write_u32_into(&d, &mut b);

        // Write header and app loader
        let mut buf = Vec::new();
        self.read_file_to_end(self.sys, "iso.hdr", &mut buf).await?;
        writer.write_all(&buf[..OFFSET_DOL_OFFSET]).await?;
        writer.write_all(&b).await?;
        writer.write_all(&buf[OFFSET_DOL_OFFSET + 0x10..]).await?;
        pos += buf.len().to_u64().ok_or(GeckoFSError::BufferTooLarge)?;
        buf.clear();
        self.read_file_to_end(self.sys, "AppLoader.ldr", &mut buf)
            .await?;
        writer.write_all(&buf).await?;
        pos += buf.len().to_u64().ok_or(GeckoFSError::BufferTooLarge)?;
        writer
            .write_all(&vec![0u8; dol_padding_size as usize])
            .await?;
        pos += dol_padding_size;

        buf.clear();
        self.read_file_to_end(self.sys, "Start.dol", &mut buf)
            .await?;
        writer.write_all(&buf).await?;
        pos += buf.len().to_u64().ok_or(GeckoFSError::BufferTooLarge)?;
        writer
            .write_all(&vec![0u8; fst_list_padding_size as usize])
            .await?;
        pos += fst_list_padding_size;

        let mut output_fst = vec![FstEntry::new_directory(0, 0, 0)?];
        let mut fst_name_bank = Vec::new();
        let mut files = Vec::new();

        let mut offset = fst_list_offset + fst_len;
        // for node in self.root_mut().iter_mut() {
        //     let l = 0;
        //     GeckoFS::visitor_fst_entries(
        //         node.as_mut(),
        //         &mut output_fst,
        //         &mut files,
        //         &mut fst_name_bank,
        //         l,
        //         &mut offset,
        //         is_wii,
        //     )?;
        // }
        let mut cur_parent_dir_index: Vec<u64> = vec![0];
        for edge in self.root.traverse(&self.arena).filter(|e| {
            self.root
                != match e {
                    indextree::NodeEdge::Start(node_id) => *node_id,
                    indextree::NodeEdge::End(node_id) => *node_id,
                }
        }) {
            match edge {
                indextree::NodeEdge::Start(node_id) => {
                    match self.arena.get(node_id).map(|n| n.get()) {
                        None => return Err(GeckoFSError::InvalidNode),
                        Some(FsNode::File { file }) => {
                            let pos = align_addr(offset, 5);
                            offset = pos;

                            let fst_entry = FstEntry::new_file(
                                fst_name_bank.len() as u32,
                                pos as u64,
                                file.len()? as u32,
                                is_wii,
                            )?;

                            fst_name_bank.extend_from_slice(file.name().as_bytes());
                            fst_name_bank.push(0);

                            offset += file.len()? as u64;
                            offset = align_addr(offset, 2);

                            output_fst.push(fst_entry);
                            files.push((file.clone(), pos));
                        }
                        Some(FsNode::Directory { name }) => {
                            let fst_entry = FstEntry::new_directory(
                                fst_name_bank.len() as u32,
                                *cur_parent_dir_index
                                    .last()
                                    .ok_or(GeckoFSError::DirStackUnderflow)?
                                    as u64,
                                0,
                            )?;

                            fst_name_bank.extend_from_slice(name.as_bytes());
                            fst_name_bank.push(0);

                            let this_dir_index = output_fst.len();

                            output_fst.push(fst_entry);
                            cur_parent_dir_index.push(this_dir_index as u64);
                        }
                    }
                }
                indextree::NodeEdge::End(node_id) => {
                    match self.arena.get(node_id).map(|n| n.get()) {
                        None => return Err(GeckoFSError::InvalidNode),
                        Some(FsNode::File { .. }) => {}
                        Some(FsNode::Directory { .. }) => {
                            let this_dir_index = cur_parent_dir_index
                                .pop()
                                .ok_or(GeckoFSError::DirStackUnderflow)?
                                as usize;
                            let next_dir_index = output_fst.len() as u32;
                            output_fst[this_dir_index].set_file_size_next_dir_index(next_dir_index);
                        }
                    }
                }
            }
        }
        {
            let next_dir_index = output_fst.len();
            output_fst[0].set_file_size_next_dir_index(next_dir_index as u32);
        }
        crate::debug!("output_fst size = {}", output_fst.len());
        crate::debug!("first fst_name entry = {}", fst_name_bank[0]);
        #[cfg(feature = "progress")]
        let write_total_size: u64 = output_fst
            .iter()
            .filter_map(|f| {
                if let FstNodeType::File = f.get_node_type() {
                    Some(f.get_file_size_next_dir_index() as u64)
                } else {
                    None
                }
            })
            .sum();

        for entry in output_fst {
            writer.write_all(&entry.pack()).await?;
            pos += FstEntry::BLOCK_SIZE as u64;
        }

        writer.write_all(&fst_name_bank).await?;
        pos += fst_name_bank
            .len()
            .to_u64()
            .ok_or(GeckoFSError::BufferTooLarge)?;

        // Pad the FST table up to the aligned length we advertised in the header so
        // the Wii word-address round-trip is exact and the FST ends on a null byte.
        if fst_len_aligned > fst_len {
            let pad = (fst_len_aligned - fst_len) as usize;
            writer.write_all(&vec![0u8; pad]).await?;
            pos += pad as u64;
        }

        // Traverse the root directory tree to write all the files in order
        #[cfg(feature = "progress")]
        if let Ok(mut updater) = UPDATER.lock() {
            updater.set_len(write_total_size as usize)?;
            updater.set_title("Writing virtual FileSystem")?;
            updater.set_type(crate::update::UpdaterType::Progress)?;
        }
        let mut offset = pos;
        #[cfg(feature = "progress")]
        let mut inc_buffer = 0usize;
        for (mut file, file_offset) in files {
            #[cfg(feature = "progress")]
            if let Ok(mut updater) = UPDATER.try_lock() {
                updater.set_message(format!(
                    "{:<32.32} ({:>8})",
                    file.name(),
                    human_bytes::human_bytes(file.len()? as f64)
                ))?;
            }
            let padding_size = (file_offset - offset) as usize;
            writer.write_all(&vec![0u8; padding_size]).await?;
            // Copy the file from the FileSystem to the Writer.
            // async_std::io::copy(file, writer).await?; // way too slow
            let mut rem = file.len()?;
            file.seek(SeekFrom::Start(0)).await?;
            loop {
                if rem == 0 {
                    break;
                }
                let transfer_size = std::cmp::min(rem, 1024 * 1024);
                let mut buf = vec![0u8; transfer_size];
                file.read_exact(&mut buf).await?;
                writer.write_all(&buf).await?;
                rem -= transfer_size;
                #[cfg(feature = "progress")]
                match UPDATER.try_lock() {
                    Ok(mut updater) => {
                        updater.increment(transfer_size + inc_buffer)?;
                        inc_buffer = 0;
                    }
                    Err(TryLockError::WouldBlock) => {
                        inc_buffer += transfer_size;
                    }
                    _ => (),
                }
            }
            offset = file_offset + file.len()? as u64;
        }

        // The disc apparently needs to be aligned to 8 bits
        let mut new_offset = align_addr(offset, 8);
        if (new_offset - offset) < 0x20 {
            new_offset = align_addr((offset) + 0x20, 8);
        }
        let padding_size = (new_offset - offset) as usize;
        writer.write_all(&vec![0u8; padding_size]).await?;
        //offset += padding_size; // Unececssary, but kept for clarity

        #[cfg(feature = "progress")]
        if let Ok(mut updater) = UPDATER.lock() {
            updater.finish()?;
        }

        writer.flush().await?;
        writer.close().await?;

        Ok(())
    }
}

impl<R> GeckoFS<R>
where
    R: std::io::Read + std::io::Seek + Clone,
{
    #[doc = r"Utility function to read the disc."]
    fn read_exact(
        reader: &mut DiscReader<R>,
        pos: SeekFrom,
        buf: &mut [u8],
    ) -> Result<(), GeckoFSError> {
        reader.seek(pos)?;
        Ok(std::io::Read::read_exact(reader, buf)?)
    }

    pub fn parse_sync(mut reader: DiscReader<R>) -> Result<Self, GeckoFSError> {
        let mut arena: Arena<FsNode<R>> = Arena::new();
        let root = arena.new_node(FsNode::Directory { name: "".into() });
        let sys = arena.new_node(FsNode::Directory {
            name: "&&systemdata".into(),
        });
        {
            let is_wii = reader.get_type() == DiscType::Wii;
            crate::debug!(
                "{}",
                if is_wii {
                    "The disc is a Wii game"
                } else {
                    "The disc is NOT a Wii game"
                }
            );
            let mut buf = [0u8; 4];
            GeckoFS::read_exact(
                &mut reader,
                SeekFrom::Start(consts::OFFSET_FST_OFFSET as u64),
                &mut buf,
            )?;
            let fst_offset = (BE::read_u32(&buf[..]) as u64) << (if is_wii { 2 } else { 0 });
            GeckoFS::read_exact(&mut reader, SeekFrom::Start(fst_offset + 8), &mut buf)?;
            let num_entries = BE::read_u32(&buf[..]) as usize;
            let mut fst_list_buf = vec![0u8; num_entries * FstEntry::BLOCK_SIZE];
            GeckoFS::read_exact(&mut reader, SeekFrom::Start(fst_offset), &mut fst_list_buf)?;
            let string_table_offset = num_entries as u64 * FstEntry::BLOCK_SIZE as u64;

            GeckoFS::read_exact(
                &mut reader,
                SeekFrom::Start(consts::OFFSET_FST_SIZE as u64),
                &mut buf,
            )?;
            let fst_size = (BE::read_u32(&buf) as u64) << (if is_wii { 2 } else { 0 });
            let mut str_tbl_buf = vec![0u8; (fst_size - string_table_offset) as usize];
            GeckoFS::read_exact(
                &mut reader,
                SeekFrom::Start(string_table_offset + fst_offset),
                &mut str_tbl_buf,
            )?;

            crate::debug!(
                "#fst enties: {}; #names: {}",
                num_entries,
                str_tbl_buf.split(|b| *b == 0).count()
            );

            let fst_entries: Vec<FstNode> = {
                #[cfg(feature = "parallel")]
                let chunks = fst_list_buf.par_chunks_exact(FstEntry::BLOCK_SIZE);
                #[cfg(not(feature = "parallel"))]
                let chunks = fst_list_buf.chunks_exact(FstEntry::BLOCK_SIZE);
                chunks
            }
            .map(|entry_buf| {
                let entry = FstEntry::try_from(entry_buf).unwrap();
                let mut node = FstNode::from_fstnode(&entry, &str_tbl_buf).unwrap();

                if is_wii && let FstNode::File { file_offset, .. } = &mut node {
                    *file_offset <<= 2;
                }

                node
            })
            .collect();

            GeckoFS::read_exact(
                &mut reader,
                SeekFrom::Start(consts::OFFSET_DOL_OFFSET as u64),
                &mut buf,
            )?;
            let dol_offset = (BE::read_u32(&buf) as u64) << (if is_wii { 2 } else { 0 });
            crate::debug!(
                "fst_size: 0x{:08X}; fst entries list size: 0x{:08X}",
                fst_size,
                num_entries * FstEntry::BLOCK_SIZE
            );

            let _ = FsNode::new_file(
                sys,
                FileDataSource::Reader {
                    reader: reader.clone(),
                    fst: FstNode::File {
                        relative_file_name: "iso.hdr".to_owned(),
                        file_offset: 0,
                        file_size: consts::HEADER_LENGTH,
                    },
                },
                &mut arena,
            );
            let _ = FsNode::new_file(
                sys,
                FileDataSource::Reader {
                    reader: reader.clone(),
                    fst: FstNode::File {
                        relative_file_name: "AppLoader.ldr".to_owned(),
                        file_offset: consts::HEADER_LENGTH as u64,
                        file_size: (dol_offset - consts::HEADER_LENGTH as u64) as usize,
                    },
                },
                &mut arena,
            );
            let _ = FsNode::new_file(
                sys,
                FileDataSource::Reader {
                    reader: reader.clone(),
                    fst: FstNode::File {
                        relative_file_name: "Start.dol".to_owned(),
                        file_offset: dol_offset,
                        file_size: (fst_offset - dol_offset) as usize,
                    },
                },
                &mut arena,
            );
            let _ = FsNode::new_file(
                sys,
                FileDataSource::Reader {
                    reader: reader.clone(),
                    fst: FstNode::File {
                        relative_file_name: "Game.toc".to_owned(),
                        file_offset: fst_offset,
                        file_size: fst_size as usize,
                    },
                },
                &mut arena,
            );

            let mut count = 1;
            while count < num_entries {
                let _ = GeckoFS::get_dir_structure_recursive(
                    &mut count,
                    &fst_entries,
                    root,
                    &mut arena,
                    &reader,
                );
                count += 1;
            }
        }
        crate::debug!("{} children", root.children(&arena).count());
        Ok(Self { root, sys, arena })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_sample_fs() -> GeckoFS<()> {
        let mut arena: Arena<FsNode<()>> = Arena::new();
        let root = arena.new_node(FsNode::Directory { name: "".into() });
        let sys = arena.new_node(FsNode::Directory { name: "&&systemdata".into() });

        let file =
            |name: &str, data: &[u8]| FileDataSource::Box { data: data.into(), name: name.into() };
        FsNode::new_file(root, file("top.bin", &[0u8; 4]), &mut arena).unwrap();
        let dir_a = FsNode::new_directory(root, "dirA", &mut arena).unwrap();
        FsNode::new_file(dir_a, file("a1.bin", &[0u8; 4]), &mut arena).unwrap();
        let dir_sub = FsNode::new_directory(dir_a, "sub", &mut arena).unwrap();
        FsNode::new_file(dir_sub, file("s1.bin", &[0u8; 4]), &mut arena).unwrap();
        FsNode::new_file(dir_a, file("a2.bin", &[0u8; 4]), &mut arena).unwrap();
        let dir_b = FsNode::new_directory(root, "dirB", &mut arena).unwrap();
        FsNode::new_file(dir_b, file("b1.bin", &[0u8; 4]), &mut arena).unwrap();
        let _ = sys;

        GeckoFS { root, sys, arena }
    }

    // Mirrors the FST-building loop in GeckoFS::serialize (tree.rs). Returns the
    // packed entries plus the name table.
    fn build_fst<R>(fs: &GeckoFS<R>, is_wii: bool) -> (Vec<FstEntry>, Vec<u8>) {
        let mut output_fst = vec![FstEntry::new_directory(0, 0, 0).unwrap()];
        let mut fst_name_bank = Vec::new();
        let mut offset = 0x100u64;
        let mut cur_parent_dir_index: Vec<u64> = vec![0];
        for edge in fs.root.traverse(&fs.arena).filter(|e| {
            fs.root
                != match e {
                    indextree::NodeEdge::Start(id) => *id,
                    indextree::NodeEdge::End(id) => *id,
                }
        }) {
            match edge {
                indextree::NodeEdge::Start(node_id) => {
                    match fs.arena.get(node_id).map(|n| n.get()) {
                        Some(FsNode::File { file }) => {
                            let pos = align_addr(offset, 5);
                            offset = pos;
                            output_fst.push(
                                FstEntry::new_file(
                                    fst_name_bank.len() as u32,
                                    pos as u64,
                                    file.len().unwrap() as u32,
                                    is_wii,
                                )
                                .unwrap(),
                            );
                            fst_name_bank.extend_from_slice(file.name().as_bytes());
                            fst_name_bank.push(0);
                            offset += file.len().unwrap() as u64;
                            offset = align_addr(offset, 2);
                        }
                        Some(FsNode::Directory { name }) => {
                            let this_idx = output_fst.len();
                            output_fst.push(
                                FstEntry::new_directory(
                                    fst_name_bank.len() as u32,
                                    *cur_parent_dir_index.last().unwrap() as u64,
                                    0,
                                )
                                .unwrap(),
                            );
                            fst_name_bank.extend_from_slice(name.as_bytes());
                            fst_name_bank.push(0);
                            cur_parent_dir_index.push(this_idx as u64);
                        }
                        _ => unreachable!(),
                    }
                }
                indextree::NodeEdge::End(node_id) => {
                    match fs.arena.get(node_id).map(|n| n.get()) {
                        Some(FsNode::File { .. }) => {}
                        Some(FsNode::Directory { .. }) => {
                            let this_idx = cur_parent_dir_index.pop().unwrap() as usize;
                            let next = output_fst.len() as u32;
                            output_fst[this_idx].set_file_size_next_dir_index(next);
                        }
                        _ => unreachable!(),
                    }
                }
            }
        }
        let n = output_fst.len() as u32;
        output_fst[0].set_file_size_next_dir_index(n);
        (output_fst, fst_name_bank)
    }

    // Faithful port of Dolphin's FileSystemGCWii::IsValid / constructor checks.
    fn dolphin_fst_is_valid(
        bytes: &[u8],
        header_word_size: u32,
        offset_shift: u8,
        num_entries: usize,
    ) -> bool {
        let get = |idx: usize, off: usize| -> u32 {
            let base = idx * 12 + off;
            u32::from_be_bytes([bytes[base], bytes[base + 1], bytes[base + 2], bytes[base + 3]])
        };
        let name_offset = |idx: usize| (get(idx, 0) & 0x00FFFFFF) as u64 + 12 * num_entries as u64;
        let is_dir = |idx: usize| (get(idx, 0) & 0xFF000000) != 0;

        // Recover the byte FST size exactly as Dolphin does (stored << offset_shift).
        let fst_size = (header_word_size as u64) << offset_shift;
        if fst_size < 12 {
            return false;
        }
        if (12 * num_entries) as u64 > fst_size {
            return false; // "too many entries for its size"
        }
        if bytes[fst_size as usize - 1] != 0 {
            return false; // "does not end with a null byte"
        }
        if !is_dir(0) {
            return false; // root must be a directory
        }

        fn recurse(
            bytes: &[u8],
            get: &dyn Fn(usize, usize) -> u32,
            name_offset: &dyn Fn(usize) -> u64,
            is_dir: &dyn Fn(usize) -> bool,
            fst_size: u64,
            total: usize,
            idx: usize,
            parent: usize,
        ) -> bool {
            if name_offset(idx) >= fst_size {
                return false;
            }
            if is_dir(idx) {
                if get(idx, 4) as usize != parent {
                    return false; // "incorrect parent offset"
                }
                let size = get(idx, 8) as usize;
                if size <= idx {
                    return false; // "impossibly small directory size"
                }
                if size > total {
                    return false;
                }
                let mut c = idx + 1;
                while c < size {
                    if !recurse(bytes, get, name_offset, is_dir, fst_size, total, c, idx) {
                        return false;
                    }
                    // Match Dolphin's `++` iterator, which skips entire subtrees.
                    c = if is_dir(c) { get(c, 8) as usize } else { c + 1 };
                }
            }
            true
        }

        recurse(
            bytes,
            &get,
            &name_offset,
            &is_dir,
            fst_size,
            num_entries,
            0,
            0,
        )
    }

    #[test]
    fn serialized_fst_is_valid_under_dolphin_rules() {
        let fs = build_sample_fs();
        let (entries, name_bank) = build_fst(&fs, true);

        let num_entries = entries.len();
        let fst_len = (num_entries * 12) + name_bank.len();
        let aligned = (fst_len + 3) & !3;
        let mut bytes: Vec<u8> = Vec::new();
        for e in &entries {
            bytes.extend_from_slice(&e.pack());
        }
        bytes.extend_from_slice(&name_bank);
        // Pad exactly like the serializer now does.
        bytes.resize(aligned, 0);

        // Without the new alignment fix, a non-word-multiple FST would make Dolphin
        // read `ceil(fst_len/4)*4` bytes and hit a non-null final byte. Check both.
        let header_word_kind = (aligned / 4) as u32;
        assert_eq!((header_word_kind as u64) << 2, aligned as u64);
        assert!(
            dolphin_fst_is_valid(&bytes, header_word_kind, 2, num_entries),
            "Wii FST should pass Dolphin validation"
        );
    }
}

