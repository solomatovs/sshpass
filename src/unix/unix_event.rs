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

use clap::ArgMatches;
use log::{debug, error, info, trace};

use crate::app::NativeApp;

#[derive(Debug)]
pub enum UnixEvent<'a> {
    // Ptyin(Ref<'a, [u8]>, usize),
    // Stdin(Ref<'a, [u8]>, usize),
    Ptyin(&'a [u8]),
    Stdin(&'a [u8]),
    SignalToShutdown(siginfo),
    SignalToResize(siginfo),
    SignalChildStatus(siginfo),
    SignalStop(siginfo),
    Timeout,
    ChildExited(Pid, i32),
    ChildSignaled(Pid, Signal, bool),
    StdIoError(std::io::Error),
    NixErrorno(nix::errno::Errno),
    EventNotCapture,
}

impl std::fmt::Display for UnixEvent<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "UnixEvent")
    }
}

impl From<std::io::Error> for UnixEvent<'_> {
    fn from(e: std::io::Error) -> Self {
        UnixEvent::StdIoError(e)
    }
}

impl From<nix::errno::Errno> for UnixEvent<'_> {
    fn from(e: nix::errno::Errno) -> Self {
        UnixEvent::NixErrorno(e)
    }
}

// impl<'a> From<WaitStatus> for UnixEvent<'a> {
//     fn from(e: WaitStatus) -> Self {
//         UnixEvent::WaitStatus(e)
//     }
// }
