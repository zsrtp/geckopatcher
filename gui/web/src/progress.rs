use std::sync::{Arc, Mutex};

use geckolib::{
    update::{UpdaterBuilder, UpdaterType},
    UPDATER,
};
use js_sys::Reflect;
use lazy_static::lazy_static;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::DedicatedWorkerGlobalScope;

lazy_static! {
    static ref BAR: Arc<Mutex<WebProgressBar>> = Arc::new(Mutex::new(WebProgressBar::new()));
}

#[derive(Debug)]
pub struct WebProgressBar {
    status: String,
    pos: usize,
    len: usize,
    type_: UpdaterType,
}

impl WebProgressBar {
    pub fn new() -> Self {
        WebProgressBar {
            status: "".into(),
            pos: 0,
            len: 0,
            type_: UpdaterType::default(),
        }
    }
}

impl Default for WebProgressBar {
    fn default() -> Self {
        Self::new()
    }
}

fn send_progress(title: &str, progress: Option<f64>) {
    let glob: DedicatedWorkerGlobalScope = js_sys::global().dyn_into().unwrap();
    let obj = js_sys::Object::new();
    let _ = Reflect::set(&obj, &"type".into(), &"progress".into());
    if !title.is_empty() {
        let _ = Reflect::set(&obj, &"title".into(), &JsValue::from_str(title));
    } else {
        let _ = Reflect::set(&obj, &"title".into(), &JsValue::null());
    }
    if let Some(progress) = progress {
        let _ = Reflect::set(&obj, &"progress".into(), &JsValue::from_f64(progress));
    } else {
        let _ = Reflect::set(&obj, &"progress".into(), &JsValue::null());
    }
    let _ = glob.post_message(&obj);
}

fn init_cb(len: Option<usize>) -> eyre::Result<()> {
    match BAR.lock() {
        Ok(mut progress) => {
            progress.pos = 0;
            if let Some(len) = len {
                progress.len = len;
            }
            send_progress(
                &progress.status,
                if progress.type_ == UpdaterType::Progress {
                    Some(progress.pos as f64 / progress.len as f64 * 100.0f64)
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
                &progress.status,
                if progress.type_ == UpdaterType::Progress {
                    Some(progress.pos as f64 / progress.len as f64 * 100.0f64)
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
                &progress.status,
                if progress.type_ == UpdaterType::Progress {
                    Some(progress.pos as f64 / progress.len as f64 * 100.0f64)
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
                &progress.status,
                if progress.type_ == UpdaterType::Progress {
                    Some(progress.pos as f64 / progress.len as f64 * 100.0f64)
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
                &progress.status,
                if progress.type_ == UpdaterType::Progress {
                    Some(progress.pos as f64 / progress.len as f64 * 100.0f64)
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
                &progress.status,
                if progress.type_ == UpdaterType::Progress {
                    Some(progress.pos as f64 / progress.len as f64 * 100.0f64)
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
                &progress.status,
                if progress.type_ == UpdaterType::Progress {
                    Some(progress.pos as f64 / progress.len as f64 * 100.0f64)
                } else {
                    None
                },
            );
            Ok(())
        }
        Err(err) => Err(eyre::eyre!("{:?}", err)),
    }
}

pub fn init_web_progress() {
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
