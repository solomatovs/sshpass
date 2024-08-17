use std::borrow::BorrowMut;
use std::boxed::Box;
use std::cell::{Ref, RefCell, RefMut};
use std::io::{Stdin, StdinLock, Write};
use std::os::fd::BorrowedFd;
use std::os::unix::io::{AsFd, AsRawFd, FromRawFd};
use std::os::unix::process::CommandExt;
use std::process::Stdio;
use std::rc::Rc;

use clap::parser::ValuesRef;
use nix::errno::Errno::{EAGAIN, ECHILD, EINTR, ESRCH};
use nix::libc;
use nix::pty::{openpty, OpenptyResult};
use nix::sys::signal::{kill, SigSet, SigmaskHow, Signal};
use nix::sys::signalfd::{siginfo, SfdFlags, SignalFd};
use nix::sys::wait::{waitpid, WaitPidFlag, WaitStatus};
use nix::unistd::{fork, ForkResult, Pid};
use nix::{
    poll::{poll, PollFd, PollFlags, PollTimeout},
    unistd::{read, write},
};

use termios::Termios;
use termios::{
    tcsetattr, BRKINT, CS8, CSIZE, ECHO, ECHONL, ICANON, ICRNL, IEXTEN, IGNBRK, IGNCR, INLCR, ISIG,
    ISTRIP, IXON, OPOST, PARENB, PARMRK, TCSANOW, VMIN, VTIME,
};

use log::{debug, error, info, trace};

#[derive(Debug)]
pub enum FdsInfo<'fd> {
    Signal {
        fd: SignalFd,
        buf: Vec<u8>,
    },
    Stdin {
        fd: StdinLock<'fd>,
        termios: Termios,
        buf: Vec<u8>,
    },
    PtyChild {
        fd: OpenptyResult,
        pid: Pid,
        buf: Vec<u8>,
    },
}

pub struct Fds<'fd> {
    inner: Vec<FdsInfo<'fd>>,
    inner_poll: Vec<libc::pollfd>,
}

pub struct FdsIterator<'a, 'b> {
    todos: &'b Fds<'a>,
    index: usize,
}

impl<'a, 'b> Iterator for FdsIterator<'a, 'b> {
    type Item = &'b FdsInfo<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.index < self.todos.inner.len() {
            let result = Some(&self.todos.inner[self.index]);
            self.index += 1;
            return result;
        }

        None
    }
}

pub struct PollReventMutIterator<'a, 'b> {
    fds: &'b mut Fds<'a>,
    index: usize,
}

impl<'a, 'b> Iterator for PollReventMutIterator<'a, 'b> {
    type Item = &'b mut FdsInfo<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        while self.index < self.fds.inner.len() {
            let res = &mut self.fds.inner_poll[self.index];
            let index = self.index;
            self.index += 1;

            if res.revents != 0 {
                res.revents = 0;
                let ptr = self.fds.inner.as_mut_ptr();
                unsafe { return Some(&mut *ptr.add(index)) }
            }
        }

        None
    }
}

impl<'fd> Fds<'fd> {
    pub fn new() -> Self {
        Self {
            inner: vec![],
            inner_poll: vec![],
        }
    }

    pub fn poll<T: Into<PollTimeout>>(&mut self, timeout: T) -> nix::Result<libc::c_int> {
        let res = unsafe {
            libc::poll(
                self.inner_poll.as_mut_ptr().cast(),
                self.inner_poll.len() as libc::nfds_t,
                i32::from(timeout.into()),
            )
        };

        nix::errno::Errno::result(res)
    }

    pub fn iter<'b>(&'b self) -> FdsIterator<'fd, 'b> {
        FdsIterator {
            todos: self,
            index: 0,
        }
    }

    pub fn iter_only_events<'b>(&'b mut self) -> PollReventMutIterator<'fd, 'b> {
        PollReventMutIterator {
            fds: self,
            index: 0,
        }
    }

    pub fn push_pty_fd(&mut self, pty_fd: OpenptyResult, child: Pid, events: PollFlags) {
        let res = libc::pollfd {
            fd: pty_fd.master.as_raw_fd(),
            events: events.bits(),
            revents: 0,
        };

        // add two array
        self.inner.push(FdsInfo::PtyChild {
            fd: pty_fd,
            pid: child,
            buf: Vec::with_capacity(1024),
        });
        self.inner_poll.push(res);
    }

    pub fn push_signal_fd(&mut self, signal_fd: SignalFd, events: PollFlags) {
        let res = libc::pollfd {
            fd: signal_fd.as_raw_fd(),
            events: events.bits(),
            revents: 0,
        };

        // add two array
        self.inner.push(FdsInfo::Signal {
            fd: signal_fd,
            buf: Vec::with_capacity(1024),
        });
        self.inner_poll.push(res);
    }

    pub fn push_stdin_lock(
        &mut self,
        stdin: StdinLock<'static>,
        termios: Termios,
        events: PollFlags,
    ) {
        let res = libc::pollfd {
            fd: stdin.as_raw_fd(),
            events: events.bits(),
            revents: 0,
        };

        // add two array
        self.inner.push(FdsInfo::Stdin {
            fd: stdin,
            // poll_fd: res,
            termios: termios,
            buf: Vec::with_capacity(1024),
        });
        self.inner_poll.push(res);
    }

    pub fn remove_signal_fd(&mut self) {
        let indexes: Vec<usize> = self
            .inner
            .iter()
            .enumerate()
            .filter(|(_, item)| match item {
                FdsInfo::Signal { fd: _, buf: _ } => false,
                _ => true,
            })
            .map(|(i, _)| i)
            .collect();

        for i in indexes {
            self.inner.remove(i);
        }
    }
}
