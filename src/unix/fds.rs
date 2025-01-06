use std::borrow::{Borrow, BorrowMut};
use std::io::{Stdin, Stdout};
// use std::ops::Deref;
use std::os::fd::OwnedFd;
use std::cell::{Ref, RefCell, RefMut};
use std::ops::{Deref, DerefMut};
use std::os::unix::io::{AsRawFd, RawFd};

use nix::libc::{self};
use nix::poll::{PollFlags, PollTimeout};
use nix::pty::OpenptyResult;
use nix::sys::signalfd::SignalFd;
use nix::unistd::{write, Pid};

use log::error;

use termios::Termios;

#[derive(Debug)]
pub enum Fd {
    Signal {
        fd: SignalFd,
        events: PollFlags,
    },
    Stdin {
        fd: Stdin,
        events: PollFlags,
        termios: Termios,
    },
    Stdout {
        fd: Stdout,
        events: PollFlags,
    },
    PtyMaster {
        fd: OwnedFd,
        events: PollFlags,
        child: Pid,
    },
    PtySlave {
        fd: OwnedFd,
        events: PollFlags,
    },
}

impl Fd {
    pub fn as_raw_fd(&self) -> RawFd {
        match self {
            Fd::Signal { fd, .. } => fd.as_raw_fd(),
            Fd::Stdin { fd, .. } => fd.as_raw_fd(),
            Fd::Stdout { fd, .. } => fd.as_raw_fd(),
            Fd::PtyMaster { fd, .. } => fd.as_raw_fd(),
            Fd::PtySlave { fd, .. } => fd.as_raw_fd(),
        }
    }
    pub fn events(&self) -> &PollFlags {
        match self {
            Fd::Signal { events, .. } => events,
            Fd::Stdin { events, .. } => events,
            Fd::Stdout { events, .. } => events,
            Fd::PtyMaster { events, .. } => events,
            Fd::PtySlave { events, .. } => events,
        }
    }
}

#[derive(Debug)]
pub struct Fds {
    inner: Vec<RefCell<Fd>>,
    pollfds: RefCell<Option<Vec<libc::pollfd>>>,
    signalfd_index: Option<usize>,
    stdin_index: Option<usize>,
    stdout_index: Option<usize>,
    pty_master_index: Option<usize>,
    pty_slave_index: Option<usize>,
}

impl Fds {
    pub fn new() -> Self {
        Self {
            inner: vec![],
            pollfds: RefCell::new(None),
            signalfd_index: None,
            stdin_index: None,
            stdout_index: None,
            pty_master_index: None,
            pty_slave_index: None,
        }
    }

    // pub fn stdout_index(self) -> Option<usize> {
    //     self.stdout_index.clone()
    // }

    // pub fn stdin_index(self) -> Option<usize> {
    //     self.stdin_index.clone()
    // }

    /// Возвращает количество файловых дескрипторов в списке
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Возвращает true, если список файловых дескрипторов пуст
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn get_pollfd_by_raw_id<'a>(
        pollfds: &'a mut RefMut<Vec<libc::pollfd>>,
        raw_fd: i32,
    ) -> Option<&'a mut libc::pollfd> {
        let res = pollfds.deref_mut();
        let res = res.iter_mut().find(|x| x.fd == raw_fd);

        res
    }

    /// Возвращает ссылку на файловый дескриптор по индексу
    pub fn get_fd_by_index(&self, index: usize) -> Option<&RefCell<Fd>> {
        self.inner.get(index)
    }

    pub fn get_fd_by_raw_fd(&self, raw_fd: i32) -> Option<&RefCell<Fd>> {
        self.inner
            .iter()
            .filter(|f| (*f).borrow().as_raw_fd() == raw_fd)
            .last()
    }

    /// Метод возвращает массив pollfd, который используется в nix::poll::poll.
    /// Если pollfds не был создан, то он создается и возвращается.
    /// Если pollfds был создан, то возвращается ссылка на него.
    pub fn as_pollfds(&self) -> RefMut<Vec<libc::pollfd>> {
        let res = self.pollfds.borrow_mut().as_deref().is_none();

        if res {
            let fds: Vec<libc::pollfd> = self
                .inner
                .iter()
                .map(|fd| libc::pollfd {
                    fd: fd.borrow().as_raw_fd(),
                    events: fd.borrow().events().bits(),
                    revents: 0,
                })
                .collect();

            self.pollfds.replace(Some(fds));
        }

        let res = RefMut::map(self.pollfds.borrow_mut(), |pollfds| {
            pollfds.as_mut().unwrap()
        });

        res
    }

    fn _push_fd(&mut self, new_fd: Fd) {
        self.inner.push(RefCell::new(new_fd));
        self.pollfds = RefCell::new(None); // Обнуляем кэш, чтобы пересоздать его позже
    }

    /// Добавляет новый файловый дескриптор в список файловых дескрипторов.
    pub fn push_fd(&mut self, new_fd: Fd) {
        match new_fd {
            Fd::Signal { .. } => self._push_fd(new_fd),
            Fd::Stdin { .. } => self._push_fd(new_fd),
            Fd::Stdout { .. } => self._push_fd(new_fd),
            Fd::PtyMaster { .. } => self._push_fd(new_fd),
            Fd::PtySlave { .. } => self._push_fd(new_fd),
        }
    }

    /// Добавляет дескриптор pty (master и slave дестрикторы) в список файловых дскрипторов
    pub fn push_pty_fd(&mut self, pty_fd: OpenptyResult, child: Pid, events: PollFlags) {
        self._push_fd(Fd::PtyMaster {
            fd: pty_fd.master,
            events,
            child,
        });
        self.pty_master_index = Some(self.inner.len() - 1);

        self._push_fd(Fd::PtySlave {
            fd: pty_fd.slave,
            events,
        });
        self.pty_master_index = Some(self.inner.len() - 1);
    }

    /// Добавляет дескриптор сигнала в список файловых дескрипторов
    pub fn push_signal_fd(&mut self, signal_fd: SignalFd, events: PollFlags) {
        self._push_fd(Fd::Signal {
            fd: signal_fd,
            events,
        });
        self.signalfd_index = Some(self.inner.len() - 1);
    }

    /// Добавляет дескриптор stdout в список файловых дескрипторов
    pub fn push_stdout_fd(&mut self, stdout: Stdout, events: PollFlags) {
        self._push_fd(Fd::Stdout { fd: stdout, events });
        self.stdout_index = Some(self.inner.len() - 1);
    }

    /// Добавляет дескриптор stdin в список файловых дескрипторов
    pub fn push_stdin_fd(&mut self, stdin: Stdin, termios: Termios, events: PollFlags) {
        self._push_fd(Fd::Stdin {
            fd: stdin,
            termios,
            events,
        });
        self.stdin_index = Some(self.inner.len() - 1);
    }

    /// Удаляет последний файловый дескриптор из списка файловых дескрипторов
    /// Если список файловых дескрипторов пуст, то ничего не делает
    pub fn pop_fd(&mut self) {
        let res = self.inner.pop();

        if let Some(fd) = res {
            match *fd.borrow() {
                Fd::Signal { .. } => {
                    self.signalfd_index = None;
                }
                Fd::Stdin { .. } => {
                    self.stdin_index = None;
                }
                Fd::Stdout { .. } => {
                    self.stdout_index = None;
                }
                Fd::PtyMaster { .. } => {
                    self.pty_master_index = None;
                }
                Fd::PtySlave { .. } => {
                    self.pty_slave_index = None;
                }
            }

            self.pollfds = RefCell::new(None);
        }
    }

    pub fn send_to(&self, index: usize, buf: &Ref<[u8]>) {
        if let Some(fd) = self.inner.get(index) {
            let mut res = fd.borrow_mut();
            let res = res.deref_mut();
            let res = match res {
                Fd::Signal { fd, .. } => {
                    error!("attempt to send a message to signalfd. this is not possible because signalfd can only be read");
                    write(fd, buf.borrow())
                }
                Fd::Stdin { fd, .. } => {
                    error!("attempt to send a message to signalfd. this is not possible because signalfd can only be read");
                    write(fd, buf.borrow())
                }
                Fd::Stdout { fd, .. } => write(fd, buf.borrow()),
                Fd::PtyMaster { fd, .. } => write(&fd, buf.borrow()),
                Fd::PtySlave { fd, .. } => write(&fd, buf.borrow()),
            };

            if let Err(e) = res {
                error!("error while sending message to fd: {}", e);
            }
        }
    }

    pub fn write_to_stdout(&self, buf: &Ref<[u8]>) {
        if let Some(index) = self.stdout_index {
            self.send_to(index, buf);
        }
    }

    pub fn write_to_stdin(&self, buf: &Ref<[u8]>) {
        if let Some(index) = self.stdin_index {
            self.send_to(index, buf);
        }
    }

    pub fn write_to_pty_master(&self, buf: &Ref<[u8]>) {
        if let Some(index) = self.pty_master_index {
            self.send_to(index, buf);
        }
    }
}

#[derive(Debug)]
pub struct Poller {
    pub fds: Fds,
    pub poll_timeout: PollTimeout,
}

/// Итератор по событиям, возвращаемым poll
/// Будет возвращать только те события, которые были зарегистрированы в poll
/// А именно те, у которых revent != 0
/// Важно! после того как событие будет найдено поле revents будет обнулено
/// Это достигается за счет RefCell
#[derive(Debug)]
pub struct PollReventIterator<'a> {
    fds: &'a Fds,
    index: usize,
}

impl<'a> Iterator for PollReventIterator<'a> {
    type Item = (Ref<'a, Fd>, usize);

    fn next(&mut self) -> Option<Self::Item> {
        let len = self.fds.len();
        while self.index < len {
            let index = self.index;
            self.index += 1;

            let fd = self.fds.get_fd_by_index(index).unwrap();
            let fd = fd.borrow();
            let raw_fd = fd.as_raw_fd();
            let mut res = self.fds.as_pollfds();
            let res = Fds::get_pollfd_by_raw_id(&mut res, raw_fd);

            if let Some(res) = res {
                if res.revents != 0 {
                    res.revents = 0;
                    return Some((fd, index));
                }
            }
        }

        None
    }
}

/// Итератор по файловым дескрипторам
#[derive(Debug)]
pub struct FdsIterator<'b> {
    poller: &'b Poller,
    index: usize,
}

impl<'b> Iterator for FdsIterator<'b> {
    type Item = Ref<'b, Fd>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.index < self.poller.fds.len() {
            let res = self.poller.fds.get_fd_by_index(self.index).unwrap();
            let res = res.borrow();
            self.index += 1;
            return Some(res);
        }

        None
    }
}

impl Poller {
    pub fn new(poll_timeout: PollTimeout) -> Self {
        Self {
            fds: Fds::new(),
            poll_timeout,
        }
    }

    pub fn poll(&self) -> nix::Result<libc::c_int> {
        let res = unsafe {
            libc::poll(
                self.fds.as_pollfds().as_mut_ptr(),
                self.fds.len() as libc::nfds_t,
                i32::from(self.poll_timeout),
            )
        };

        nix::errno::Errno::result(res)
    }

    pub fn revent_iter(&self) -> PollReventIterator {
        PollReventIterator {
            fds: &self.fds,
            index: 0,
        }
    }

    pub fn iter(&self) -> FdsIterator {
        FdsIterator {
            poller: self,
            index: 0,
        }
    }
}
