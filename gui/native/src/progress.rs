use std::sync::{Arc, Mutex};

use flume::Sender;
use geckolib::{update::{UpdaterType, UpdaterBuilder}, UPDATER};
use lazy_static::lazy_static;

use crate::app::FromAppMsg;

lazy_static! {
    static ref BAR: Arc<Mutex<GuiProgressBar>> = Arc::new(Mutex::new(GuiProgressBar::new(None)));
}

#[derive(Debug)]
pub struct GuiProgressBar {
    status: String,
    pos: usize,
    len: usize,
    type_: UpdaterType,
    sender: Option<Sender<FromAppMsg>>,
}

impl GuiProgressBar {
    pub fn new(sender: Option<Sender<FromAppMsg>>) -> Self {
        GuiProgressBar {
            status: "".into(),
            pos: 0,
            len: 0,
            type_: UpdaterType::default(),
            sender,
        }
    }

    pub fn set_sender(&mut self, sender: Option<Sender<FromAppMsg>>) {
        self.sender = sender;
    }
}

fn send_progress(sender: &Option<Sender<FromAppMsg>>, title: &str, progress: Option<f64>) {
    if let Some(sender) = sender {
        let _ = sender.send(FromAppMsg::Progress(
            if !title.is_empty() {
                Some(title.to_string())
            } else {
                None
            },
            progress.map(|x| x as f32),
        ));
    }
}

fn init_cb(len: Option<usize>) -> eyre::Result<()> {
    match BAR.lock() {
        Ok(mut progress) => {
            progress.pos = 0;
            if let Some(len) = len {
                progress.len = len;
            }
            send_progress(
                &progress.sender,
                &progress.status,
                if progress.type_ == UpdaterType::Progress {
                    Some(progress.pos as f64 / progress.len as f64)
                } else {
                    None
                },
            );
            Ok(())
        }
        Err(err) => Err(eyre::eyre!("{:?}", err)),
    }
}

fn inc_cb(n: usize) -> eyre::Result<()> {
    match BAR.lock() {
        Ok(mut progress) => {
            progress.pos += n;
            send_progress(
                &progress.sender,
                &progress.status,
                if progress.type_ == UpdaterType::Progress {
                    Some(progress.pos as f64 / progress.len as f64)
                } else {
                    None
                },
            );
            Ok(())
        }
        Err(err) => Err(eyre::eyre!("{:?}", err)),
    }
}

fn finish_cb() -> eyre::Result<()> {
    match BAR.lock() {
        Ok(mut progress) => {
            progress.status = "".to_string();
            progress.pos = 0;
            send_progress(
                &progress.sender,
                &progress.status,
                if progress.type_ == UpdaterType::Progress {
                    Some(progress.pos as f64 / progress.len as f64)
                } else {
                    None
                },
            );
            Ok(())
        }
        Err(err) => Err(eyre::eyre!("{:?}", err)),
    }
}

fn reset_cb() -> eyre::Result<()> {
    match BAR.lock() {
        Ok(mut progress) => {
            progress.status = "".to_string();
            progress.pos = 0;
            send_progress(
                &progress.sender,
                &progress.status,
                if progress.type_ == UpdaterType::Progress {
                    Some(progress.pos as f64 / progress.len as f64)
                } else {
                    None
                },
            );
            Ok(())
        }
        Err(err) => Err(eyre::eyre!("{:?}", err)),
    }
}

fn set_pos_cb(pos: usize) -> eyre::Result<()> {
    match BAR.lock() {
        Ok(mut progress) => {
            progress.pos = pos;
            send_progress(
                &progress.sender,
                &progress.status,
                if progress.type_ == UpdaterType::Progress {
                    Some(progress.pos as f64 / progress.len as f64)
                } else {
                    None
                },
            );
            Ok(())
        }
        Err(err) => Err(eyre::eyre!("{:?}", err)),
    }
}

fn on_title_cb(title: String) -> eyre::Result<()> {
    match BAR.lock() {
        Ok(mut progress) => {
            progress.status = title;
            send_progress(
                &progress.sender,
                &progress.status,
                if progress.type_ == UpdaterType::Progress {
                    Some(progress.pos as f64 / progress.len as f64)
                } else {
                    None
                },
            );
            Ok(())
        }
        Err(err) => Err(eyre::eyre!("{:?}", err)),
    }
}

fn on_type_cb(type_: UpdaterType) -> eyre::Result<()> {
    match BAR.lock() {
        Ok(mut progress) => {
            progress.type_ = type_;
            send_progress(
                &progress.sender,
                &progress.status,
                if progress.type_ == UpdaterType::Progress {
                    Some(progress.pos as f64 / progress.len as f64)
                } else {
                    None
                },
            );
            Ok(())
        }
        Err(err) => Err(eyre::eyre!("{:?}", err)),
    }
}

pub fn init_gui_progress(sender: Sender<FromAppMsg>) {
    if let Ok(mut progress) = BAR.lock() {
        progress.set_sender(Some(sender));
    }
    if let Ok(mut updater) = UPDATER.lock() {
        let mut ub = UpdaterBuilder::new();
        ub.init(Some(init_cb))
            .increment(Some(inc_cb))
            .finish(Some(finish_cb))
            .reset(Some(reset_cb))
            .set_pos(Some(set_pos_cb))
            .set_title(Some(on_title_cb))
            .set_type(Some(on_type_cb));
        *updater = ub.build();
    }
}
