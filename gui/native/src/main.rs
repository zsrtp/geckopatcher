#![warn(clippy::all, rust_2018_idioms)]
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] // hide console window on Windows in release

mod app;
pub mod progress;
use std::sync::Arc;

pub use app::PatcherApp;
use eframe::Theme;
use egui::Vec2;

const ICON: &[u8; 94245] = include_bytes!("../assets/icon.png");

pub(crate) fn load_icon() -> egui::viewport::IconData {
    let (icon_rgba, icon_width, icon_height) = {
        let image = image::load_from_memory(ICON)
            .expect("Failed to open icon path")
            .into_rgba8();
        let (width, height) = image.dimensions();
        let rgba = image.into_raw();
        (rgba, width, height)
    };

    egui::viewport::IconData {
        rgba: icon_rgba,
        width: icon_width,
        height: icon_height,
    }
}

// When compiling natively:
fn main() -> eframe::Result<()> {
    env_logger::init(); // Log to stderr (if you run with `RUST_LOG=debug`).

    let icon = Arc::new(load_icon());

    // let native_options = eframe::NativeOptions::default();
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder {
            inner_size: Some(Vec2::new(300., 200.)),
            min_inner_size: Some(Vec2::new(280., 220.)),
            icon: Some(icon),
            drag_and_drop: Some(true),
            ..Default::default()
        },
        centered: true,
        follow_system_theme: true,
        default_theme: Theme::Dark,
        run_and_return: false,
        ..Default::default()
    };
    eframe::run_native(
        "Romhack Patcher",
        native_options,
        Box::new(|cc| Ok(Box::new(app::PatcherApp::new(cc)))),
    )
}
