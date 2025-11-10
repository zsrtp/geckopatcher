use crate::{
    crypto::Unpackable,
    iso::{
        FstEntry, FstNode, consts,
        disc::{DiscType},
        read::DiscReader,
    },
    vfs::data_source::FileDataSource,
};
use byteorder::{BE, ByteOrder};
use futures::{AsyncRead, AsyncReadExt, AsyncSeek, AsyncSeekExt, io};
use indextree::{Arena, NodeId};
use rayon::iter::ParallelIterator;
#[cfg(feature = "parallel")]
use rayon::slice::ParallelSlice;
use std::{
    io::{ErrorKind, Seek, SeekFrom}, path::{Path, PathBuf}, sync::{Arc, Mutex}, task::{Context, Poll}
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

#[derive(Debug)]
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

    fn name(&self) -> String {
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
    NodeIsNotDirectory(String, PathBuf),
}

#[derive(Debug)]
pub enum FsNode<R> {
    Directory { name: String },
    File { file: FsFile<R> },
}

impl<R> FsNode<R> {
    fn assert_dir(node: NodeId, arena: &Arena<FsNode<R>>) -> Result<(), FsNodeError> {
        // First, check the parent is a directory
        if let Some(n) = arena.get(node)
            && let FsNode::File { .. } = n.get()
        {
            let path = node
                .ancestors(&arena)
                .filter_map(|n| arena.get(n))
                .map(|n| n.get().name())
                .collect::<Vec<_>>()
                .iter()
                .rev()
                .collect();
            return Err(FsNodeError::NodeIsNotDirectory(n.get().name(), path));
        }
        Ok(())
    }

    pub fn new_file(
        parent: NodeId,
        data: FileDataSource<R>,
        arena: &mut Arena<FsNode<R>>,
    ) -> Result<NodeId, FsNodeError> {
        // First, check the parent is a directory
        Self::assert_dir(parent, &arena)?;
        let id = parent.append_value(
            Self::File {
                file: FsFile::new(data),
            },
            arena,
        );
        Ok(id)
    }

    pub fn new_directory<S: Into<String>>(
        parent: NodeId,
        name: S,
        arena: &mut Arena<FsNode<R>>,
    ) -> Result<NodeId, FsNodeError> {
        // First, check the parent is a directory
        Self::assert_dir(parent, &arena)?;
        let id = parent.append_value(Self::Directory { name: name.into() }, arena);
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
    #[error("Error while reading the disc")]
    ReadError(#[from] std::io::Error),
    #[error(transparent)]
    FsNodeError(#[from] FsNodeError),
}

pub struct GeckoFS<R> {
    pub root: NodeId,
    pub sys: NodeId,
    arena: Arena<FsNode<R>>,
}

impl<R> GeckoFS<R>
where
    R: AsyncRead + AsyncSeek + Clone + Unpin + 'static,
{
    #[doc = r"Utility function to read the disc."]
    async fn read_exact(
        reader: &mut DiscReader<R>,
        pos: SeekFrom,
        buf: &mut [u8],
    ) -> Result<(), GeckoFSError> {
        reader.seek(pos).await?;
        Ok(reader.read_exact(buf).await?)
    }

    fn get_dir_structure_recursive(
        cur_index: &mut usize,
        fst: &Vec<FstNode>,
        parent_dir: NodeId,
        arena: &mut Arena<FsNode<R>>,
        reader: &DiscReader<R>,
    ) -> Result<(), GeckoFSError> {
        let entry = &fst[*cur_index];

        match entry.clone() {
            FstNode::Directory {
                relative_file_name,
                parent_dir: _,
                next_dir_index,
            } => {
                let dir = FsNode::new_directory(parent_dir, relative_file_name.clone(), arena)?;

                while *cur_index < next_dir_index - 1 {
                    *cur_index += 1;
                    GeckoFS::get_dir_structure_recursive(cur_index, fst, dir, arena, reader)?;
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
            GeckoFS::read_exact(
                &mut reader,
                SeekFrom::Start(consts::OFFSET_FST_OFFSET as u64),
                &mut buf,
            )
            .await?;
            let fst_offset = (BE::read_u32(&buf[..]) as u64) << (if is_wii { 2 } else { 0 });
            GeckoFS::read_exact(&mut reader, SeekFrom::Start(fst_offset + 8), &mut buf).await?;
            let num_entries = BE::read_u32(&buf[..]) as usize;
            let mut fst_list_buf = vec![0u8; num_entries * FstEntry::BLOCK_SIZE];
            GeckoFS::read_exact(&mut reader, SeekFrom::Start(fst_offset), &mut fst_list_buf)
                .await?;
            let string_table_offset = num_entries as u64 * FstEntry::BLOCK_SIZE as u64;

            GeckoFS::read_exact(
                &mut reader,
                SeekFrom::Start(consts::OFFSET_FST_SIZE as u64),
                &mut buf,
            )
            .await?;
            let fst_size = (BE::read_u32(&buf) as u64) << (if is_wii { 2 } else { 0 });
            let mut str_tbl_buf = vec![0u8; (fst_size - string_table_offset) as usize];
            GeckoFS::read_exact(
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

                if is_wii {
                    match &mut node {
                        FstNode::File { file_offset, .. } => {
                            *file_offset <<= 2;
                        }
                        FstNode::Directory { parent_dir, .. } => {
                            *parent_dir <<= 2;
                        }
                    }
                }

                node
            })
            .collect();

            GeckoFS::read_exact(
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

    /*/
    pub async fn serialize<W>(&mut self, writer: &mut DiscWriter<W>) -> Result<()>
    where
        W: AsyncWrite + AsyncSeek + Unpin,
    {
        crate::debug!("Serializing the FileSystem");
        let is_wii = writer.get_type() == DiscType::Wii;
        let mut pos: u64 = 0;
        let header_size = self.iter_dfs(self.sys).find(|c| c.eq("iso.hdr"))
            .get_file("iso.hdr")?
            .len()? as u64;
        let apploader_size = self.sys().get_file("AppLoader.ldr")?.len()? as u64;

        // Calculate dynamic offsets
        let dol_offset_raw = header_size + apploader_size;
        let dol_offset = align_addr(dol_offset_raw, consts::DOL_ALIGNMENT_BIT);
        let dol_padding_size = dol_offset - dol_offset_raw;
        let dol_size = self.sys().get_file("Start.dol")?.len()? as u64;

        let fst_list_offset_raw = dol_offset + dol_size;
        let fst_list_offset = align_addr(fst_list_offset_raw, consts::FST_ALIGNMENT_BIT);
        let fst_list_padding_size = fst_list_offset - fst_list_offset_raw;

        let fst_len = GeckoFS::visitor_fst_len(0, &self.root) - 1;

        let d = [
            (dol_offset >> if is_wii { 2u8 } else { 0u8 }) as u32,
            (fst_list_offset >> if is_wii { 2u8 } else { 0u8 }) as u32,
            fst_len as u32,
            fst_len as u32,
        ];
        let mut b = vec![0u8; 0x10];
        BE::write_u32_into(&d, &mut b);

        // Write header and app loader
        let mut buf = Vec::new();
        self.sys_mut()
            .get_file_mut("iso.hdr")?
            .read_to_end(&mut buf)
            .await?;
        writer.write_all(&buf[..OFFSET_DOL_OFFSET]).await?;
        writer.write_all(&b).await?;
        writer.write_all(&buf[OFFSET_DOL_OFFSET + 0x10..]).await?;
        pos += buf.len().to_u64().ok_or(eyre::eyre!("Buffer too large"))?;
        buf.clear();
        self.sys_mut()
            .get_file_mut("AppLoader.ldr")?
            .read_to_end(&mut buf)
            .await?;
        writer.write_all(&buf).await?;
        pos += buf.len().to_u64().ok_or(eyre::eyre!("Buffer too large"))?;
        writer
            .write_all(&vec![0u8; dol_padding_size as usize])
            .await?;
        pos += dol_padding_size;

        buf.clear();
        self.sys_mut()
            .get_file_mut("Start.dol")?
            .read_to_end(&mut buf)
            .await?;
        writer.write_all(&buf).await?;
        pos += buf.len().to_u64().ok_or(eyre::eyre!("Buffer too large"))?;
        writer
            .write_all(&vec![0u8; fst_list_padding_size as usize])
            .await?;
        pos += fst_list_padding_size;

        let mut output_fst = vec![FstEntry::new_directory(0, 0, 0, is_wii)?];
        let mut fst_name_bank = Vec::new();
        let mut files = Vec::new();

        let mut offset = fst_list_offset + fst_len;
        for node in self.root_mut().iter_mut() {
            let l = 0;
            GeckoFS::visitor_fst_entries(
                node.as_mut(),
                &mut output_fst,
                &mut files,
                &mut fst_name_bank,
                l,
                &mut offset,
                is_wii,
            )?;
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
            .ok_or(eyre::eyre!("Buffer too large"))?;

        // Traverse the root directory tree to write all the files in order
        #[cfg(feature = "progress")]
        if let Ok(mut updater) = UPDATER.lock() {
            updater.set_len(write_total_size as usize)?;
            updater.set_title("Writing virtual FileSystem".to_string())?;
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
                    human_bytes(file.len()? as f64)
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
    }// */

    pub fn get_file_ref<P: AsRef<Path>>(&self, root: NodeId, path: P) -> Option<&FsNode<R>> {
        if path.as_ref().iter().any(|c| c.to_str().is_none()) {
            return None;
        }
        path.as_ref().iter()
            .filter_map(|c| c.to_str())
            .try_fold(root, |node, c| {
                node.children(&self.arena).find(|ch| {
                    self.arena
                        .get(*ch)
                        .map(|n| n.get())
                        .is_some_and(|n| n.name() == c)
                })
            })
            .and_then(|n| self.arena.get(n))
            .map(indextree::Node::get)
            .filter(|n| n.is_file())
    }

    pub fn get_file_mut<P: AsRef<Path>>(&mut self, root: NodeId, path: P) -> Option<&mut FsNode<R>> {
        if path.as_ref().iter().any(|c| c.to_str().is_none()) {
            return None;
        }
        path.as_ref().iter()
            .filter_map(|c| c.to_str())
            .try_fold(root, |node, c| {
                node.children(&self.arena).find(|ch| {
                    self.arena
                        .get(*ch)
                        .map(|n| n.get())
                        .is_some_and(|n| n.name() == c)
                })
            })
            .and_then(|n| self.arena.get_mut(n))
            .map(indextree::Node::get_mut)
            .filter(|n| n.is_file())
    }

    pub fn get_dir_ref<P: AsRef<Path>>(&self, root: NodeId, path: P) -> Option<&FsNode<R>> {
        if path.as_ref().iter().any(|c| c.to_str().is_none()) {
            return None;
        }
        path.as_ref().iter()
            .filter_map(|c| c.to_str())
            .try_fold(root, |node, c| {
                node.children(&self.arena).find(|ch| {
                    self.arena
                        .get(*ch)
                        .map(|n| n.get())
                        .is_some_and(|n| n.name() == c)
                })
            })
            .and_then(|n| self.arena.get(n))
            .map(indextree::Node::get)
            .filter(|n| n.is_dir())
    }

    pub fn get_dir_mut<P: AsRef<Path>>(&mut self, root: NodeId, path: P) -> Option<&mut FsNode<R>> {
        if path.as_ref().iter().any(|c| c.to_str().is_none()) {
            return None;
        }
        path.as_ref().iter()
            .filter_map(|c| c.to_str())
            .try_fold(root, |node, c| {
                node.children(&self.arena).find(|ch| {
                    self.arena
                        .get(*ch)
                        .map(|n| n.get())
                        .is_some_and(|n| n.name() == c)
                })
            })
            .and_then(|n| self.arena.get_mut(n))
            .map(indextree::Node::get_mut)
            .filter(|n| n.is_dir())
    }

    #[doc = "Iterates through all entries under the provided root node using Depth First Search"]
    pub fn iter_dfs(&self, root: NodeId) -> impl Iterator<Item = PathBuf> {
        let root_clone = root.clone();
        root.descendants(&self.arena).skip(1).map(move |d| {
            d.ancestors(&self.arena)
                .filter(|a| *a != root_clone)
                .filter_map(|a| self.arena.get(a))
                .map(|a| a.get().name())
                .collect::<Vec<_>>()
                .iter()
                .rev()
                .collect()
        })
    }
}
