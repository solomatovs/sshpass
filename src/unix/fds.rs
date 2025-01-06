use std::borrow::{Borrow, BorrowMut};
use std::io::{Stdin, Stdout, Write};
// use std::ops::Deref;
// use std::os::fd;
use std::cell::{RefCell, RefMut, Ref};
use std::ops::DerefMut;
use std::os::unix::io::{AsRawFd, RawFd};

use nix::libc::{self, pollfd};
use nix::poll::{PollFlags, PollTimeout};
use nix::pty::OpenptyResult;
use nix::sys::signalfd::SignalFd;
use nix::unistd::{write, Pid};

use log::{trace, error, debug, info};

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
        fd: OpenptyResult,
        events: PollFlags,
        child: Pid,
    },
}

impl Fd {
    pub fn as_raw_fd(&self) -> RawFd {
        match self {
            Fd::Signal { fd, .. } => fd.as_raw_fd(),
            Fd::Stdin { fd, .. } => fd.as_raw_fd(),
            Fd::Stdout { fd, .. } => fd.as_raw_fd(),
            Fd::PtyMaster { fd, .. } => fd.master.as_raw_fd(),
        }
    }
    pub fn events(&self) -> &PollFlags {
        match self {
            Fd::Signal { events, .. } => events,
            Fd::Stdin { events, .. } => events,
            Fd::Stdout { events, .. } => events,
            Fd::PtyMaster { events, .. } => events,
        }
    }
}

#[derive(Debug)]
pub struct Fds {
    inner: Vec<Fd>,
    pollfds: RefCell<Option<Vec<libc::pollfd>>>,
    signalfd_index: Option<usize>,
    stdin_index: Option<usize>,
    stdout_index: Option<usize>,
}

// impl Deref for Fds {
//     type Target = Vec<Fd>;

//     fn deref(&self) -> &Self::Target {
//         &self.inner
//     }
// }

impl Fds {
    pub fn new() -> Self {
        Self {
            inner: vec![],
            pollfds: RefCell::new(None),
            signalfd_index: None,
            stdin_index: None,
            stdout_index: None,
        }
    }

    /// Возвращает количество файловых дескрипторов в списке
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Возвращает true, если список файловых дескрипторов пуст
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn get_pollfd_by_raw_id<'a>(pollfds: &'a mut RefMut<Vec<libc::pollfd>>, raw_fd: i32) -> Option<&'a mut libc::pollfd> {
        let res = pollfds.deref_mut();
        let res = res.iter_mut().find(|x| x.fd == raw_fd);

        res
    }

    /// Возвращает ссылку на файловый дескриптор по индексу
    pub fn get_fd_by_index(&self, index: usize) -> Option<&Fd> {
        self.inner.get(index)
    }

    pub fn get_fd_by_raw_fd(&self, raw_fd: i32) -> Option<&Fd> {
        self.inner.iter().filter(|f| f.as_raw_fd() == raw_fd).last()
    }

    /// Метод возвращает массив pollfd, который используется в nix::poll::poll.
    /// Если pollfds не был создан, то он создается и возвращается.
    /// Если pollfds был создан, то возвращается ссылка на него.
    pub fn as_pollfds(&self) -> RefMut<Vec<libc::pollfd>> {
        let res = self.pollfds.borrow_mut().as_deref().is_none();
        
        if res {
            let fds: Vec<libc::pollfd> = self.inner
                .iter()
                .map(|fd| libc::pollfd {
                    fd: fd.as_raw_fd(),
                    events: fd.events().bits(),
                    revents: 0,
                })
                .collect();

            self.pollfds.replace(Some(fds));
        }

        let res = RefMut::map(
            self.pollfds.borrow_mut(),
            |pollfds| pollfds.as_mut().unwrap(),
        );

        return res;


            // self.pollfds
            // .get_or_insert_with(|| {
            //     self.inner
            //         .iter()
            //         .map(|fd| libc::pollfd {
            //             fd: fd.as_raw_fd(),
            //             events: fd.events().bits(),
            //             revents: 0,
            //         })
            //         .collect()
            // })
            // .as_mut_slice()
    }

    fn _push_fd(&mut self, new_fd: Fd) {
        self.inner.push(new_fd);
        self.pollfds = RefCell::new(None); // Обнуляем кэш, чтобы пересоздать его позже
    }

    /// Добавляет новый файловый дескриптор в список файловых дескрипторов.
    pub fn push_fd(&mut self, new_fd: Fd) {
        match new_fd {
            Fd::Signal { .. } => self._push_fd(new_fd),
            Fd::Stdin { .. } => self._push_fd(new_fd),
            Fd::Stdout { .. } => self._push_fd(new_fd),
            Fd::PtyMaster { .. } => self._push_fd(new_fd),
        }
    }

    /// Добавляет дескриптор pty (master и slave дестрикторы) в список файловых дскрипторов
    pub fn push_pty_fd(&mut self, pty_fd: OpenptyResult, child: Pid, events: PollFlags) {
        self._push_fd(Fd::PtyMaster {
            fd: pty_fd,
            events,
            child,
        });
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

    /// Удаляет дескриптор сигнала из списка файловых дескрипторов
    pub fn remove_signal_fd(&mut self) {
        if let Some(index) = self.signalfd_index {
            self.inner.remove(index);
            self.signalfd_index = None;
            self.pollfds = RefCell::new(None);
        }
    }

    /// Удаляет дескриптор stdin из списка файловых дескрипторов
    pub fn remove_stdin_fd(&mut self) {
        if let Some(index) = self.stdin_index {
            self.inner.remove(index);
            self.stdin_index = None;
            self.pollfds = RefCell::new(None);
        }
    }

    /// Удаляет дескриптор stdout из списка файловых дескрипторов
    pub fn remove_stdout_fd(&mut self) {
        if let Some(index) = self.stdout_index {
            self.inner.remove(index);
            self.stdout_index = None;
            self.pollfds = RefCell::new(None);
        }
    }

    /// Удаляет последний файловый дескриптор из списка файловых дескрипторов
    /// Если список файловых дескрипторов пуст, то ничего не делает
    pub fn pop_fd(&mut self) {
        if let Some(fd) = self.inner.last() {
            match fd {
                Fd::Signal { .. } => self.remove_signal_fd(),
                Fd::Stdin { .. } => self.remove_stdin_fd(),
                Fd::Stdout { .. } => self.remove_stdout_fd(),
                Fd::PtyMaster { .. } => {
                    self.inner.pop();
                    self.pollfds = RefCell::new(None);
                }
            }
        }
    }

    pub fn send_to(&mut self, index: usize, buf: &[u8]) {
        if let Some(fd) = self.inner.get_mut(index) {
            let res = match fd {
                Fd::Signal { fd, .. } => {
                    error!("attempt to send a message to signalfd. this is not possible because signalfd can only be read");
                    write(fd, buf)
                },
                Fd::Stdin { fd, .. } => {
                    error!("attempt to send a message to signalfd. this is not possible because signalfd can only be read");
                    write(fd, buf)
                },
                Fd::Stdout { fd, .. } => {
                    write(fd, buf)
                },
                Fd::PtyMaster { fd, .. } => {
                    write(&fd.master, buf)
                }
            };

            if let Err(e) = res {
                error!("error while sending message to fd: {}", e);
            }
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
    type Item = (&'a Fd, usize);

    fn next(&mut self) -> Option<Self::Item> {
        let len = self.fds.len();
        while self.index < len {
            let index = self.index;
            self.index += 1;

            let fd = self.fds.get_fd_by_index(index).unwrap();
            let mut res = self.fds.as_pollfds();
            let res = Fds::get_pollfd_by_raw_id(&mut res, fd.as_raw_fd());

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
    type Item = &'b Fd;

    fn next(&mut self) -> Option<Self::Item> {
        if self.index < self.poller.fds.len() {
            let res = self.poller.fds.get_fd_by_index(self.index).unwrap();
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
