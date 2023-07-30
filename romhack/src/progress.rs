use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use geckolib::update::UpdaterType;
use geckolib::UPDATER;
use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};
use lazy_static::lazy_static;

lazy_static! {
    static ref BAR: Arc<Mutex<CLIProgressBar>> = Arc::new(Mutex::new(CLIProgressBar::new()));
}

#[derive(Debug)]
pub struct CLIProgressBar {
    bar: indicatif::ProgressBar,
    title_idx: usize,
    type_: UpdaterType,
}

impl CLIProgressBar {
    pub fn new() -> Self {
        let bar = ProgressBar::hidden();
        CLIProgressBar {
            bar,
            title_idx: 0,
            type_: UpdaterType::default(),
        }
    }
}

impl Default for CLIProgressBar {
    fn default() -> Self {
        Self::new()
    }
}

fn init_cb(len: Option<usize>) -> color_eyre::Result<()> {
    match BAR.lock() {
        Ok(progress) => {
            progress.bar.reset();
            if let Some(len) = len {
                progress.bar.set_length(len as u64);
            }
            progress.bar.set_draw_target(ProgressDrawTarget::stderr());
            progress
                .bar
                .set_prefix(format!("[{}/?]", progress.title_idx));
            progress.bar.enable_steady_tick(Duration::from_millis(200));
            progress.bar.set_style(match progress.type_ {
                UpdaterType::Spinner => {
                    ProgressStyle::with_template("{prefix:.bold.dim} {msg} {spinner}")?
                }
                UpdaterType::Progress => ProgressStyle::with_template(
                    "{spinner} {prefix:.bold.dim} {msg} {wide_bar} {percent}% {human_pos}/{human_len:6}",
                )?,
            });
            Ok(())
        }
        Err(err) => Err(color_eyre::eyre::eyre!("{:?}", err)),
    }
}

fn inc_cb(n: usize) -> color_eyre::Result<()> {
    match BAR.lock() {
        Ok(progress) => {
            progress.bar.inc(n as u64);
            Ok(())
        }
        Err(err) => Err(color_eyre::eyre::eyre!("{:?}", err)),
    }
}

fn tick_cb() {
    if let Ok(progress) = BAR.lock() {
        if !progress.bar.is_hidden() {
            progress.bar.tick();
        }
    }
}

fn finish_cb() -> color_eyre::Result<()> {
    match BAR.lock() {
        Ok(progress) => {
            progress.bar.finish_and_clear();
            Ok(())
        }
        Err(err) => Err(color_eyre::eyre::eyre!("{:?}", err)),
    }
}

fn reset_cb() -> color_eyre::Result<()> {
    match BAR.lock() {
        Ok(mut progress) => {
            progress.title_idx = 0;
            progress.bar.reset();
            Ok(())
        }
        Err(err) => Err(color_eyre::eyre::eyre!("{:?}", err)),
    }
}

fn on_msg_cb(message: String) -> color_eyre::Result<()> {
    match BAR.lock() {
        Ok(progress) => {
            progress.bar.set_message(message.to_string());
            Ok(())
        }
        Err(err) => Err(color_eyre::eyre::eyre!("{:?}", err)),
    }
}

fn on_title_cb(title: String) -> color_eyre::Result<()> {
    match BAR.lock() {
        Ok(mut progress) => {
            progress.title_idx += 1;
            progress
                .bar
                .set_prefix(format!("[{}/?]", progress.title_idx));
            progress.bar.println(title);
            Ok(())
        }
        Err(err) => Err(color_eyre::eyre::eyre!("{:?}", err)),
    }
}

fn on_type_cb(type_: UpdaterType) -> color_eyre::Result<()> {
    match BAR.lock() {
        Ok(mut progress) => {
            progress.type_ = type_;
            progress.bar.set_style(match progress.type_ {
                UpdaterType::Spinner => {
                    ProgressStyle::with_template("{prefix:.bold.dim} {msg} {spinner}")?
                }
                UpdaterType::Progress => ProgressStyle::with_template(
                    "{spinner} {prefix:.bold.dim} {msg} {wide_bar} {percent}% {human_pos}/{human_len:6}",
                )?,
            });
            Ok(())
        }
        Err(err) => Err(color_eyre::eyre::eyre!("{:?}", err)),
    }
}

pub fn init_cli_progress() {
    if let Ok(mut updater) = UPDATER.lock() {
        updater
            .init(Some(init_cb))
            .increment(Some(inc_cb))
            .tick(Some(tick_cb))
            .finish(Some(finish_cb))
            .reset(Some(reset_cb))
            .set_message(Some(on_msg_cb))
            .set_title(Some(on_title_cb))
            .set_type(Some(on_type_cb));
    }
}

#[cfg(disabled)]
impl geckolib::update::UpdateMut for CLIProgressBar {
    type Error = TemplateError;
    type Size = u64;

    fn init(&mut self) -> Result<(), Self::Error> {
        self.bar.reset();
        self.bar.set_draw_target(ProgressDrawTarget::stderr());
        self.bar.set_prefix("[1/?]");
        Ok(())
    }

    fn prepare(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }

    fn increment(&mut self, n: Self::Size) -> Result<(), Self::Error> {
        self.bar.inc(n);
        Ok(())
    }

    fn tick(&mut self) {
        self.bar.tick()
    }

    fn finish(&mut self) -> Result<(), Self::Error> {
        self.bar.finish_and_clear();
        Ok(())
    }

    fn reset(&mut self) -> Result<(), Self::Error> {
        self.bar.reset();
        Ok(())
    }

    fn set_message(&mut self, message: &str) -> Result<(), Self::Error> {
        self.bar.set_message(message.to_string());
        Ok(())
    }

    fn set_type(&mut self, type_: UpdaterType) -> Result<(), Self::Error> {
        self.type_ = type_;
        self.bar.set_style(match self.type_ {
            UpdaterType::Spinner => {
                ProgressStyle::with_template("{prefix:.bold.dim} {msg} {spinner}")?
            }
            UpdaterType::Progress => ProgressStyle::with_template(
                "{prefix:.bold.dim} {msg} {wide_bar} {percent}% {pos}/{len:6}",
            )?,
        });
        Ok(())
    }

    fn set_title(&mut self, title: &str) -> Result<(), Self::Error> {
        self.bar.println(title);
        Ok(())
    }
}
