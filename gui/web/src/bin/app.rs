#![warn(clippy::all, rust_2018_idioms)]
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] // hide console window on Windows in release

#[cfg(feature = "debug_alloc")]
#[global_allocator]
static ALLOC: wasm_tracing_allocator::WasmTracingAllocator<std::alloc::System> = wasm_tracing_allocator::WasmTracingAllocator(std::alloc::System);

fn main() {
    console_log::init().expect("could not initialize console_log");
    yew::Renderer::<web_gui_patcher::App>::new().render();
}