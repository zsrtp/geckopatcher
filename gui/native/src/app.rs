use std::io::{Read, Seek};

use egui::Vec2;
use flume::{Receiver, Sender, TryRecvError, TrySendError};
use futures_lite::future::block_on;
use geckolib::async_file::AsyncFile;
use geckolib::iso::builder::Builder;
use geckolib::{open_config_from_patch, UPDATER};
use regex::Regex;
use std::path::PathBuf;

use crate::progress::init_gui_progress;

#[derive(Debug)]
pub enum InFile {
    Dropped(egui::DroppedFile),
    Path(PathBuf),
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
            InFile::Path(p) => p
                .file_name()
                .and_then(|n| n.to_str())
                .map(|s| s.to_string())
                .unwrap_or("<no name>".to_string()),
        }
    }

    fn path(&self) -> PathBuf {
        match self {
            InFile::Path(p) => p.clone(),
            InFile::Dropped(file) => file.path.clone().unwrap(),
        }
    }
}

#[derive(Debug)]
pub enum ToAppMsg {
    GetIso,
    GetPatch,
    PatchAndSave(PathBuf, PathBuf),
}

#[derive(Debug)]
pub enum FromAppMsg {
    Progress(Option<String>, Option<f32>),
    OpenedFile(PathBuf),
    OpenedPatch(PathBuf),
    SaveSelected,
    FinishedSave,
    NoFileOpened,
    NoPatchOpened,
    NoSaveOpened,
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
    is_patching: bool,
    status: Option<String>,
    progress: Option<f32>,
    game_code_regex: Regex,
}

async fn apply(patch: PathBuf, iso: PathBuf, save: PathBuf) -> Result<(), eyre::Error> {
    let patch = std::fs::OpenOptions::new().read(true).open(patch)?;
    let iso = AsyncFile::new(std::fs::OpenOptions::new().read(true).open(iso)?);
    let save = AsyncFile::new(
        std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(save)?,
    );
    let mut builder = open_config_from_patch(patch, iso, save).await?;
    builder.build().await?;
    Ok(())
}

fn patcher_thread(snd: Sender<FromAppMsg>, rcv: Receiver<ToAppMsg>) {
    init_gui_progress(snd.clone());

    loop {
        match rcv.recv() {
            Err(flume::RecvError::Disconnected) => {
                log::warn!("Thread's receiver disconnected");
                return;
            }
            Ok(ToAppMsg::GetPatch) => {
                match rfd::FileDialog::new()
                    .add_filter("application/zip", &["patch"])
                    .add_filter("All Files", &["*"])
                    .pick_file()
                {
                    Some(path) => {
                        log::debug!("Got a patch file from the user!");
                        if snd.send(FromAppMsg::OpenedPatch(path)).is_err() {
                            log::error!("could not send OpenedPatch");
                            return;
                        }
                    }
                    None => {
                        if snd.send(FromAppMsg::NoPatchOpened).is_err() {
                            log::error!("could not send NoPatchOpened");
                            return;
                        }
                    }
                };
            }
            Ok(ToAppMsg::GetIso) => {
                match rfd::FileDialog::new()
                    .add_filter("application/x-cd-image", &["iso"])
                    .add_filter("All Files", &["*"])
                    .pick_file()
                {
                    Some(path) => {
                        log::debug!("Got a file from the user!");
                        if snd.send(FromAppMsg::OpenedFile(path)).is_err() {
                            log::error!("could not send OpenedFile");
                            return;
                        }
                    }
                    None => {
                        if snd.send(FromAppMsg::NoFileOpened).is_err() {
                            log::error!("could not send NoFileOpened");
                            return;
                        }
                    }
                };
            }
            Ok(ToAppMsg::PatchAndSave(patch, iso)) => {
                match rfd::FileDialog::new()
                    .add_filter("application/x-cd-image", &["iso"])
                    .add_filter("All Files", &["*"])
                    .set_file_name("output.iso")
                    .save_file()
                {
                    Some(save) => {
                        log::debug!("Got a file from the user! Starting patching");
                        if snd
                            .send(FromAppMsg::Progress(
                                Some("Starting process".to_string()),
                                None,
                            ))
                            .is_err()
                        {
                            log::error!("Could not send Progress (update gui)");
                            return;
                        }
                        // reset updater
                        if let Ok(mut updater) = UPDATER.lock() {
                            let _ = updater.reset();
                        }
                        if let Err(err) = block_on(apply(patch, iso, save)) {
                            log::error!("{:?}", err);
                            if snd
                                .send(FromAppMsg::Progress(Some(format!("{}", err)), None))
                                .is_err()
                            {
                                log::error!("could not send Progress (error in reproc)");
                                return;
                            }
                            if snd.send(FromAppMsg::NoSaveOpened).is_err() {
                                log::error!("could not send NoSaveOpened (error in reproc)");
                                return;
                            }
                        } else {
                            if snd.send(FromAppMsg::Progress(None, None)).is_err() {
                                log::trace!("could not send Progress");
                            }
                            log::info!("reproc done");
                            if snd.send(FromAppMsg::FinishedSave).is_err() {
                                log::error!("could not send FinishedSave");
                                return;
                            }
                        }
                    }
                    None => {
                        if snd.send(FromAppMsg::NoSaveOpened).is_err() {
                            log::error!("could not send NoSaveOpened");
                            return;
                        }
                    }
                };
            }
        }
    }
}

impl PatcherApp {
    /// Called once before the first frame.
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        // This is also where you can customize the look and feel of egui using
        // `cc.egui_ctx.set_visuals` and `cc.egui_ctx.set_fonts`.

        // Start the parallel async thread which will handle all the asynchronous tasks.
        let (snd_app, rcv_ui) = flume::unbounded::<FromAppMsg>();
        let (snd_ui, rcv_app) = flume::unbounded::<ToAppMsg>();
        std::thread::spawn(move || patcher_thread(snd_app, rcv_app));
        Self {
            patch_file: None,
            in_file: None,
            channels: MessageChannel {
                sender: snd_ui,
                receiver: rcv_ui,
            },
            picked_patch: Default::default(),
            picked_file: Default::default(),
            is_patching: false,
            status: None,
            progress: None,
            game_code_regex: Regex::new(
                "^((([RSGUDP0124])([A-Z0-9]{2})([DEFIJKPRSTU]))([A-Z0-9]{2}))",
            )
            .expect("Couldn't parse the GameCode RegEx"),
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
            is_patching,
            status,
            progress,
            game_code_regex: re,
        } = self;

        egui_extras::install_image_loaders(ctx);

        // Examples of how to create different panels and windows.
        // Pick whichever suits you.
        // Tip: a good default choice is to just keep the `CentralPanel`.
        // For inspiration and more examples, go to https://emilk.github.io/egui

        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            // The top panel is often a good place for a menu bar:
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("Quit").clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
            });
        });

        egui::TopBottomPanel::bottom("status_bar").show(ctx, |ui| {
            if status.is_some() || progress.is_some() {
                ui.horizontal(|ui| {
                    if let Some(status) = status {
                        ui.label(status.clone());
                    }
                    if let Some(progress) = progress {
                        ui.add(
                            egui::ProgressBar::new(*progress)
                                .text("Loading...")
                                .show_percentage()
                                .animate(true),
                        );
                    }
                });
            }
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.vertical_centered_justified(|ui| {
                ui.group(|ui| {
                    ui.heading("ISO Patching");
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
                            *picked_patch != OpenFileState::Opening && !*is_patching,
                            egui::Button::new(patch_button_text),
                        );
                        if patch_button.clicked() {
                            match channels.sender.try_send(ToAppMsg::GetPatch) {
                                Ok(_) => {
                                    *picked_patch = OpenFileState::Opening;
                                }
                                Err(TrySendError::Full(_)) => {
                                    *picked_patch = OpenFileState::None;
                                }
                                Err(TrySendError::Disconnected(_)) => {
                                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
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
                                *picked_file != OpenFileState::Opening && !*is_patching,
                                egui::Button::new(file_button_text),
                            )
                            .clicked()
                        {
                            match channels.sender.try_send(ToAppMsg::GetIso) {
                                Ok(_) => {
                                    *picked_file = OpenFileState::Opening;
                                }
                                Err(TrySendError::Full(_)) => {
                                    *picked_file = OpenFileState::None;
                                }
                                Err(TrySendError::Disconnected(_)) => {
                                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                                }
                            };
                        }
                        ui.end_row();

                        if ui
                            .add_enabled(
                                in_file.is_some() && patch_file.is_some() && !*is_patching,
                                egui::Button::new("Patch ISO"),
                            )
                            .clicked()
                        {
                            match channels.sender.try_send(ToAppMsg::PatchAndSave(
                                patch_file.as_ref().unwrap().path(),
                                in_file.as_ref().unwrap().path(),
                            )) {
                                Ok(_) => {
                                    *is_patching = true;
                                }
                                Err(TrySendError::Full(_)) => {}
                                Err(TrySendError::Disconnected(_)) => {
                                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                                }
                            };
                        }
                    });
                });
            });

            ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                ui.horizontal(|ui| {
                    ui.add(
                        egui::Image::new(if ui.style().visuals.dark_mode {
                            egui::include_image!("../assets/github-mark-white.png")
                        } else {
                            egui::include_image!("../assets/github-mark.png")
                        })
                        .fit_to_exact_size(Vec2 { x: 15.0, y: 15.0 }),
                    );
                    ui.hyperlink_to("Github", "https://github.com/kipcode66/geckopatcher")
                });
                egui::warn_if_debug_build(ui);
            });
        });

        loop {
            match channels.receiver.try_recv() {
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
                Ok(FromAppMsg::NoSaveOpened) => {
                    *is_patching = false;
                }
                Ok(FromAppMsg::FinishedSave) => {
                    *is_patching = false;
                    status.replace("Done".into());
                }
                Ok(FromAppMsg::Progress(status_, progress_)) => {
                    *status = status_;
                    *progress = progress_;
                }
                Ok(FromAppMsg::SaveSelected) => {
                    ctx.request_repaint();
                    ctx.send_viewport_cmd(egui::ViewportCommand::RequestUserAttention(
                        egui::UserAttentionType::Informational,
                    ));
                }
                Err(TryRecvError::Disconnected) => {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close)
                }
                Err(TryRecvError::Empty) => {
                    break;
                }
            };
        }

        if !*is_patching {
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
                            let is_game = re.is_match(&String::from_utf8_lossy(&buf));
                            if is_game {
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
}
