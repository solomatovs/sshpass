use std::io::Stdin;
use std::os::unix::io::AsRawFd;

use nix::libc;
use nix::poll::{PollFlags, PollTimeout};
use nix::pty::OpenptyResult;
use nix::sys::signalfd::SignalFd;
use nix::unistd::Pid;

use termios::Termios;

#[derive(Debug)]
pub enum FdsInfo {
    Signal { fd: SignalFd },
    Stdin { fd: Stdin, termios: Termios },
    PtyMaster { fd: OpenptyResult, _pid: Pid },
}

#[derive(Debug)]
pub struct Fds {
    inner: Vec<FdsInfo>,
    inner_poll: Vec<libc::pollfd>,
}

#[derive(Debug)]
pub struct PollReventConsumeIterator<'a> {
    fds: &'a Fds,
    index: usize,
}

impl<'a> Iterator for PollReventConsumeIterator<'a> {
    type Item = &'a FdsInfo;

    fn next(&mut self) -> Option<Self::Item> {
        while self.index < self.fds.inner.len() {
            let index = self.index;

            let res = {
                let mut res = self.fds.inner_poll[self.index];

                self.index += 1;

                if res.revents != 0 {
                    res.revents = 0;
                    true
                } else {
                    false
                }
            };

            if res {
                return Some(&self.fds.inner[index]);
            }
        }

        None
    }
}

#[derive(Debug)]
pub struct FdsIterator<'b> {
    fds: &'b Fds,
    index: usize,
}

impl<'b> Iterator for FdsIterator<'b> {
    type Item = &'b FdsInfo;

    fn next(&mut self) -> Option<Self::Item> {
        if self.index < self.fds.inner.len() {
            let res = &self.fds.inner[self.index];
            self.index += 1;
            return Some(res);
        }

        None
    }
}

impl Fds {
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

    pub fn iter_only_revent(&self) -> PollReventConsumeIterator {
        PollReventConsumeIterator {
            fds: self,
            index: 0,
        }
    }

    pub fn iter(&self) -> FdsIterator {
        FdsIterator {
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
        self.inner.push(FdsInfo::PtyMaster {
            fd: pty_fd,
            _pid: child,
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
        self.inner.push(FdsInfo::Signal { fd: signal_fd });
        self.inner_poll.push(res);
    }

    pub fn push_stdin_fd(&mut self, stdin: Stdin, termios: Termios, events: PollFlags) {
        let res = libc::pollfd {
            fd: stdin.as_raw_fd(),
            events: events.bits(),
            revents: 0,
        };

        // add two array
        self.inner.push(FdsInfo::Stdin { fd: stdin, termios });
        self.inner_poll.push(res);
    }

    pub fn remove_signal_fd(&mut self) {
        let indexes: Vec<usize> = self
            .inner
            .iter()
            .enumerate()
            .filter(|(_, item)| matches!(item, FdsInfo::Signal { fd: _ }))
            .map(|(i, _)| i)
            .collect();

        for i in indexes {
            self.inner.remove(i);
        }
    }

    // pub fn get_fd_by_index(&self, index:usize) -> Option<&FdsInfo> {
    //     self.inner.get(index)
    // }
}
