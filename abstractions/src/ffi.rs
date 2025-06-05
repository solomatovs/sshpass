use nix::libc::c_int;

#[repr(C)]
#[derive(Debug, Clone)]
pub struct PluginInterface<C> {
    pub drop: extern "C" fn(this: *mut PluginInterface<C>, ctx: *mut C) -> c_int,
    pub handle: extern "C" fn(this: *mut PluginInterface<C>, ctx: *mut C) -> c_int,
}

// Определение типа для функции создания плагина
pub type CreatePluginFn<C> = extern "C" fn(ctx: *mut C) -> *mut PluginInterface<C>;


// impl PollHandler {
//     // Конструктор для создания нового обработчика
//     pub fn new() -> Self {
//         PollHandler {
//             handler: None,
//         }
//     }

//     // Устанавливаем обработчик событий (например, основную логику обработки)
//     pub fn set_handle_fn(&mut self, handler: unsafe extern "C" fn(app: *mut UnixContext, res: c_int)) {
//         self.handler = Some(handler);
//     }
// }
