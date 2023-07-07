extern crate log;
#[cfg(feature = "web")]
extern crate wasm_bindgen;
#[cfg(not(target = "wasm32"))]
extern crate rayon;
extern crate thiserror;
#[macro_use]
extern crate lazy_static;
extern crate async_std;
extern crate block_modes;
extern crate eyre;
extern crate sha1_smol;

pub mod iso;
pub mod vfs;
pub mod crypto;

