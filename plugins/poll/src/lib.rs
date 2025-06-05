use abstractions::{UnixContext, ShutdownType};
use log::{error, info, trace};
use nix::errno::Errno;
use nix::libc;
use std::os::raw::c_int;

use common::init_log::init_log;

#[repr(C)]
#[derive(Debug, Clone)]
pub struct PluginInterface<C> {
    pub drop: extern "C" fn(this: &mut MyPlugin, ctx: &mut C) -> c_int,
    pub handle: extern "C" fn(this: &mut MyPlugin, ctx: &mut C) -> c_int,
}


// Определяем структуру для нашего плагина в Rust-стиле
#[repr(C)]
pub struct MyPlugin {
    plugin: PluginInterface<UnixContext>,
    counter: i32,
}

impl MyPlugin {
    pub fn new(_ctx: &mut UnixContext) -> Self {
        init_log();

        info!("poll: plugin initializing");

        MyPlugin {
            plugin: PluginInterface::<UnixContext> {
                drop: Self::drop,
                handle: Self::handle,
            },
            counter: 0,
        }
    }
    pub extern "C" fn handle(&mut self, ctx: &mut UnixContext) -> c_int {
        if ctx.shutdown.is_stoping() {
            return ShutdownType::Stoped.to_int();
        }

        let res = unsafe {
            libc::poll(
                ctx.poll.as_raw_mut().fds_ptr,
                ctx.poll.len() as libc::nfds_t,
                ctx.poll.timeout(),
            )
        };

        match Errno::result(res) {
            // poll error, handling
            Err(e) => {
                error!("poll: error {}", e);
                ctx.shutdown.to_smart_stop();
                ctx.shutdown.set_message(e.desc().into());
            }
            // poll recv event, handling
            Ok(number_events) => {
                trace!("poll: received {} events", number_events);
                ctx.poll.set_result(number_events);
            }
        }

        if self.counter > 1000 {
            ctx.shutdown.to_smart_stop();
        }

        self.counter += 1;

        0 // 0 означает успешное выполнение
    }

    pub extern "C" fn drop(&mut self, _ctx: &mut UnixContext) -> c_int {
        info!("poll: plugin cleaning up");
        // Освобождение ресурсов, если нужно
        0 // 0 означает успешное освобождение
    }
}

/// Creates a new instance of MyPlugin.
/// 
/// # Safety
/// 
/// The caller must ensure that:
/// - `ctx` is a valid, non-null pointer to a properly initialized `UnixContext`
/// - The `UnixContext` pointed to by `ctx` remains valid for the duration of the call
/// - The `UnixContext` is not being mutably accessed from other parts of the code during this call
#[no_mangle]
pub unsafe extern "C" fn new(ctx: *mut UnixContext) -> *mut MyPlugin {
    if ctx.is_null() {
        return std::ptr::null_mut();
    }
    
    match std::panic::catch_unwind(|| {
        MyPlugin::new(&mut *ctx)
    }) {
        Ok(plugin) => Box::into_raw(Box::new(plugin)),
        Err(_) => std::ptr::null_mut()
    }
}
