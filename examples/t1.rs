use nix::pty::openpty;
use nix::sys::signalfd;
use nix::unistd::Pid;
use tokio::sync::mpsc::error::SendError;
use std::os::unix::process::CommandExt;
use std::process::Stdio;
use std::fs::File;
use std::io::{Read, StderrLock, StdinLock, StdoutLock, Write};
use std::os::fd::{OwnedFd};
use std::env;
use nix::unistd::{fork, ForkResult, dup2};
use nix::sys::wait::{waitpid, WaitPidFlag, WaitStatus};
use nix::sys::signal::{kill, signal, sigprocmask, SigSet, SIGINT, SIGTERM};
use nix::libc::{ioctl, F_SETFL, O_NONBLOCK, signalfd, sigset_t, sigemptyset, sigaddset};
// use std::os::fd::AsRawFd;

use tokio;
use tokio::sync::mpsc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command as TokioCommand;
use tokio::process::Child;
// use tokio::signal;
use nix::sys::signalfd::SfdFlags;


use std::os::unix::io::{AsFd, AsRawFd, FromRawFd};
use nix::{poll::{PollTimeout, PollFd, PollFlags, poll},
unistd::{pipe, read}
};


use termios::Termios;
use termios::{
    tcsetattr, BRKINT, CS8, CSIZE, ECHO, ECHONL, ICANON, ICRNL, IEXTEN, IGNBRK, IGNCR, INLCR, ISIG,
    ISTRIP, IXON, OPOST, PARENB, PARMRK, TCSANOW, VMIN, VTIME,
};


fn main() {
    let mut mask = SigSet::empty();
    mask.add(nix::sys::signal::SIGINT);
    mask.add(nix::sys::signal::SIGTERM);

    let sfd = nix::sys::signalfd::SignalFd::with_flags(&mask, SfdFlags::SFD_NONBLOCK).unwrap();
    let mut fds = [
        PollFd::new(sfd.as_fd(), PollFlags::POLLIN)
    ];

    loop {
            // let res = poll(&mut fds, PollTimeout::MAX);

            // let _res = match res {
            //     Ok(-1) => {
            //         eprintln!("poll Ok(-1)");
            //         continue;
            //     }
            //     Ok(0) => {
            //         eprintln!("poll Ok(0)");
            //         continue;
            //     }
            //     Ok(n) => {
            //         eprintln!("poll Ok({})", n);
            //         n
            //     }
            //     Err(e) => {
            //         eprintln!("poll Err({:#?})", e);
            //         continue;
            //     }
            // };
            
            match sfd.read_signal() {
                // we caught a signal
                Ok(Some(sig)) => {
                    eprintln!("sfd.read_signal(Some({:#?}))", sig);
                    break;
                }
                Ok(None) => {
                    // there were no signals waiting (only happens when the SFD_NONBLOCK flag is set,
                    // otherwise the read_signal call blocks)
                    eprintln!("sfd.read_signal(Ok(None))");
                }
                Err(nix::errno::Errno::EAGAIN) => {
                    eprintln!("sfd.read_signal(Err(nix::errno::Errno::EAGAIN))");
                }
                Err(e) => {
                    eprintln!("sfd.read_signal(Err({:#?}))", e);
                    break;
                }, // some error happend
            }
    }

    eprintln!("buy");
}