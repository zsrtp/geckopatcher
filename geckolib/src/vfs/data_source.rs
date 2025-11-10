use crate::iso::{FstNode, read::DiscReader};

#[derive(Debug)]
pub enum FileDataSource<R> {
    Reader { reader: DiscReader<R>, fst: FstNode },
    Box { data: Box<[u8]>, name: String },
}

impl<R> FileDataSource<R> {
    pub fn name(&self) -> String {
        match self {
            Self::Reader { fst, .. } => fst.get_relative_file_name().to_owned(),
            Self::Box { name, .. } => name.clone(),
        }
    }

    pub fn len(&self) -> usize {
        match self {
            Self::Reader { fst, .. } => fst.get_file_size().unwrap(),
            Self::Box { data, .. } => data.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl<R> Clone for FileDataSource<R>
where
    R: Clone,
{
    fn clone(&self) -> Self {
        match self {
            Self::Reader { reader, fst } => Self::Reader {
                reader: reader.clone(),
                fst: fst.clone(),
            },
            Self::Box { data, name } => Self::Box {
                data: data.clone(),
                name: name.clone(),
            },
        }
    }
}
