// use std::alloc::{self, AllocError, Layout};
// use std::collections::{HashMap, VecDeque};
// use std::ops::{Deref, DerefMut};
// use std::borrow::{Borrow, BorrowMut};
// use std::boxed::Box;
// use std::cell::{Ref, RefCell};
// use std::io::{Read, Stdin, StdinLock};
// use std::ops::{Deref, DerefMut};
// use std::mem;
// use nix::pty::openpty;
// use nix::sys::eventfd::EventFd;
// use nix::unistd::Pid;
// use nix::unistd::{fork, ForkResult};
// use std::ffi::OsStr;
// use std::os::fd::{AsFd, BorrowedFd, OwnedFd, RawFd};
// use std::os::unix::io::{AsRawFd, FromRawFd};
// use std::os::unix::process::CommandExt;
// use std::process::Stdio;

// use nix::libc::{self, timerfd_settime};
// use nix::poll::PollFlags;
// use nix::sys::signal::{SigSet, Signal};
// use nix::sys::signalfd::{siginfo, SfdFlags, SignalFd};
// use nix::fcntl;
// use nix::sys::termios::{self, ControlFlags, InputFlags, LocalFlags, OutputFlags, SetArg, Termios};
// use nix::sys::time::{TimeSpec, TimeVal, TimeValLike};
// use nix::sys::timer::{Timer, Expiration, TimerSetTimeFlags};
// use nix::sys::timerfd::{TimerFd, ClockId, TimerFlags};

// use log::trace;

// use std::time::{Duration, Instant};

// use nix::errno::Errno;
// use nix::libc;
// use std::os::raw::c_void;
// use std::ptr;
// use std::{collections::HashMap, env, fs};

// use libloading::{Library, Symbol};
// use toml::Value;

// use abstractions::ffi::{CreatePluginFn, PluginInterface};
// use abstractions::{
//     AppShutdown, FdEventHandler, PollErrHandler, PollErrorHandler, PollHupHandler, PollNvalHandler,
//     ReadHandler, UnixContext, ShutdownType,
// };
use abstractions::UnixContext;

use common::plugin::{PluginLoader, ManagedPlugin};

// pub struct OrderedFdEventHandler<C> {
//     order: usize,
//     file_path: String,
//     lib: Library,
//     handler: Box<dyn FdEventHandler<C>>,
// }

// impl<C> OrderedFdEventHandler<C> {
//     pub fn new(order: usize, handler: Box<dyn FdEventHandler<C>>) -> Self {
//         OrderedFdEventHandler {
//             order,
//             handler,
//         }
//     }

//     fn get_order(&self) -> usize {
//         self.order
//     }
// }

// impl<C> FdEventHandler<C> for OrderedFdEventHandler<C> {
//     fn handle(&mut self, app: &mut C, pollfd_index: usize) {
//         self.handler.handle(app, pollfd_index);
//     }

//     fn reg_next(&mut self, next: Box<dyn FdEventHandler<C>>) {
//         self.handler.reg_next(next);
//     }

//     fn reg_pollin(&mut self, handler: Box<dyn ReadHandler<C>>) {
//         self.handler.reg_pollin(handler);
//     }

//     fn reg_pollerr(&mut self, handler: Box<dyn PollErrHandler<C>>) {
//         self.handler.reg_pollerr(handler);
//     }

//     fn reg_pollhup(&mut self, handler: Box<dyn PollHupHandler<C>>) {
//         self.handler.reg_pollhup(handler);
//     }

//     fn reg_pollnval(&mut self, handler: Box<dyn PollNvalHandler<C>>) {
//         self.handler.reg_pollnval(handler);
//     }
// }

// // Реализация PartialEq и Eq для сравнения обработчиков
// impl<C> PartialEq for OrderedFdEventHandler<C> {
//     fn eq(&self, other: &Self) -> bool {
//         self.order == other.order
//     }
// }

// impl<C> Eq for OrderedFdEventHandler<C> {}

// // Реализация PartialOrd и Ord для сортировки
// impl<C> PartialOrd for OrderedFdEventHandler<C> {
//     fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
//         Some(self.order.cmp(&other.order))
//     }
// }

// impl<C> Ord for OrderedFdEventHandler<C> {
//     fn cmp(&self, other: &Self) -> std::cmp::Ordering {
//         self.order.cmp(&other.order)
//     }
// }

pub struct App {
    context: UnixContext,
    handlers: Vec<ManagedPlugin>,
}

impl App {
    pub fn new(context: UnixContext) -> Self {
        let mut res = App {
            context,
            handlers: Vec::new(),
        };

        if let Err(e) = res.load_plugins("config.toml") {
            res.context.shutdown.shutdown_smart();
            res.context.shutdown.set_message(e);
            res.context.shutdown.set_code(-1);
        }

        res
    }

    pub fn exit_code(&self) -> i32 {
        self.context.shutdown.get_code()
    }

    pub fn is_stoped(&self) -> bool {
        self.context.shutdown.is_stoped()
    }

    fn is_stoping_success(&self) -> bool {
        self.handlers.is_empty() && self.context.shutdown.is_stoping()
    }

    pub fn exit_message(&self) -> Option<String> {
        self.context.shutdown.get_message()
    }

    pub fn processing(&mut self) {
        let mut i = 0;
        while i < self.handlers.len() {
            match self.handlers[i].handle(&mut self.context) {
                1 => {
                    self.handlers
                        .remove(i)
                        .free(&mut self.context);
                }
                _ => {
                    i += 1;
                    continue;
                }
            }
        }

        if self.is_stoping_success() {
            self.context.shutdown.to_stoped();
        }
    }

    // Загрузка нового плагина
    pub fn load_plugins(&mut self, config_path: &str) -> Result<(), String> {
        self.handlers = PluginLoader::reload_plugins(config_path, &mut self.context)?;

        Ok(())
    }
}

// #[derive(Debug)]
// pub enum UnixEvent {
//     Event(RawFd),
// }

// #[derive(Debug, Clone)]
// pub enum UnixTask {
//     SmartStop {
//         code: i32,
//         message: Option<String>,
//         start: Instant,
//     },
//     FastStop {
//         code: i32,
//         message: Option<String>,
//         start: Instant,
//     },
//     ImmediateStop {
//         code: i32,
//         message: Option<String>,
//         start: Instant,
//     },
// }

// impl UnixTask {
//     /// возвращает true если на наступило время запуска таска
//     pub fn task_is_ready(&self) -> bool {
//         match self {
//             UnixTask::SmartStop { .. } => true,
//             UnixTask::FastStop { .. } => true,
//             UnixTask::ImmediateStop { .. } => true,
//         }
//     }

//     /// время в которое таск должен быть запущен
//     pub fn scheduled_time(&self) -> Option<Instant> {
//         match self {
//             UnixTask::SmartStop { .. } => None,
//             UnixTask::FastStop { .. } => None,
//             UnixTask::ImmediateStop { .. } => None,
//         }
//     }
// }

// #[derive(Debug, Clone)]
// pub struct UnixQueuePool {
//     queue: VecDeque<UnixTask>,
//     setup_len: usize,
// }

// impl UnixQueuePool {
//     pub fn new(setup_len: usize) -> Self {
//         Self {
//             queue: VecDeque::with_capacity(setup_len),
//             setup_len,
//         }
//     }

//     pub fn try_add_queue(&mut self, queue: UnixTask) -> Result<(), UnixError> {
//         let len = self.queue.len();
//         let iter = 1;
//         if len >= self.setup_len {
//             return Err(UnixError::AllocationError(format!(
//                 "queue is full: {}",
//                 len,
//             )))
//         }

//         if len < self.setup_len {
//             self.queue.try_reserve(iter).map_err(|_| {
//                 UnixError::AllocationError(format!(
//                     "extend queue pool up to: {}",
//                     len+iter
//                 ))
//             })?;
//             self.queue.push_back(queue);
//         }

//         Ok(())
//     }

//     /// Добавляет новый элемент в очередь, удаляя старый при необходимости
//     pub fn add_queue_with_replace_old(&mut self, queue: UnixTask) -> Result<(), UnixError> {
//         if self.queue.len() >= self.setup_len {
//             // Очередь полна, удаляем самый старый элемент
//             self.queue.pop_front();
//         }

//         // Добавляем новый элемент в конец
//         self.queue.push_back(queue);
//         Ok(())
//     }

//     /// Удаляет и возвращает первый элемент (если есть)
//     pub fn pop_task(&mut self) -> Option<UnixTask> {
//         self.queue.pop_front()
//     }

//     /// Возвращает ссылку на первый элемент, не удаляя его
//     pub fn peek_task(&self) -> Option<&UnixTask> {
//         self.queue.front()
//     }
// }
