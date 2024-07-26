#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Ord, Eq, Hash, Default)]
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
    init_cb: Option<InitCallback<E, U>>,
    prepare_cb: Option<DefaultCallback<E>>,
    inc_cb: Option<SizeCallback<E, U>>,
    tick_cb: Option<fn()>,
    finish_cb: Option<DefaultCallback<E>>,
    reset_cb: Option<DefaultCallback<E>>,
    set_pos_cb: Option<SizeCallback<E, U>>,
    set_len_cb: Option<SizeCallback<E, U>>,

    on_msg_cb: Option<StringCallback<E>>,
    on_type_cb: Option<TypeCallback<E>>,
    on_title_cb: Option<StringCallback<E>>,
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
            set_len_cb: Default::default(),
            on_msg_cb: Default::default(),
            on_type_cb: Default::default(),
            on_title_cb: Default::default(),
        }
    }
}

impl<E, U: num::Unsigned> Updater<E, U> {
    pub fn init(&mut self, len: Option<U>) -> Result<(), E> {
        if let Some(init_cb) = self.init_cb {
            return init_cb(len);
        }
        Ok(())
    }
    pub fn prepare(&mut self) -> Result<(), E> {
        if let Some(prepare_cb) = self.prepare_cb {
            return prepare_cb();
        }
        Ok(())
    }
    pub fn increment(&mut self, n: U) -> Result<(), E> {
        if let Some(inc_cb) = self.inc_cb {
            return inc_cb(n);
        }
        Ok(())
    }
    pub fn tick(&mut self) {
        if let Some(tick_cb) = self.tick_cb {
            tick_cb();
        }
    }
    pub fn finish(&mut self) -> Result<(), E> {
        if let Some(finish_cb) = self.finish_cb {
            return finish_cb();
        }
        Ok(())
    }
    pub fn reset(&mut self) -> Result<(), E> {
        if let Some(reset_cb) = self.reset_cb {
            return reset_cb();
        }
        Ok(())
    }
    pub fn set_pos(&mut self, pos: U) -> Result<(), E> {
        if let Some(set_pos_cb) = self.set_pos_cb {
            return set_pos_cb(pos);
        }
        Ok(())
    }

    pub fn set_message(&mut self, msg: String) -> Result<(), E> {
        if let Some(on_msg_cb) = self.on_msg_cb {
            return on_msg_cb(msg);
        }
        Ok(())
    }
    pub fn set_type(&mut self, type_: UpdaterType) -> Result<(), E> {
        if let Some(on_type_cb) = self.on_type_cb {
            return on_type_cb(type_);
        }
        Ok(())
    }
    pub fn set_title(&mut self, title: String) -> Result<(), E> {
        if let Some(on_title_cb) = self.on_title_cb {
            return on_title_cb(title);
        }
        Ok(())
    }
    pub fn set_len(&mut self, len: U) -> Result<(), E> {
        if let Some(set_len_cb) = self.set_len_cb {
            return set_len_cb(len);
        }
        Ok(())
    }
}

#[derive(Debug)]
pub struct UpdaterBuilder<E, U: num::Unsigned = usize> {
    init_cb: Option<InitCallback<E, U>>,
    prepare_cb: Option<DefaultCallback<E>>,
    inc_cb: Option<SizeCallback<E, U>>,
    tick_cb: Option<fn()>,
    finish_cb: Option<DefaultCallback<E>>,
    reset_cb: Option<DefaultCallback<E>>,
    set_pos_cb: Option<SizeCallback<E, U>>,
    set_len_cb: Option<SizeCallback<E, U>>,
    on_msg_cb: Option<StringCallback<E>>,
    on_type_cb: Option<TypeCallback<E>>,
    on_title_cb: Option<StringCallback<E>>,
}

impl<E, U: num::Unsigned> Default for UpdaterBuilder<E, U> {
    fn default() -> Self {
        Self {
            init_cb: Default::default(),
            prepare_cb: Default::default(),
            inc_cb: Default::default(),
            tick_cb: Default::default(),
            finish_cb: Default::default(),
            reset_cb: Default::default(),
            set_pos_cb: Default::default(),
            set_len_cb: Default::default(),
            on_msg_cb: Default::default(),
            on_type_cb: Default::default(),
            on_title_cb: Default::default(),
        }
    }
}

impl<E, U: num::Unsigned> UpdaterBuilder<E, U> {
    pub fn new() -> Self {
        Self::default()
    }

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
    pub fn set_len(&mut self, set_len_cb: Option<SizeCallback<E, U>>) -> &mut Self {
        self.set_len_cb = set_len_cb;
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

    pub fn build(self) -> Updater<E, U> {
        Updater {
            init_cb: self.init_cb,
            prepare_cb: self.prepare_cb,
            inc_cb: self.inc_cb,
            tick_cb: self.tick_cb,
            finish_cb: self.finish_cb,
            reset_cb: self.reset_cb,
            set_pos_cb: self.set_pos_cb,
            set_len_cb: self.set_len_cb,
            on_msg_cb: self.on_msg_cb,
            on_type_cb: self.on_type_cb,
            on_title_cb: self.on_title_cb,
        }
    }
}
