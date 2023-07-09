use crate::iso::read::{DiscReader, DiscType};
use crate::iso::{consts, FstEntry, FstNodeType};
use async_std::io::prelude::SeekExt;
use async_std::io::{Read as AsyncRead, ReadExt, Seek as AsyncSeek};
use async_std::sync::{Arc, Mutex, Weak};
use byteorder::{ByteOrder, BE};
use eyre::Result;
use std::io::{Error, SeekFrom};
use std::ops::DerefMut;
use std::task::{Context, Poll};

pub trait Node<R: AsyncRead + AsyncSeek> {
    fn name<'a>(&'a self) -> &'a str;
    fn get_type(&self) -> NodeType;
    fn as_directory_ref<'a>(&'a self) -> Option<&'a Directory<R>>;
    fn as_directory_mut<'a>(&'a mut self) -> Option<&'a mut Directory<R>>;
    fn as_file_ref<'a>(&'a self) -> Option<&'a File<R>>;
    fn as_file_mut<'a>(&'a mut self) -> Option<&'a mut File<R>>;

    fn as_enum_ref<'a>(&'a self) -> NodeEnumRef<'a, R> {
        match self.get_type() {
            NodeType::File => NodeEnumRef::File(self.as_file_ref().unwrap()),
            NodeType::Directory => NodeEnumRef::Directory(self.as_directory_ref().unwrap()),
        }
    }

    fn as_enum_mut<'a>(&'a mut self) -> NodeEnumMut<'a, R> {
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
    pub(super) _iso_hdr: File<R>,
    pub(super) _app_ldr: File<R>,
    pub(super) _main_dol: File<R>,
    pub(super) _game_toc: File<R>,
    reader: Arc<Mutex<DiscReader<R>>>,
}

impl<R> GeckoFS<R>
where
    R: AsyncRead + AsyncSeek + 'static,
{
    #[doc = r"Visits all the children and replaces the weak reference of the filesystem."]
    fn weak_replace_visitor(w: Weak<GeckoFS<R>>, root: &mut Directory<R>) {
        root.fs = w.clone();
        for child in root
            .children
            .iter_mut()
            .filter_map(|c| c.as_directory_mut())
        {
            GeckoFS::weak_replace_visitor(w.clone(), child);
        }
    }

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
            let mut dir = Directory::new(parent_dir.fs.clone(), entry.relative_file_name.clone());
            
            while cur_index < entry.file_size_next_dir_index - 1 {
                cur_index = GeckoFS::get_dir_structure_recursive(cur_index + 1, fst, &mut dir);
            }

            parent_dir.children.push(Box::new(dir));
        } else {
            let file = File::new(parent_dir.fs.clone(), entry.relative_file_name.clone(), entry.file_offset_parent_dir, entry.file_size_next_dir_index, entry.file_name_offset);
            parent_dir.children.push(Box::new(file));
        }

        cur_index
    }

    pub async fn parse(reader: Arc<Mutex<DiscReader<R>>>) -> Result<Arc<Self>> {
        let mut root = Directory::new(Weak::new(), "root");
        let mut iso_hdr: File<R>;
        let mut app_ldr: File<R>;
        let mut main_dol: File<R>;
        let mut game_toc: File<R>;
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
            GeckoFS::read_exact(
                &mut guard,
                SeekFrom::Start((fst_offset + 8) as u64),
                &mut buf,
            )
            .await?;
            let num_entries = BE::read_u32(&buf[..]) as usize;
            let string_table_offset = num_entries as u64 * 0xC;

            let mut fst_entries = Vec::with_capacity(num_entries);
            for i in 0..num_entries {
                let mut buf = [0u8; 1];
                GeckoFS::read_exact(
                    &mut guard,
                    SeekFrom::Start((fst_offset + i as u64 * 12) as u64),
                    &mut buf,
                )
                .await?;
                let kind = FstNodeType::try_from(buf[0]).unwrap_or(FstNodeType::Directory);

                let mut buf = [0u8; 4];
                GeckoFS::read_exact(
                    &mut guard,
                    SeekFrom::Start((fst_offset + i as u64 * 12) as u64),
                    &mut buf,
                )
                .await?;
                let string_offset = (BE::read_u32(&buf) & 0x00ffffff) as usize;

                let pos = string_offset as u64 + string_table_offset + fst_offset;
                let mut buf = [0u8; 1];
                let mut end = pos;
                GeckoFS::read_exact(&mut guard, SeekFrom::Start(end), &mut buf).await?;
                while buf[0] != 0 {
                    end += 1;
                    GeckoFS::read_exact(&mut guard, SeekFrom::Start(end), &mut buf).await?;
                }
                let mut buf = vec![0u8; (end - pos) as usize];
                GeckoFS::read_exact(&mut guard, SeekFrom::Start(pos), &mut buf).await?;
                let relative_file_name = String::from_utf8(buf)?;

                let mut buf = [0u8; 4];
                GeckoFS::read_exact(
                    &mut guard,
                    SeekFrom::Start(fst_offset + i as u64 * 12 + 4),
                    &mut buf,
                )
                .await?;
                let file_offset_parent_dir =
                    (BE::read_u32(&buf) as usize) << (if is_wii { 2 } else { 0 });
                GeckoFS::read_exact(
                    &mut guard,
                    SeekFrom::Start(fst_offset + i as u64 * 12 + 8),
                    &mut buf,
                )
                .await?;
                let file_size_next_dir_index = BE::read_u32(&buf) as usize;

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
            GeckoFS::read_exact(
                &mut guard,
                SeekFrom::Start(consts::OFFSET_FST_SIZE as u64),
                &mut buf,
            )
            .await?;
            let fst_size = (BE::read_u32(&buf) as usize) << (if is_wii { 2 } else { 0 });

            iso_hdr = File::new(Weak::new(), "iso.hdr", 0, consts::HEADER_LENGTH, 0);
            app_ldr = File::new(
                Weak::new(),
                "AppLoader.ldr",
                consts::HEADER_LENGTH,
                dol_offset - consts::HEADER_LENGTH,
                0,
            );
            main_dol = File::new(
                Weak::new(),
                "Start.dol",
                dol_offset,
                fst_offset as usize - dol_offset,
                0,
            );
            game_toc = File::new(Weak::new(), "Game.toc", fst_offset as usize, fst_size, 0);

            let mut count = 1;

            while count < num_entries {
                let entry = &fst_entries[count];
                if entry.kind == FstNodeType::Directory {
                    let mut new_dir = Directory::<R>::new(Weak::new(), entry.relative_file_name.clone());
                    while count < entry.file_size_next_dir_index - 1 {
                        count = GeckoFS::get_dir_structure_recursive(count + 1, &fst_entries, &mut new_dir);
                    }
                    root.children.push(Box::new(new_dir));
                } else {
                    let file = File::new(Weak::new(), entry.relative_file_name.clone(), entry.file_offset_parent_dir, entry.file_size_next_dir_index, entry.file_name_offset);
                    root.children.push(Box::new(file));
                }
                count += 1;
            }
        }
        Ok(Arc::new_cyclic(|weak| {
            GeckoFS::weak_replace_visitor(weak.clone(), &mut root);
            crate::debug!("{} children", root.children.len());
            iso_hdr.fs = weak.clone();
            app_ldr.fs = weak.clone();
            game_toc.fs = weak.clone();
            main_dol.fs = weak.clone();
            Self {
                root,
                _iso_hdr: iso_hdr,
                _app_ldr: app_ldr,
                _game_toc: game_toc,
                _main_dol: main_dol,
                reader: reader.clone(),
            }
        }))
    }

    pub fn main_dol_mut(&mut self) -> Option<&mut File<R>> {
        Some(
            self.root
                .resolve_node("&&systemdata/Start.dol")?
                .as_file_mut()?,
        )
    }

    pub fn banner_mut(&mut self) -> Option<&mut File<R>> {
        Some(self.root.resolve_node("opening.bnr")?.as_file_mut()?)
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
    fn name<'a>(&'a self) -> &'a str {
        self.root.name()
    }

    fn get_type(&self) -> NodeType {
        NodeType::Directory
    }

    fn as_directory_ref<'a>(&'a self) -> Option<&'a Directory<R>> {
        Some(&self.root)
    }

    fn as_directory_mut<'a>(&'a mut self) -> Option<&'a mut Directory<R>> {
        Some(&mut self.root)
    }

    fn as_file_ref<'a>(&'a self) -> Option<&'a File<R>> {
        None
    }

    fn as_file_mut<'a>(&'a mut self) -> Option<&'a mut File<R>> {
        None
    }
}

pub struct Directory<R: AsyncRead + AsyncSeek> {
    name: String,
    children: Vec<Box<dyn Node<R>>>,
    fs: Weak<GeckoFS<R>>,
}

impl<R> Directory<R>
where
    R: AsyncRead + AsyncSeek + 'static,
{
    pub fn new<S: Into<String>>(fs: Weak<GeckoFS<R>>, name: S) -> Directory<R> {
        Self {
            name: name.into(),
            children: Vec::new(),
            fs,
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

    pub async fn mkdir<'a>(&'a mut self, name: String) -> Result<&mut Directory<R>> {
        self.children
            .push(Box::new(Directory::new(self.fs.clone(), name)));
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
    fn name<'a>(&'a self) -> &'a str {
        &self.name
    }

    fn get_type(&self) -> NodeType {
        NodeType::Directory
    }

    fn as_directory_ref<'a>(&'a self) -> Option<&'a Directory<R>> {
        Some(self)
    }

    fn as_directory_mut<'a>(&'a mut self) -> Option<&'a mut Directory<R>> {
        Some(self)
    }

    fn as_file_ref<'a>(&'a self) -> Option<&'a File<R>> {
        None
    }

    fn as_file_mut<'a>(&'a mut self) -> Option<&'a mut File<R>> {
        None
    }
}

struct FileState {
    cursor: u64,
}

pub struct File<R: AsyncRead + AsyncSeek> {
    fst: FstEntry,
    state: FileState,
    fs: Weak<GeckoFS<R>>,
}

impl<R> File<R>
where
    R: AsyncRead + AsyncSeek,
{
    pub fn new<S: Into<String>>(
        fs: Weak<GeckoFS<R>>,
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
            state: FileState { cursor: 0 },
            fs,
        }
    }
}

impl<R: AsyncRead + AsyncSeek> AsyncSeek for File<R> {
    fn poll_seek(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut Context<'_>,
        pos: SeekFrom,
    ) -> Poll<std::io::Result<u64>> {
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
        match self.fs.upgrade() {
            Some(fs) => match fs.reader.try_lock_arc() {
                Some(mut guard) => {
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
                None => Poll::Ready(Err(async_std::io::Error::new(
                    std::io::ErrorKind::Other,
                    eyre::eyre!("Reader was deallocated! what!?"),
                ))),
            },
            None => Poll::Ready(Err(async_std::io::Error::new(
                std::io::ErrorKind::Other,
                eyre::eyre!("File System was deallocated! what!?"),
            ))),
        }
    }
}

impl<R: AsyncRead + AsyncSeek> AsyncRead for File<R> {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut [u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        if self.state.cursor + buf.len() as u64 > self.fst.file_size_next_dir_index as u64 {
            return Poll::Ready(Err(async_std::io::Error::new(
                std::io::ErrorKind::Other,
                eyre::eyre!("Index out of range"),
            )));
        }
        match self.fs.upgrade() {
            Some(fs) => match fs.reader.try_lock_arc() {
                Some(mut guard) => {
                    let guard_pin = std::pin::pin!(guard.deref_mut());
                    match guard_pin.poll_read(cx, buf) {
                        Poll::Ready(Ok(num_read)) => {
                            self.state.cursor += num_read as u64;
                            Poll::Ready(Ok(num_read))
                        }
                        Poll::Ready(Err(err)) => Poll::Ready(Err(err)),
                        Poll::Pending => todo!(),
                    }
                }
                None => Poll::Ready(Err(async_std::io::Error::new(
                    std::io::ErrorKind::Other,
                    eyre::eyre!("Reader was deallocated! what!?"),
                ))),
            },
            None => Poll::Ready(Err(async_std::io::Error::new(
                std::io::ErrorKind::Other,
                eyre::eyre!("File System was deallocated! what!?"),
            ))),
        }
    }
}

impl<R> Node<R> for File<R>
where
    R: AsyncRead + AsyncSeek,
{
    fn name<'a>(&'a self) -> &'a str {
        &self.fst.relative_file_name
    }

    fn get_type(&self) -> NodeType {
        NodeType::File
    }

    fn as_directory_ref<'a>(&'a self) -> Option<&'a Directory<R>> {
        None
    }

    fn as_directory_mut<'a>(&'a mut self) -> Option<&'a mut Directory<R>> {
        None
    }

    fn as_file_ref<'a>(&'a self) -> Option<&'a File<R>> {
        todo!()
    }

    fn as_file_mut<'a>(&'a mut self) -> Option<&'a mut File<R>> {
        todo!()
    }
}
