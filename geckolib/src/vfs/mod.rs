use crate::iso::read::{DiscReader, DiscType};
use crate::iso::{consts, FstEntry, FstNodeType};
use async_std::io::prelude::SeekExt;
use async_std::io::{Read as AsyncRead, ReadExt, Seek as AsyncSeek};
use async_std::sync::{Arc, Mutex};
use byteorder::{ByteOrder, BE};
use eyre::Result;
use std::io::{Error, SeekFrom};
use std::ops::DerefMut;
use std::task::{Context, Poll};

pub trait Node<R: AsyncRead + AsyncSeek> {
    fn name(&self) -> &str;
    fn get_type(&self) -> NodeType;
    fn as_directory_ref(&self) -> Option<&Directory<R>>;
    fn as_directory_mut(&mut self) -> Option<&mut Directory<R>>;
    fn as_file_ref(&self) -> Option<&File<R>>;
    fn as_file_mut(&mut self) -> Option<&mut File<R>>;

    fn as_enum_ref(&self) -> NodeEnumRef<'_, R> {
        match self.get_type() {
            NodeType::File => NodeEnumRef::File(self.as_file_ref().unwrap()),
            NodeType::Directory => NodeEnumRef::Directory(self.as_directory_ref().unwrap()),
        }
    }

    fn as_enum_mut(&mut self) -> NodeEnumMut<'_, R> {
        match self.get_type() {
            NodeType::File => NodeEnumMut::File(self.as_file_mut().unwrap()),
            NodeType::Directory => NodeEnumMut::Directory(self.as_directory_mut().unwrap()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub enum NodeType {
    File = 0,
    Directory,
}

pub enum NodeEnumRef<'a, R: AsyncRead + AsyncSeek> {
    File(&'a File<R>),
    Directory(&'a Directory<R>),
}

pub enum NodeEnumMut<'a, R: AsyncRead + AsyncSeek> {
    File(&'a mut File<R>),
    Directory(&'a mut Directory<R>),
}

pub struct GeckoFS<R: AsyncRead + AsyncSeek> {
    pub(super) root: Directory<R>,
    pub(super) system: Directory<R>,
    reader: Arc<Mutex<DiscReader<R>>>,
}

impl<R> GeckoFS<R>
where
    R: AsyncRead + AsyncSeek + 'static,
{
    #[doc = r"Utility function to read the disc."]
    async fn read_exact<R2: DerefMut<Target = DiscReader<R>>>(
        reader: &mut R2,
        pos: SeekFrom,
        buf: &mut [u8],
    ) -> Result<()> {
        reader.seek(pos).await?;
        Ok(reader.read_exact(buf).await?)
    }

    fn get_dir_structure_recursive(
        mut cur_index: usize,
        fst: &Vec<FstEntry>,
        parent_dir: &mut Directory<R>,
    ) -> usize {
        let entry = &fst[cur_index];

        if entry.kind == FstNodeType::Directory {
            let mut dir =
                Directory::new(parent_dir.reader.clone(), entry.relative_file_name.clone());

            while cur_index < entry.file_size_next_dir_index - 1 {
                cur_index = GeckoFS::get_dir_structure_recursive(cur_index + 1, fst, &mut dir);
            }

            parent_dir.children.push(Box::new(dir));
        } else {
            let file = File::new(
                parent_dir.reader.clone(),
                entry.relative_file_name.clone(),
                entry.file_offset_parent_dir,
                entry.file_size_next_dir_index,
                entry.file_name_offset,
            );
            parent_dir.children.push(Box::new(file));
        }

        cur_index
    }

    pub async fn parse(reader: Arc<Mutex<DiscReader<R>>>) -> Result<Arc<Mutex<Self>>> {
        let mut root = Directory::new(reader.clone(), "root");
        let mut system = Directory::new(reader.clone(), "&&systemdata");
        {
            let mut guard = reader.lock_arc().await;
            let is_wii = guard.get_type() == DiscType::Wii;
            crate::debug!(
                "{:?}",
                if is_wii {
                    "The disc is a Wii game"
                } else {
                    "The disc is NOT a Wii game"
                }
            );
            let mut buf = [0u8; 4];
            GeckoFS::read_exact(
                &mut guard,
                SeekFrom::Start(consts::OFFSET_FST_OFFSET as u64),
                &mut buf,
            )
            .await?;
            let fst_offset = (BE::read_u32(&buf[..]) << (if is_wii { 2 } else { 0 })) as u64;
            GeckoFS::read_exact(&mut guard, SeekFrom::Start(fst_offset + 8), &mut buf).await?;
            let num_entries = BE::read_u32(&buf[..]) as usize;
            let mut fst_list_buf = vec![0u8; num_entries * 0xC];
            GeckoFS::read_exact(&mut guard, SeekFrom::Start(fst_offset), &mut fst_list_buf).await?;
            let string_table_offset = num_entries as u64 * 0xC;

            GeckoFS::read_exact(
                &mut guard,
                SeekFrom::Start(consts::OFFSET_FST_SIZE as u64),
                &mut buf,
            )
            .await?;
            let fst_size = (BE::read_u32(&buf) as usize) << (if is_wii { 2 } else { 0 });
            let mut str_tbl_buf = vec![0u8; fst_size - string_table_offset as usize];
            GeckoFS::read_exact(
                &mut guard,
                SeekFrom::Start(string_table_offset + fst_offset),
                &mut str_tbl_buf,
            )
            .await?;

            let mut fst_entries = Vec::with_capacity(num_entries);
            for i in 0..num_entries {
                let kind =
                    FstNodeType::try_from(fst_list_buf[i * 12]).unwrap_or(FstNodeType::Directory);

                let string_offset = (BE::read_u32(&fst_list_buf[i * 12..]) & 0x00ffffff) as usize;

                let pos = string_offset;
                let mut end = pos;
                while str_tbl_buf[end] != 0 {
                    end += 1;
                }
                let mut str_buf = Vec::new();
                str_buf.extend_from_slice(&str_tbl_buf[pos..end]);
                let relative_file_name = String::from_utf8(str_buf)?;

                let file_offset_parent_dir = (BE::read_u32(&fst_list_buf[i * 12 + 4..]) as usize)
                    << (if is_wii { 2 } else { 0 });
                let file_size_next_dir_index = BE::read_u32(&fst_list_buf[i * 12 + 8..]) as usize;

                fst_entries.push(FstEntry {
                    kind,
                    relative_file_name,
                    file_offset_parent_dir,
                    file_size_next_dir_index,
                    file_name_offset: 0,
                });
            }

            GeckoFS::read_exact(
                &mut guard,
                SeekFrom::Start(consts::OFFSET_DOL_OFFSET as u64),
                &mut buf,
            )
            .await?;
            let dol_offset = (BE::read_u32(&buf) as usize) << (if is_wii { 2 } else { 0 });
            crate::debug!(
                "fst_size: 0x{:08X}; fst entries list size: 0x{:08X}",
                fst_size,
                num_entries * 12
            );

            system.children.push(Box::new(File::new(
                reader.clone(),
                "iso.hdr",
                0,
                consts::HEADER_LENGTH,
                0,
            )));
            system.children.push(Box::new(File::new(
                reader.clone(),
                "AppLoader.ldr",
                consts::HEADER_LENGTH,
                dol_offset - consts::HEADER_LENGTH,
                0,
            )));
            system.children.push(Box::new(File::new(
                reader.clone(),
                "Start.dol",
                dol_offset,
                fst_offset as usize - dol_offset,
                0,
            )));
            system.children.push(Box::new(File::new(
                reader.clone(),
                "Game.toc",
                fst_offset as usize,
                fst_size,
                0,
            )));

            let mut count = 1;

            while count < num_entries {
                let entry = &fst_entries[count];
                if entry.kind == FstNodeType::Directory {
                    let mut new_dir =
                        Directory::<R>::new(reader.clone(), entry.relative_file_name.clone());
                    while count < entry.file_size_next_dir_index - 1 {
                        count = GeckoFS::get_dir_structure_recursive(
                            count + 1,
                            &fst_entries,
                            &mut new_dir,
                        );
                    }
                    root.children.push(Box::new(new_dir));
                } else {
                    let file = File::new(
                        reader.clone(),
                        entry.relative_file_name.clone(),
                        entry.file_offset_parent_dir,
                        entry.file_size_next_dir_index,
                        entry.file_name_offset,
                    );
                    root.children.push(Box::new(file));
                }
                count += 1;
            }
        }
        crate::debug!("{} children", root.children.len());
        Ok(Arc::new(Mutex::new(Self {
            root,
            system,
            reader: reader.clone(),
        })))
    }

    pub fn main_dol_mut(&mut self) -> &mut File<R> {
        self.system
            .resolve_node("Start.dol")
            .unwrap()
            .as_file_mut()
            .unwrap()
    }

    pub fn banner_mut(&mut self) -> Option<&mut File<R>> {
        self.root.resolve_node("opening.bnr")?.as_file_mut()
    }

    pub async fn get_disc_type(&self) -> DiscType {
        self.reader.lock_arc().await.get_type()
    }

    pub fn get_child_count(&self) -> usize {
        self.root.children.len()
    }
}

impl<R> Node<R> for GeckoFS<R>
where
    R: AsyncRead + AsyncSeek,
{
    fn name(&self) -> &str {
        self.root.name()
    }

    fn get_type(&self) -> NodeType {
        NodeType::Directory
    }

    fn as_directory_ref(&self) -> Option<&Directory<R>> {
        Some(&self.root)
    }

    fn as_directory_mut(&mut self) -> Option<&mut Directory<R>> {
        Some(&mut self.root)
    }

    fn as_file_ref(&self) -> Option<&File<R>> {
        None
    }

    fn as_file_mut(&mut self) -> Option<&mut File<R>> {
        None
    }
}

pub struct Directory<R: AsyncRead + AsyncSeek> {
    name: String,
    children: Vec<Box<dyn Node<R>>>,
    reader: Arc<Mutex<DiscReader<R>>>,
}

impl<R> Directory<R>
where
    R: AsyncRead + AsyncSeek + 'static,
{
    pub fn new<S: Into<String>>(reader: Arc<Mutex<DiscReader<R>>>, name: S) -> Directory<R> {
        Self {
            name: name.into(),
            children: Vec::new(),
            reader,
        }
    }

    pub fn resolve_node(&mut self, path: &str) -> Option<&mut dyn Node<R>> {
        let mut dir = self;
        let mut segments = path.split('/').peekable();

        while let Some(segment) = segments.next() {
            if segments.peek().is_some() {
                // Must be a folder
                dir = dir
                    .children
                    .iter_mut()
                    .filter_map(|c| c.as_directory_mut())
                    .find(|d| d.name == segment)?;
            } else {
                return dir
                    .children
                    .iter_mut()
                    .filter_map(|c| c.as_file_mut())
                    .find(|f| f.name() == segment)
                    .map(|x| x as &mut dyn Node<R>);
            }
        }
        Some(dir)
    }

    pub async fn mkdir(&mut self, name: String) -> Result<&mut Directory<R>> {
        self.children
            .push(Box::new(Directory::new(self.reader.clone(), name)));
        Ok(self
            .children
            .last_mut()
            .map(|x| x.as_directory_mut().unwrap())
            .unwrap())
    }
}

impl<R> Node<R> for Directory<R>
where
    R: AsyncRead + AsyncSeek,
{
    fn name(&self) -> &str {
        &self.name
    }

    fn get_type(&self) -> NodeType {
        NodeType::Directory
    }

    fn as_directory_ref(&self) -> Option<&Directory<R>> {
        Some(self)
    }

    fn as_directory_mut(&mut self) -> Option<&mut Directory<R>> {
        Some(self)
    }

    fn as_file_ref(&self) -> Option<&File<R>> {
        None
    }

    fn as_file_mut(&mut self) -> Option<&mut File<R>> {
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
enum FileReadState {
    #[default]
    Seeking,
    Reading,
}

#[derive(Debug, Clone, Copy, Default)]
struct FileState {
    cursor: u64,
    state: FileReadState,
}

pub struct File<R: AsyncRead + AsyncSeek> {
    fst: FstEntry,
    state: FileState,
    reader: Arc<Mutex<DiscReader<R>>>,
}

impl<R> File<R>
where
    R: AsyncRead + AsyncSeek,
{
    pub fn new<S: Into<String>>(
        reader: Arc<Mutex<DiscReader<R>>>,
        name: S,
        file_offset_parent_dir: usize,
        file_size_next_dir_index: usize,
        file_name_offset: usize,
    ) -> Self {
        Self {
            fst: FstEntry {
                kind: FstNodeType::File,
                relative_file_name: name.into(),
                file_offset_parent_dir,
                file_size_next_dir_index,
                file_name_offset,
            },
            state: Default::default(),
            reader,
        }
    }
}

impl<R: AsyncRead + AsyncSeek> AsyncSeek for File<R> {
    fn poll_seek(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut Context<'_>,
        pos: SeekFrom,
    ) -> Poll<std::io::Result<u64>> {
        crate::debug!("Seeking \"{0}\" to {1:?} ({1:016X?})", self.name(), pos,);
        let pos = match pos {
            SeekFrom::Start(pos) => {
                if pos > self.fst.file_size_next_dir_index as u64 {
                    return Poll::Ready(Err(Error::new(
                        async_std::io::ErrorKind::Other,
                        eyre::eyre!("Index out of range"),
                    )));
                }
                SeekFrom::Start(pos)
            }
            SeekFrom::End(pos) => {
                let new_pos = self.fst.file_size_next_dir_index as i64 + pos;
                if new_pos < 0 || pos > 0 {
                    return Poll::Ready(Err(Error::new(
                        async_std::io::ErrorKind::Other,
                        eyre::eyre!("Index out of range"),
                    )));
                }
                SeekFrom::End(pos)
            }
            SeekFrom::Current(pos) => {
                let new_pos = self.state.cursor as i64 + pos;
                if new_pos < 0 || new_pos > self.fst.file_size_next_dir_index as i64 {
                    return Poll::Ready(Err(Error::new(
                        async_std::io::ErrorKind::Other,
                        eyre::eyre!("Index out of range"),
                    )));
                }
                SeekFrom::Current(pos)
            }
        };
        match self.reader.try_lock_arc() {
            Some(mut guard) => {
                crate::debug!("Disc reader locked");
                let guard_pin = std::pin::pin!(guard.deref_mut());
                match guard_pin.poll_seek(
                    cx,
                    match pos {
                        SeekFrom::Start(pos) => {
                            SeekFrom::Start(self.fst.file_offset_parent_dir as u64 + pos)
                        }
                        SeekFrom::End(pos) => SeekFrom::Start(
                            ((self.fst.file_offset_parent_dir as i64
                                + self.fst.file_size_next_dir_index as i64)
                                + pos) as u64,
                        ),
                        SeekFrom::Current(pos) => SeekFrom::Start(
                            (self.fst.file_offset_parent_dir as i64
                                + self.state.cursor as i64
                                + pos) as u64,
                        ),
                    },
                ) {
                    Poll::Ready(Ok(p)) => match pos {
                        SeekFrom::Start(pos) => {
                            self.state.cursor = pos;
                            Poll::Ready(Ok(p))
                        }
                        SeekFrom::End(pos) => {
                            self.state.cursor =
                                (self.fst.file_size_next_dir_index as i64 + pos) as u64;
                            Poll::Ready(Ok(p))
                        }
                        SeekFrom::Current(pos) => {
                            self.state.cursor = (self.state.cursor as i64 + pos) as u64;
                            Poll::Ready(Ok(p))
                        }
                    },
                    Poll::Ready(Err(err)) => Poll::Ready(Err(err)),
                    Poll::Pending => Poll::Pending,
                }
            }
            None => {
                crate::debug!("Disc reader can't be locked");
                cx.waker().wake_by_ref();
                Poll::Pending
            }
        }
    }
}

impl<R: AsyncRead + AsyncSeek> AsyncRead for File<R> {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut [u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        crate::debug!(
            "Reading \"{}\" for 0x{:08X} byte(s)",
            self.name(),
            buf.len()
        );
        match self.state.state {
            FileReadState::Seeking => match self.reader.try_lock_arc() {
                Some(mut guard) => {
                    crate::debug!("Disc reader locked");
                    let guard_pin = std::pin::pin!(guard.deref_mut());
                    match guard_pin.poll_seek(cx, SeekFrom::Start(self.fst.file_offset_parent_dir as u64 + self.state.cursor)) {
                        Poll::Ready(Ok(_)) => {
                            self.state.state = FileReadState::Reading;
                            cx.waker().wake_by_ref();
                            Poll::Pending
                        }
                        Poll::Ready(Err(err)) => {
                            self.state.state = FileReadState::Seeking;
                            Poll::Ready(Err(err))
                        }
                        Poll::Pending => Poll::Pending,
                    }
                },
                None => {
                    crate::debug!("Disc reader can't be locked");
                    cx.waker().wake_by_ref();
                    Poll::Pending
                }
            },
            FileReadState::Reading => match self.reader.try_lock_arc() {
                Some(mut guard) => {
                    crate::debug!("Disc reader locked");
                    let guard_pin = std::pin::pin!(guard.deref_mut());
                    match guard_pin.poll_read(cx, buf) {
                        Poll::Ready(Ok(num_read)) => {
                            self.state.cursor += num_read as u64;
                            self.state.state = FileReadState::Seeking;
                            Poll::Ready(Ok(num_read))
                        }
                        Poll::Ready(Err(err)) => {
                            self.state.state = FileReadState::Seeking;
                            Poll::Ready(Err(err))
                        }
                        Poll::Pending => Poll::Pending,
                    }
                }
                None => {
                    crate::debug!("Disc reader can't be locked");
                    cx.waker().wake_by_ref();
                    Poll::Pending
                }
            },
        }
    }
}

impl<R> Node<R> for File<R>
where
    R: AsyncRead + AsyncSeek,
{
    fn name(&self) -> &str {
        &self.fst.relative_file_name
    }

    fn get_type(&self) -> NodeType {
        NodeType::File
    }

    fn as_directory_ref(&self) -> Option<&Directory<R>> {
        None
    }

    fn as_directory_mut(&mut self) -> Option<&mut Directory<R>> {
        None
    }

    fn as_file_ref(&self) -> Option<&File<R>> {
        Some(self)
    }

    fn as_file_mut(&mut self) -> Option<&mut File<R>> {
        Some(self)
    }
}
