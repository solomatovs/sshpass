use abstractions::{UnixContext, ShutdownType, Buffer};
use log::info;
use std::os::raw::c_int;
use std::os::fd::RawFd;
use nix::poll::PollFlags;
use std::os::unix::io::AsRawFd;

use nix::fcntl;

use nix::sys::signal::{SigSet, Signal};
use nix::sys::signalfd::{siginfo, SfdFlags, SignalFd};

use common::init_log::init_log;


#[repr(C)]
#[derive(Debug, Clone)]
pub struct PluginInterface<C> {
    pub drop: extern "C" fn(this: &mut MyPlugin, ctx: &mut C) -> c_int,
    pub handle: extern "C" fn(this: &mut MyPlugin, ctx: &mut C) -> c_int,
}

// Определяем структуру для нашего плагина в Rust-стиле
// Первым полем должно быть поле PluginInterface, которое содержит функции drop и handle
#[repr(C)]
pub struct MyPlugin {
    plugin: PluginInterface<UnixContext>,
    fd: SignalFd,
    buf: Buffer,
}

impl MyPlugin {
    pub fn new(ctx: &mut UnixContext) -> Self {
        init_log();

        info!("signal: plugin initializing");

        let (fd, buf) = match Self::get_signal_fd(ctx) {
            Ok(x) => x,
            Err(e) => {
                panic!("Error getting signal fd: {}", e)
            }
        };
        
        MyPlugin {
            plugin: PluginInterface::<UnixContext> {
                drop: Self::drop,
                handle: Self::handle,
            },
            fd,
            buf,
        }
    }

    pub extern "C" fn drop(&mut self, ctx: &mut UnixContext) -> c_int {
        info!("signal: plugin cleaning up");

        if !ctx.poll.remove_fd(self.fd.as_raw_fd()) {
            return 1;
        }

        0 // 0 означает успешное освобождение
    }

    pub extern "C" fn handle(&mut self, ctx: &mut UnixContext) -> c_int {
        if ctx.shutdown.is_stoping() {
            return ShutdownType::Stoped.to_int();
        }

        0 // 0 означает успешное выполнение
    }

    fn _is_valid_fd(&self, fd: RawFd) -> bool {
        let mut res = fcntl::fcntl(fd, fcntl::F_GETFD);

        // запрашиваю до тех пор, пока приходит EINTR
        // так как это означает что вызов fcntl был прерван сигналом и надо повторить попытку
        while let Err(nix::Error::EINTR) = res {
            res = fcntl::fcntl(fd, fcntl::F_GETFD);
        }

        if res.is_ok() {
            return true;
        }

        false
    }

    fn get_signal_fd(ctx: &mut UnixContext) -> Result<(SignalFd, Buffer), String> {
        let buffer_length = std::mem::size_of::<siginfo>();

        let buf = Buffer::try_new(buffer_length).map_err(|_e| {
            format!(
                "signal fd buffer allocation error: {} bytes",
                buffer_length
            )
        })?;
        
        let mut mask = SigSet::empty();

        // добавляю в обработчик все сигналы
        for signal in Signal::iterator() {
            if matches!(signal, Signal::SIGKILL | Signal::SIGSTOP) {
                continue;
            }

            mask.add(signal);
        }

        let mut new_mask = SigSet::thread_get_mask().map_err(|e| format!("failed get thread mask: {:#?}", e))?;
        for s in mask.into_iter() {
            new_mask.add(s);
        }

        new_mask
            .thread_block()
            .map_err(|e| format!("failed set thread mask: {:#?}", e))?;

        let fd: SignalFd = SignalFd::with_flags(&new_mask, SfdFlags::SFD_NONBLOCK | SfdFlags::SFD_CLOEXEC)
            .map_err(|e| {
                format!("signalfd create failed error: {:#?}", e)
            })?;

        let flags = PollFlags::POLLIN | PollFlags::POLLERR | PollFlags::POLLHUP | PollFlags::POLLNVAL;

        ctx.poll.add_fd(fd.as_raw_fd(), flags.bits());

        Ok((fd, buf))
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
