use std::ffi::CString;

#[derive(Debug, Clone)]
pub(crate) enum IntoCStringError {
    FromVecWithNull(std::ffi::FromVecWithNulError),
    NulError(std::ffi::NulError),
}

impl core::error::Error for IntoCStringError {}

impl std::fmt::Display for IntoCStringError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IntoCStringError::FromVecWithNull(err) => err.fmt(f),
            IntoCStringError::NulError(nul_error) => nul_error.fmt(f),
        }
    }
}

impl From<std::ffi::FromVecWithNulError> for IntoCStringError {
    fn from(value: std::ffi::FromVecWithNulError) -> Self {
        Self::FromVecWithNull(value)
    }
}

impl From<std::ffi::NulError> for IntoCStringError {
    fn from(value: std::ffi::NulError) -> Self {
        Self::NulError(value)
    }
}

pub(crate) fn c_string_from_slice(vec: &[u8]) -> Result<CString, IntoCStringError> {
    let end = vec
        .iter()
        .enumerate()
        .find_map(|(i, n)| if *n == 0 { Some(i) } else { None });
    if let Some(end) = end {
        CString::from_vec_with_nul(vec[..end].to_vec()).map_err(Into::into)
    } else {
        CString::new(vec.to_vec()).map_err(Into::into)
    }
}

#[macro_export(local_inner_macros)]
macro_rules! static_assert_eq_size {
    ($type:ty, $size:expr) => {
        const _: () = ::core::assert!(
            ::core::mem::size_of::<$type>() == $size,
            ::core::concat!(
                "Invalid size for ",
                ::core::stringify!($type),
                "; Expecting ",
                ::core::stringify!($size),
            )
        );
    };
}

#[macro_export(local_inner_macros)]
macro_rules! static_assert_eq_offset {
    ($type:ty, $field:expr, $offset:expr) => {
        const _: () = ::core::assert!(
            ::core::mem::offset_of!($type, $field) == $offset,
            ::core::concat!(
                "Invalid offset for field \"",
                ::core::stringify!($field),
                "\" of type ",
                ::core::stringify!($type),
                "; Expecting ",
                ::core::stringify!($offset)
            )
        );
    };
}

#[macro_export(local_inner_macros)]
macro_rules! static_debug_size {
    ($type:ty, $size:expr) => {
        const _: [u8; $size] = [0; ::core::mem::size_of::<$type>()];
    };
}

#[macro_export(local_inner_macros)]
macro_rules! static_debug_offset {
    ($type:ty, $field:expr, $offset:expr) => {
        const _: [u8; $offset] = [0; ::core::mem::offset_of!($type, $field)];
    };
}
