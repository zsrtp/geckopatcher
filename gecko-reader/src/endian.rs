pub trait FromBytesLE: Sized {
    const N: usize;
    fn from_bytes_le(buf: &[u8]) -> Self;
}

pub trait FromBytesBE: Sized {
    const N: usize;
    fn from_bytes_be(buf: &[u8]) -> Self;
}

macro_rules! impl_from_bytes {
    ($type:ty, $name:tt, $size:tt) => {
        impl FromBytesLE for $type {
            const N: usize = $size;
            fn from_bytes_le(buf: &[u8]) -> Self {
                use ::byteorder::ByteOrder;
                ::byteorder::LE::$name(&buf[..])
            }
        }
        impl FromBytesBE for $type {
            const N: usize = $size;
            fn from_bytes_be(buf: &[u8]) -> Self {
                use ::byteorder::ByteOrder;
                ::byteorder::BE::$name(&buf[..])
            }
        }
    };
}

impl FromBytesLE for u8 {
    const N: usize = 1;
    fn from_bytes_le(buf: &[u8]) -> Self {
        buf[0]
    }
}

impl FromBytesBE for u8 {
    const N: usize = 1;
    fn from_bytes_be(buf: &[u8]) -> Self {
        buf[0]
    }
}

impl FromBytesLE for i8 {
    const N: usize = 1;
    fn from_bytes_le(buf: &[u8]) -> Self {
        buf[0] as i8
    }
}

impl FromBytesBE for i8 {
    const N: usize = 1;
    fn from_bytes_be(buf: &[u8]) -> Self {
        buf[0] as i8
    }
}

impl_from_bytes!(u16, read_u16, 2);
impl_from_bytes!(u32, read_u32, 4);
impl_from_bytes!(u64, read_u64, 8);
impl_from_bytes!(u128, read_u128, 16);
impl_from_bytes!(i16, read_i16, 2);
impl_from_bytes!(i32, read_i32, 4);
impl_from_bytes!(i64, read_i64, 8);
impl_from_bytes!(i128, read_i128, 16);
impl_from_bytes!(f32, read_f32, 4);
impl_from_bytes!(f64, read_f64, 8);
