#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Ord, Eq, Default)]
pub enum UpdaterType {
    #[default]
    Spinner,
    Progress,
}

pub type InitCallback<E, U> = fn(Option<U>) -> Result<(), E>;
pub type DefaultCallback<E> = fn() -> Result<(), E>;
pub type StringCallback<E> = fn(String) -> Result<(), E>;
pub type TypeCallback<E> = fn(UpdaterType) -> Result<(), E>;
pub type SizeCallback<E, U> = fn(U) -> Result<(), E>;

#[derive(Debug)]
pub struct Updater<E, U: num::Unsigned = usize> {
    pub(crate) init_cb: Option<InitCallback<E, U>>,
    pub(crate) prepare_cb: Option<DefaultCallback<E>>,
    pub(crate) inc_cb: Option<SizeCallback<E, U>>,
    pub(crate) tick_cb: Option<fn()>,
    pub(crate) finish_cb: Option<DefaultCallback<E>>,
    pub(crate) reset_cb: Option<DefaultCallback<E>>,
    pub(crate) set_pos_cb: Option<SizeCallback<E, U>>,

    pub(crate) on_msg_cb: Option<StringCallback<E>>,
    pub(crate) on_type_cb: Option<TypeCallback<E>>,
    pub(crate) on_title_cb: Option<StringCallback<E>>,
}

impl<E, U: num::Unsigned> Default for Updater<E, U> {
    fn default() -> Self {
        Self {
            init_cb: Default::default(),
            prepare_cb: Default::default(),
            inc_cb: Default::default(),
            tick_cb: Default::default(),
            finish_cb: Default::default(),
            reset_cb: Default::default(),
            set_pos_cb: Default::default(),
            on_msg_cb: Default::default(),
            on_type_cb: Default::default(),
            on_title_cb: Default::default(),
        }
    }
}

impl<E, U: num::Unsigned> Updater<E, U> {
    pub fn init(&mut self, init_cb: Option<InitCallback<E, U>>) -> &mut Self {
        self.init_cb = init_cb;
        self
    }
    pub fn prepare(&mut self, prepare_cb: Option<DefaultCallback<E>>) -> &mut Self {
        self.prepare_cb = prepare_cb;
        self
    }
    pub fn increment(&mut self, inc_cb: Option<fn(U) -> Result<(), E>>) -> &mut Self {
        self.inc_cb = inc_cb;
        self
    }
    pub fn tick(&mut self, tick_cb: Option<fn()>) -> &mut Self {
        self.tick_cb = tick_cb;
        self
    }
    pub fn finish(&mut self, finish_cb: Option<DefaultCallback<E>>) -> &mut Self {
        self.finish_cb = finish_cb;
        self
    }
    pub fn reset(&mut self, reset_cb: Option<DefaultCallback<E>>) -> &mut Self {
        self.reset_cb = reset_cb;
        self
    }
    pub fn set_pos(&mut self, set_pos_cb: Option<SizeCallback<E, U>>) -> &mut Self {
        self.set_pos_cb = set_pos_cb;
        self
    }

    pub fn set_message(&mut self, on_msg_cb: Option<fn(String) -> Result<(), E>>) -> &mut Self {
        self.on_msg_cb = on_msg_cb;
        self
    }
    pub fn set_type(&mut self, on_type_cb: Option<fn(UpdaterType) -> Result<(), E>>) -> &mut Self {
        self.on_type_cb = on_type_cb;
        self
    }
    pub fn set_title(&mut self, on_title_cb: Option<fn(String) -> Result<(), E>>) -> &mut Self {
        self.on_title_cb = on_title_cb;
        self
    }
}
