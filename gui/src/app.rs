#[cfg(not(target_arch = "wasm32"))]
use async_std::channel::{Receiver, Sender};
use async_std::{
    channel::{TryRecvError, TrySendError},
    task::JoinHandle,
};
use futures_lite::Future;
use rfd::FileHandle;
#[cfg(target_arch = "wasm32")]
use std::{cell::RefCell, rc::Rc};
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::{closure::Closure, prelude::wasm_bindgen, JsValue};
#[cfg(target_arch = "wasm32")]
use web_sys::MessageChannel;

#[derive(Debug)]
pub enum InFile {
    Dropped(egui::DroppedFile),
    Path(std::path::PathBuf),
}

impl InFile {
    fn name(&self) -> String {
        match self {
            InFile::Dropped(f) => f.name.clone(),
            InFile::Path(f) => f.file_name().expect("File name could not be obtained").to_str().expect("File name is not a valid UTF-8 string").to_string(),
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Clone, Copy, Default)]
enum ToAppMsg {
    #[default]
    Echo,
    GetOpenFile,
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Default)]
enum FromAppMsg {
    #[default]
    Echo,
    OpenedFile(FileHandle),
    NoFileOpened,
}

#[cfg(not(target_arch = "wasm32"))]
struct MessageChannel {
    sender: Sender<ToAppMsg>,
    receiver: Receiver<FromAppMsg>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
enum OpenFileState {
    #[default]
    None,
    Opening,
    Opened,
}

pub struct PatcherApp {
    in_file: Option<InFile>,
    // Convention for wasm32 is port1 is gui's and port2 is background
    channels: MessageChannel,
    picked_file: OpenFileState,
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn run_thread(_port: &JsValue) {
    loop {
        wasm_rs_async_executor::single_threaded::block_on(async {
            // println!("application thread received: {:?}", rcv_app.recv(Some(Duration::from_secs_f64(60. * 60. * 24. * 365.))).unwrap());
            // let _ = snd_app.send(&0xa5);
        });
    }
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = window)]
    pub fn getSelectedFile();
}

impl PatcherApp {
    #[cfg(not(target_arch = "wasm32"))]
    /// Called once before the first frame.
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        // This is also where you can customize the look and feel of egui using
        // `cc.egui_ctx.set_visuals` and `cc.egui_ctx.set_fonts`.

        // Start the parallel async thread which will handle all the asynchronous tasks.

        use async_std::channel::RecvError;
        {
            let (snd_app, rcv_ui) = async_std::channel::unbounded::<FromAppMsg>();
            let (snd_ui, rcv_app) = async_std::channel::unbounded::<ToAppMsg>();
            async_std::task::spawn(async move {
                let (snd_app, rcv_app) = (snd_app, rcv_app);

                loop {
                    match rcv_app.recv().await {
                        Ok(msg) => match msg {
                            ToAppMsg::Echo => {
                                if let Err(err) = snd_app.send(FromAppMsg::Echo).await {
                                    log::warn!(
                                        "The channel was closed! Terminating worker thread. {:?}",
                                        err
                                    );
                                    return;
                                }
                            }
                            ToAppMsg::GetOpenFile => {
                                match rfd::AsyncFileDialog::new()
                                    .add_filter("application/x-cd-image", &[".iso"])
                                    .pick_file()
                                    .await
                                {
                                    Some(file) => {
                                        log::debug!("Got a file from the user!");
                                        if snd_app.send(FromAppMsg::OpenedFile(file)).await.is_err()
                                        {
                                            return;
                                        }
                                    }
                                    None => {
                                        if snd_app.send(FromAppMsg::NoFileOpened).await.is_err() {
                                            return;
                                        }
                                    }
                                };
                            }
                        },
                        Err(RecvError) => {
                            return;
                        }
                    }
                }
            });
            Self {
                in_file: None,
                channels: MessageChannel {
                    sender: snd_ui,
                    receiver: rcv_ui,
                },
                picked_file: Default::default(),
            }
        }
    }

    #[cfg(target_arch = "wasm32")]
    /// Called once before the first frame.
    pub fn new(_cc: &eframe::CreationContext<'_>, worker: Rc<RefCell<web_sys::Worker>>) -> Self {
        use wasm_bindgen::JsCast;
        // This is also where you can customize the look and feel of egui using
        // `cc.egui_ctx.set_visuals` and `cc.egui_ctx.set_fonts`.

        // Start the parallel async thread which will handle all the asynchronous tasks.
        {
            let channels = MessageChannel::new().expect("Could not obtain a MessageChannel");
            let data = js_sys::Object::new();
            let _ = js_sys::Reflect::set(&data, &"type".into(), &JsValue::from_str("wasm_mem"));
            let _ = js_sys::Reflect::set(&data, &"port".into(), &channels.port2());
            // let _ = js_sys::Reflect::set(&data, &"mem".into(), &wasm_bindgen::memory());
            let _ = worker
                .borrow()
                .post_message_with_transfer(&data, &js_sys::Array::of1(&channels.port2()));
            let _ = channels.port1().add_event_listener_with_callback(
                "message",
                Closure::<dyn Fn(JsValue)>::new(|event: JsValue| {
                    log::debug!("UI Message Channel got a message: {:?}", event);
                })
                .as_ref()
                .unchecked_ref(),
            );
            channels.port1().start();
            web_sys::console::dir_1(&wasm_bindgen::memory());
            Self {
                in_file: None,
                channels,
                file_picker: None,
            }
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
        let Self {
            in_file,
            channels,
            picked_file,
        } = self;

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
            if ui
                .add_enabled(
                    *picked_file != OpenFileState::Opening,
                    egui::Button::new(file_button_text),
                )
                .clicked()
            {
                #[cfg(not(target_arch = "wasm32"))]
                if let Some(file) = rfd::FileDialog::new().pick_file() {
                    *picked_file = OpenFileState::Opening;
                    *in_file = Some(InFile::Path(file));
                    *picked_file = OpenFileState::Opened;
                } else {
                    *picked_file = OpenFileState::None;
                };
                #[cfg(target_arch = "wasm32")]
                {
                    let port = channels.port1();
                    wasm_bindgen_futures::spawn_local(async move {
                        let picked_file = rfd::AsyncFileDialog::new().pick_file().await;
                        log::debug!("{:?}", picked_file.unwrap());
                        // let _ = port.post_message(&JsValue::from_f64(42.));
                    });
                    getSelectedFile();
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

        match channels.receiver.try_recv() {
            Ok(FromAppMsg::Echo) => {
                log::warn!("GUI got Echo");
            },
            Ok(FromAppMsg::NoFileOpened) => {
                *picked_file = match *in_file {
                    Some(_) => OpenFileState::Opened,
                    None => OpenFileState::None,
                };
                *in_file = None;
            }
            Ok(FromAppMsg::OpenedFile(file)) => {
                *picked_file = OpenFileState::Opened;
                // *in_file = Some(InFile::Path(file));
            }
            Err(TryRecvError::Closed) => _frame.close(),
            Err(TryRecvError::Empty) => {}
        };
    }
}
