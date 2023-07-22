use num::Unsigned;

/// A trait implemented for 
pub trait Update {
    #[doc = "test"]
    type Error: std::error::Error;

    fn init(&self) -> Result<(), Self::Error>;
    fn prepare(&self) -> Result<(), Self::Error>;
    fn increment(&self, n: impl Unsigned) -> Result<(), Self::Error>;
    fn tick(&self);
    fn finish(&self) -> Result<(), Self::Error>;
    fn reset(&self) -> Result<(), Self::Error>;

    fn set_message(&self, message: impl Into<String>) -> Result<(), Self::Error>;
    fn set_title(&self, title: impl Into<String>) -> Result<(), Self::Error>;
}

pub trait UpdateMut: Update {
    fn init(&mut self) -> Result<(), <Self as Update>::Error> {
        <Self as Update>::init(self)
    }
    fn prepare(&mut self) -> Result<(), Self::Error> {
        <Self as Update>::prepare(self)
    }
    fn increment(&mut self, n: impl Unsigned) -> Result<(), Self::Error> {
        <Self as Update>::increment(self, n)
    }
    fn tick(&self) {
        <Self as Update>::tick(self)
    }
    fn finish(&mut self) -> Result<(), Self::Error> {
        <Self as Update>::finish(self)
    }
    fn reset(&mut self) -> Result<(), Self::Error> {
        <Self as Update>::reset(self)
    }

    fn set_message(&self, message: impl Into<String>) -> Result<(), Self::Error> {
        <Self as Update>::set_message(self, message)
    }
    fn set_title(&self, title: impl Into<String>) -> Result<(), Self::Error> {
        <Self as Update>::set_title(self, title)
    }
}
