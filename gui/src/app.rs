#[cfg(target_arch = "wasm32")]
use std::time::Duration;

#[cfg(not(target_arch = "wasm32"))]
use async_std::channel::{Sender, Receiver};
#[cfg(target_arch = "wasm32")]
use wasm_rs_shared_channel::spsc::{Receiver, Sender};

#[derive(Debug)]
pub enum InFile {
    Dropped(egui::DroppedFile),
    Path(rfd::FileHandle),
}

impl InFile {
    fn name(&self) -> String {
        match self {
            InFile::Dropped(f) => f.name.clone(),
            InFile::Path(f) => f.file_name(),
        }
    }
}

pub struct PatcherApp {
    // Example stuff:
    in_file: Option<InFile>,
    channels: (Sender<u64>, Receiver<u64>),
}

impl PatcherApp {
    /// Called once before the first frame.
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        // This is also where you can customize the look and feel of egui using
        // `cc.egui_ctx.set_visuals` and `cc.egui_ctx.set_fonts`.

        // Start the parallel async thread which will handle all the asynchronous tasks.
        #[cfg(not(target_arch = "wasm32"))]
        {
            let (snd_app, rcv_ui) = async_std::channel::unbounded::<u64>();
            let (snd_ui, rcv_app) = async_std::channel::unbounded::<u64>();
            std::thread::spawn(move || -> ! {
                let rcv_app = rcv_app;

                loop {
                    async_std::task::block_on(async {
                        if let Ok(msg) = rcv_app.recv().await {
                            println!("application thread received: {}", msg);
                            let _ = snd_app.send(msg).await;
                        }
                    });
                }
            });
            Self { in_file: None, channels: (snd_ui, rcv_ui) }
        }
        #[cfg(target_arch = "wasm32")]
        {
            let (snd_app, rcv_ui) = wasm_rs_shared_channel::spsc::channel(2).split();
            let (snd_ui, rcv_app) = wasm_rs_shared_channel::spsc::channel(2).split();
            wasm_rs_async_executor::single_threaded::spawn(async move {
                loop {
                    wasm_rs_async_executor::single_threaded::block_on(async {
                        println!("application thread received: {:?}", rcv_app.recv(Some(Duration::from_secs_f64(60. * 60. * 24. * 365.))).unwrap());
                    });
                }
            });
            Self { in_file: None, channels: (snd_ui, rcv_ui) }
        }

    }
}

impl eframe::App for PatcherApp {
    // /// Called by the frame work to save state before shutdown.
    // fn save(&mut self, _storage: &mut dyn eframe::Storage) {
    //     // eframe::set_value(storage, eframe::APP_KEY, self);
    // }

    /// Called each time the UI needs repainting, which may be many times per second.
    /// Put your widgets into a `SidePanel`, `TopPanel`, `CentralPanel`, `Window` or `Area`.
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let Self { in_file, channels: (snd, rcv) } = self;

        // Examples of how to create different panels and windows.
        // Pick whichever suits you.
        // Tip: a good default choice is to just keep the `CentralPanel`.
        // For inspiration and more examples, go to https://emilk.github.io/egui

        #[cfg(not(target_arch = "wasm32"))] // no File->Quit on web pages!
        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            // The top panel is often a good place for a menu bar:
            egui::menu::bar(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("Quit").clicked() {
                        _frame.close();
                    }
                });
            });
        });

        egui::SidePanel::left("side_panel").show(ctx, |ui| {
            ui.heading("ISO Patching");

            let file_button_text = {
                if let Some(f) = in_file {
                    f.name()
                } else {
                    "Open File...".into()
                }
            };
            if ui.button(file_button_text).clicked() {
                #[cfg(not(target_arch = "wasm32"))]
                {
                    let result = snd.send_blocking(42);
                    log::debug!("{:?}", result);
                    // let picked_file = futures_lite::future::block_on(rfd::AsyncFileDialog::new().pick_file());
                    // *in_file = picked_file.map(InFile::Path);
                };
                #[cfg(target_arch = "wasm32")]
                {
                    let result = snd.send(&42);
                    log::error!("{:?}", result);
                    // wasm_bindgen_futures::spawn_local(async move {
                    //     let picked_file = rfd::AsyncFileDialog::new().pick_file().await;
                    //     // *in_file = picked_file.map(InFile::Path);
                    // });
                };
            }

            ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    ui.label("powered by ");
                    ui.hyperlink_to("egui", "https://github.com/emilk/egui");
                    ui.label(" and ");
                    ui.hyperlink_to(
                        "eframe",
                        "https://github.com/emilk/egui/tree/master/crates/eframe",
                    );
                    ui.label(".");
                });
                ui.horizontal(|ui| {
                    ui.hyperlink_to("Github", "https://github.com/kipcode66/geckopatcher")
                });
                egui::warn_if_debug_build(ui);
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            // The central panel the region left after adding TopPanel's and SidePanel's

            ui.heading("Web Romhack Patcher");
        });

        if false {
            egui::Window::new("Window").show(ctx, |ui| {
                ui.label("Windows can be moved by dragging them.");
                ui.label("They are automatically sized based on contents.");
                ui.label("You can turn on resizing and scrolling if you like.");
                ui.label("You would normally choose either panels OR windows.");
            });
        }
    }
}
