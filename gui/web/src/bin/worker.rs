use js_sys::{Function, Array};
use wasm_bindgen::JsValue;
use wasm_bindgen::prelude::*;

// When the `wee_alloc` feature is enabled, use `wee_alloc` as the global
// allocator.
#[cfg(feature = "wee_alloc")]
#[global_allocator]
static ALLOC: wee_alloc::WeeAlloc<'_> = wee_alloc::WeeAlloc::INIT;

#[wasm_bindgen]
pub async extern "C" fn run_patch(patch: &JsValue, file: &JsValue, save: &JsValue) -> Result<(), JsValue> {
    let patch: web_sys::File = patch.clone().dyn_into()?;
    let file: web_sys::File = file.clone().dyn_into()?;
    log::debug!("Patching game... {:?} {:?} {:?}", patch, file, save);
    Ok(())
}

fn main() {
    console_log::init_with_level(log::Level::Debug).expect("could not initialize worker's console_log");
    log::debug!("Starting Worker Thread");
}