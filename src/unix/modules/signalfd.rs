use crate::common::{AppContext, Handler};
use crate::unix::{UnixEvent, UnixEventResponse};
use super::EventMiddlewareType;

use std::cell::RefCell;
use std::rc::Rc;

use nix::sys::wait::{waitpid, WaitPidFlag, WaitStatus};
use nix::unistd::Pid;
use nix::sys::signal::Signal;

use log::{trace, error, debug, info};


pub struct SignalfdMiddleware<'a> {
    next: Option<Rc<RefCell<EventMiddlewareType<'a>>>>,
}

impl <'a> SignalfdMiddleware<'a> {
    pub fn new() -> Self {
        Self {
            next: None,
        }
    }

    pub fn waitpid(&self, pid: nix::libc::pid_t) -> nix::Result<WaitStatus> {
        trace!("check child process {} is running...", pid);
        let pid = Pid::from_raw(pid);
        let options = Some(
            WaitPidFlag::WNOHANG
                | WaitPidFlag::WSTOPPED
                | WaitPidFlag::WCONTINUED
                | WaitPidFlag::WUNTRACED,
        );

        let res = waitpid(pid, options);

        match res {
            Err(e) => {
                error!("waitpid error: {}", e);
            }
            Ok(WaitStatus::Exited(pid, status)) => {
                info!("WaitStatus::Exited(pid: {:?}, status: {:?}", pid, status);
            }
            Ok(WaitStatus::Signaled(pid, sig, _dumped)) => {
                info!(
                    "WaitStatus::Signaled(pid: {:?}, sig: {:?}, dumped: {:?})",
                    pid, sig, _dumped
                );
            }
            Ok(WaitStatus::Stopped(pid, sig)) => {
                debug!("WaitStatus::Stopped(pid: {:?}, sig: {:?})", pid, sig);
            }
            Ok(WaitStatus::StillAlive) => {
                trace!("WaitStatus::StillAlive");
            }
            Ok(WaitStatus::Continued(pid)) => {
                trace!("WaitStatus::Continued(pid: {:?})", pid);
            }
            Ok(WaitStatus::PtraceEvent(pid, sig, c)) => {
                trace!(
                    "WaitStatus::PtraceEvent(pid: {:?}, sig: {:?}, c: {:?})",
                    pid,
                    sig,
                    c
                );
            }
            Ok(WaitStatus::PtraceSyscall(pid)) => {
                trace!("WaitStatus::PtraceSyscall(pid: {:?})", pid);
            }
        }

        res
    }
}

impl<'a> Handler<&'a mut AppContext, UnixEvent<'a>, UnixEventResponse<'a>> for SignalfdMiddleware<'a>  {
    fn handle(&mut self, context: &'a mut AppContext, value: UnixEvent<'a>) -> UnixEventResponse<'a> {
        trace!("signalfd middleware");

        let mut res = UnixEventResponse::Unhandled;

        if let UnixEvent::Signal(_index, sig, _sigino) = &value {
            trace!("signal {:#?}", sig);
            if matches!(sig, Signal::SIGINT | Signal::SIGTERM) {
                context.shutdown.shutdown_starting(0, None);
            }

            if matches!(sig, Signal::SIGCHLD) {
                let pid = _sigino.ssi_pid as nix::libc::pid_t;
                let res = self.waitpid(pid);
                trace!("waitpid({}) = {:#?}", pid, res);
            }
        }

        if let Some(ref next) = self.next {
            res = Rc::clone(next).borrow_mut().handle(context, value);
        }
        
        res
    }
}
