#![warn(clippy::all, rust_2018_idioms)]
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] // hide console window on Windows in release

// When the `wee_alloc` feature is enabled, use `wee_alloc` as the global
// allocator.
#[cfg(all(target_arch = "wasm32", feature = "wee_alloc"))]
#[global_allocator]
static ALLOC: wee_alloc::WeeAlloc<'_> = wee_alloc::WeeAlloc::INIT;

// When compiling natively:
#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result<()> {
    env_logger::init(); // Log to stderr (if you run with `RUST_LOG=debug`).

    let native_options = eframe::NativeOptions::default();
    eframe::run_native(
        "Web Romhack Patcher",
        native_options,
        Box::new(|cc| Box::new(gui::PatcherApp::new(cc))),
    )
}

// When compiling to web using trunk:
#[cfg(target_arch = "wasm32")]
fn main() {
    use std::{rc::Rc, cell::RefCell};
    use web_sys::{WorkerOptions, WorkerType};
    let _ = console_log::init_with_level(log::Level::Debug);
    // Redirect `log` message to `console.log` and friends:
    eframe::WebLogger::init(log::LevelFilter::Debug).ok();

    let web_options = eframe::WebOptions::default();

    // Start the application thread (on a WebWorker running in parallel)
    let worker_handle = Rc::new(RefCell::new(web_sys::Worker::new_with_options("./worker.js", WorkerOptions::new().type_(WorkerType::Module)).unwrap()));

    // Start the GUI thread (on the main thread of the the browser's page)
    wasm_bindgen_futures::spawn_local(async {
        eframe::WebRunner::new()
            .start(
                "the_canvas_id", // hardcode it
                web_options,
                Box::new(|cc| Box::new(gui::PatcherApp::new(cc, worker_handle))),
            )
            .await
            .expect("failed to start eframe");
    });
}