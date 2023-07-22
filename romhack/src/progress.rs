
use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};

#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Ord, Eq, Default)]
pub enum CLIProgressType {
    #[default]
    Spinner,
    Bar,
}

pub struct CLIProgressBar {
    bar: indicatif::ProgressBar,
}

impl CLIProgressBar {
    pub fn new() -> Self {
        let bar = ProgressBar::hidden();
        CLIProgressBar { bar }
    }
}

impl geckolib::update::Update for CLIProgressBar {
    type Error = async_std::io::Error;

    fn init(&self) -> Result<(), Self::Error> {
        let n_group = 0;
        self.bar.reset();
        self.bar.set_draw_target(ProgressDrawTarget::stderr());
        self.bar.set_length(n_group * 2);
        self.bar.set_style(
            ProgressStyle::with_template(
                "{prefix:.bold.dim} {msg} {wide_bar} {percent}% {pos}/{len:6}",
            )
            .map_err(|err| std::io::Error::new(async_std::io::ErrorKind::Other, err))?,
        );
        self.bar.set_prefix("[1/2]");
        self.bar.set_message("Building hash table");
        Ok(())
    }

    fn prepare(&self) -> Result<(), Self::Error> {
        todo!()
    }

    fn increment(&self, n: impl num::Unsigned) -> Result<(), Self::Error> {
        todo!()
    }

    fn tick(&self) {
        todo!()
    }

    fn finish(&self) -> Result<(), Self::Error> {
        todo!()
    }

    fn reset(&self) -> Result<(), Self::Error> {
        todo!()
    }

    fn set_message(&self, message: impl Into<String>) -> Result<(), Self::Error> {
        todo!()
    }

    fn set_title(&self, title: impl Into<String>) -> Result<(), Self::Error> {
        todo!()
    }
}