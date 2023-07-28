use std::io::{Read, Seek};

use flume::{Receiver, Sender, TryRecvError, TrySendError};
use rfd::FileHandle;

#[derive(Debug)]
pub enum InFile {
    Dropped(egui::DroppedFile),
    Path(FileHandle),
}

impl InFile {
    fn name(&self) -> String {
        match self {
            InFile::Dropped(f) => f
                .path
                .as_ref()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str())
                .map(|s| s.to_string())
                .unwrap_or("<no name>".to_string()),
            InFile::Path(f) => f.file_name(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
enum ToAppMsg {
    #[default]
    Echo,
    GetOpenFile,
    GetPatchFile,
}

#[derive(Debug, Default)]
enum FromAppMsg {
    #[default]
    Echo,
    OpenedFile(FileHandle),
    OpenedPatch(FileHandle),
    NoFileOpened,
    NoPatchOpened,
}

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
    patch_file: Option<InFile>,
    in_file: Option<InFile>,
    // Convention for wasm32 is port1 is gui's and port2 is background
    channels: MessageChannel,
    picked_patch: OpenFileState,
    picked_file: OpenFileState,
}

impl PatcherApp {
    /// Called once before the first frame.
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        // This is also where you can customize the look and feel of egui using
        // `cc.egui_ctx.set_visuals` and `cc.egui_ctx.set_fonts`.

        // Start the parallel async thread which will handle all the asynchronous tasks.
        let (snd_app, rcv_ui) = flume::unbounded::<FromAppMsg>();
        let (snd_ui, rcv_app) = flume::unbounded::<ToAppMsg>();
        async_std::task::spawn(async move {
            let (snd_app, rcv_app) = (snd_app, rcv_app);

            loop {
                match rcv_app.recv_async().await {
                    Ok(msg) => match msg {
                        ToAppMsg::Echo => {
                            if let Err(err) = snd_app.send_async(FromAppMsg::Echo).await {
                                log::warn!(
                                    "The channel was closed! Terminating worker thread. {:?}",
                                    err
                                );
                                return;
                            }
                        }
                        ToAppMsg::GetPatchFile => {
                            match rfd::AsyncFileDialog::new()
                                .add_filter("application/zip", &["patch"])
                                .pick_file()
                                .await
                            {
                                Some(file) => {
                                    log::debug!("Got a patch file from the user!");
                                    if snd_app
                                        .send_async(FromAppMsg::OpenedPatch(file))
                                        .await
                                        .is_err()
                                    {
                                        return;
                                    }
                                }
                                None => {
                                    if snd_app.send_async(FromAppMsg::NoPatchOpened).await.is_err()
                                    {
                                        return;
                                    }
                                }
                            };
                        }
                        ToAppMsg::GetOpenFile => {
                            match rfd::AsyncFileDialog::new()
                                .add_filter("application/x-cd-image", &["iso"])
                                .pick_file()
                                .await
                            {
                                Some(file) => {
                                    log::debug!("Got a file from the user!");
                                    if snd_app
                                        .send_async(FromAppMsg::OpenedFile(file))
                                        .await
                                        .is_err()
                                    {
                                        return;
                                    }
                                }
                                None => {
                                    if snd_app.send_async(FromAppMsg::NoFileOpened).await.is_err() {
                                        return;
                                    }
                                }
                            };
                        }
                    },
                    Err(flume::RecvError::Disconnected) => {
                        return;
                    }
                }
            }
        });
        Self {
            patch_file: None,
            in_file: None,
            channels: MessageChannel {
                sender: snd_ui,
                receiver: rcv_ui,
            },
            picked_patch: Default::default(),
            picked_file: Default::default(),
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
            patch_file,
            in_file,
            channels,
            picked_patch,
            picked_file,
        } = self;

        // Examples of how to create different panels and windows.
        // Pick whichever suits you.
        // Tip: a good default choice is to just keep the `CentralPanel`.
        // For inspiration and more examples, go to https://emilk.github.io/egui

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

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("ISO Patching");

            ui.group(|ui| {
                egui::Grid::new("files_grid").show(ui, |ui| {
                    ui.label("Patch File:");
                    let patch_button_text = {
                        if let Some(f) = patch_file {
                            f.name()
                        } else {
                            "Open Patch...".into()
                        }
                    };
                    let patch_button = ui.add_enabled(
                        *picked_patch != OpenFileState::Opening,
                        egui::Button::new(patch_button_text),
                    );
                    if patch_button.clicked() {
                        match channels.sender.try_send(ToAppMsg::GetPatchFile) {
                            Ok(_) => {
                                *picked_patch = OpenFileState::Opening;
                            }
                            Err(TrySendError::Full(_)) => {
                                *picked_patch = OpenFileState::None;
                            }
                            Err(TrySendError::Disconnected(_)) => {
                                _frame.close();
                            }
                        };
                    }
                    ui.end_row();

                    ui.label("ISO File:");
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
                        match channels.sender.try_send(ToAppMsg::GetOpenFile) {
                            Ok(_) => {
                                *picked_file = OpenFileState::Opening;
                            }
                            Err(TrySendError::Full(_)) => {
                                *picked_file = OpenFileState::None;
                            }
                            Err(TrySendError::Disconnected(_)) => {
                                _frame.close();
                            }
                        };
                    }
                    ui.end_row();

                    if ui
                        .add_enabled(
                            in_file.is_some() && patch_file.is_some(),
                            egui::Button::new("Patch ISO"),
                        )
                        .clicked()
                    {
                        todo!("Open Save File and launch Patching");
                    }
                });
            });

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

        egui::TopBottomPanel::bottom("status_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label("Status...");
                ui.add(
                    egui::ProgressBar::new(1.0)
                        .animate(true)
                        .text("Loading...")
                        .show_percentage(),
                );
            });
        });

        match channels.receiver.try_recv() {
            Ok(FromAppMsg::Echo) => {
                log::warn!("GUI got Echo");
            }
            Ok(FromAppMsg::NoFileOpened) => {
                *in_file = None;
                *picked_file = OpenFileState::None;
            }
            Ok(FromAppMsg::OpenedFile(file)) => {
                *in_file = Some(InFile::Path(file));
                *picked_file = OpenFileState::Opened;
            }
            Ok(FromAppMsg::NoPatchOpened) => {
                *patch_file = None;
                *picked_patch = OpenFileState::None;
            }
            Ok(FromAppMsg::OpenedPatch(file)) => {
                *patch_file = Some(InFile::Path(file));
                *picked_patch = OpenFileState::Opened;
            }
            Err(TryRecvError::Disconnected) => _frame.close(),
            Err(TryRecvError::Empty) => {}
        };

        let files = ctx.input_mut(|i| i.raw.take().dropped_files);
        for f in files {
            if let Some(path) = &f.path {
                let path_ = path.clone();
                let fd = std::fs::File::open(path);
                match fd {
                    Err(err) => {
                        log::warn!("could not read file at \"{path_:?}\": {err}");
                        break;
                    }
                    Ok(mut file) => {
                        let mut buf = [0u8; 6];
                        if file.read_exact(&mut buf).is_err() {
                            break;
                        }
                        let _ = file.seek(std::io::SeekFrom::Start(0));
                        if [
                            b"RZDE01", b"RZDP01", b"RZDJ01", b"GZ2E01", b"GZ2P01", b"GZ2J01",
                        ]
                        .contains(&&buf)
                        {
                            *in_file = Some(InFile::Dropped(f));
                        } else if buf[..4] == [b'P', b'K', 3, 4] {
                            *patch_file = Some(InFile::Dropped(f));
                        }
                    }
                };
            }
        }
    }
}
