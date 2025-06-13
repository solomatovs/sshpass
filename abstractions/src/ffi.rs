use nix::libc::{c_int, c_void};

pub trait PluginRust<C> {
    fn handle(&mut self, ctx: &mut C) -> c_int;
    fn free(&mut self, ctx: &mut C) -> c_int;
}

#[repr(C)]
#[derive(Debug, Clone)]
pub struct PluginC<C> {
    pub handle: extern "C" fn(this: *mut PluginC<C>, ctx: *mut C) -> c_int,
    pub free: extern "C" fn(this: *mut PluginC<C>, ctx: *mut C) -> c_int,
}

#[repr(C)]
pub enum PluginType<C> {
    Rust(Box<dyn PluginRust<C>>),
    C(*mut PluginC<C>),
}

#[repr(C)]
pub struct PluginRegistrator<'a, C> {
    ctx: &'a mut C,
    plugins: Vec<PluginType<C>>,
}

impl<C> PluginRegistrator<'_, C> {
    pub fn add_plugin(&mut self, plugin: Box<dyn PluginRust<C>>) {
        self.plugins.push(PluginType::Rust(plugin));
    }

    pub fn get_plugins(self) -> Vec<PluginType<C>> {
        self.plugins
    }

    pub fn get_context(&mut self) -> &mut C {
        self.ctx
    }
}


#[repr(C)]
pub struct PluginRegistratorCInterface<C> {
    pub this: *mut c_void,
    pub add_plugin: extern "C" fn(*mut c_void, *mut PluginC<C>),
}

extern "C" fn add_plugin<C>(this: *mut c_void, plugin: *mut PluginC<C>) {
    let registrar: &mut PluginRegistrator<C> = unsafe {
        &mut *(this as *mut PluginRegistrator<C>)
    };

    if plugin.is_null() {
        panic!("Plugin pointer is null");
    }

    registrar.plugins.push(PluginType::C(plugin));
}

impl<'a, C> PluginRegistrator<'a, C> {
    pub fn new(ctx: &'a mut C) -> Self {
        Self {
            ctx,
            plugins: Vec::new(),
        }
    }

    pub fn as_c_interface(&mut self) -> *mut PluginRegistratorCInterface<C> {
        let res = PluginRegistratorCInterface {
            add_plugin: add_plugin::<C>,
            this: Box::into_raw(Box::new(self)) as *mut c_void,
        };

        Box::into_raw(Box::new(res))
    }
}


// Определение типа для функции создания плагина
pub type RegisterRustPluginFn<C> = extern "Rust" fn(&mut PluginRegistrator<C>) -> Result<(), String>;
pub type RegisterCPluginFn<C> = extern "C" fn(registrator: *mut PluginRegistratorCInterface<C>) -> bool;
