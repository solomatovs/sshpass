use std::io::Write;
use std::os::unix::io::{AsFd, AsRawFd, FromRawFd};
use std::os::unix::process::CommandExt;
use std::process::Stdio;
use std::cell::{Ref, RefCell};

use nix::sys::signal::{kill, SigSet, Signal};
use nix::sys::wait::{waitpid, WaitPidFlag, WaitStatus};

use log::{debug, error, info, trace};
use nix::pty::{openpty, OpenptyResult};
use nix::unistd::{fork, ForkResult, Pid};
use nix::sys::signalfd::{SignalFd, SfdFlags};
use nix::errno::Errno::{EINTR, ECHILD};
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

use crate::app::SpecifiedApp;


// Флаг          Значение
// ISIG          Разрешить посылку сигналов
// ICANON        Канонический ввод (обработка забоя и стирания строки)
// XCASE         Каноническое представление верхнего/нижнего регистров
// ECHO          Разрешить эхо
// ECHOE         Эхо на символ забоя - BS-SP-BS
// ECHOK         Выдавать NL после символа стирания строки
// ECHONL        Выдавать эхо на NL
// NOFLSH        Запретить сброс буферов после сигналов прерывания и
//               завершения
// TOSTOP        Посылать SIGTTOU фоновым процессам, которые пытаются
//               выводить на терминал
// ECHOCTL       Выдавать эхо на CTRL-символы как .r, ASCII DEL как
//               ?
// ECHOPRT       Эхо на символ забоя как стертый символ
// ECHOKE        При стирании строки, очищать ранее введенную строку
//               символами BS-SP-BS
// FLUSHO        Сбрасывание буфера вывода (состояние)
// PENDIN        Повторять несчитанный ввод при следующем чтении или
//               введенном символе
// IEXTEN        Разрешить расширенные (определенные реализацией)
//               функции
fn set_keypress_mode(termios: &mut Termios) {
    termios.c_iflag &= !(IGNBRK | BRKINT | PARMRK | ISTRIP | INLCR | IGNCR | ICRNL | IXON);
    termios.c_oflag &= !OPOST;
    termios.c_lflag &= !(ECHO | ECHONL | ICANON | ISIG | IEXTEN);
    termios.c_cflag &= !(CSIZE | PARENB);
    termios.c_cflag |= CS8;
    termios.c_cc[VMIN] = 0;
    termios.c_cc[VTIME] = 0;
}

fn set_termios(stdin_fild: i32, termios: &Termios) -> std::io::Result<()> {
    Ok(tcsetattr(stdin_fild, TCSANOW, &termios)?)
}

fn get_termios(stdin_fild: i32) -> std::io::Result<Termios> {
    Termios::from_fd(stdin_fild)
}

fn get_termsize(stdin_fild: i32) -> std::io::Result<Box<nix::libc::winsize>> {
    let mut size = Box::new(nix::libc::winsize {
        ws_row: 25,
        ws_col: 80,
        ws_xpixel: 0,
        ws_ypixel: 0,
    });
    let ret = unsafe { nix::libc::ioctl(stdin_fild, nix::libc::TIOCGWINSZ, &mut *size) };

    match ret {
        0 => Ok(size),
        _ => Err(std::io::Error::last_os_error()),
    }
}

pub fn set_termsize(fd: i32, mut size: Box<nix::libc::winsize>) -> std::io::Result<()> {
    let ret = unsafe { nix::libc::ioctl(fd, nix::libc::TIOCSWINSZ, &mut *size) };

    match ret {
        0 => Ok(()),
        _ => Err(std::io::Error::last_os_error()),
    }
}

pub struct UnixApp{
    pty: OpenptyResult,
    termios: Termios,
    signal_fd: SignalFd,
    stdin: std::io::StdinLock<'static>,
    child: Pid,
    buf: RefCell<[u8; 1024]>,
}

impl UnixApp {
    pub fn new(args: ArgMatches) -> Result<Self, UnixError> {
        // Создаем псевдотерминал (PTY)
        let pty = openpty(None, None).expect("Failed to open PTY");

        // fork() - создает дочерний процесс из текущего
        // parent блок это продолжение текущего запущенного процесса
        // child блок это то, что выполняется в дочернем процессе
        // все окружение дочернего процесса наследуется из родительского
        let status = match unsafe { fork() } {
            Ok(ForkResult::Child) => {
                let master = pty.master.try_clone().expect("try_clone pty.master");
                unsafe { nix::libc::ioctl(master.as_raw_fd(), nix::libc::TIOCNOTTY) };
                unsafe { nix::libc::setsid() };
                unsafe { nix::libc::ioctl(pty.slave.as_raw_fd(), nix::libc::TIOCSCTTY) };
                // эта программа исполняется только в дочернем процессе
                // родительский процесс в это же время выполняется и что то делает

                let program = args.get_one::<String>("program").unwrap();
                let args = args.get_many::<String>("program_args").unwrap();

                // lambda функция для перенаправления stdio
                let new_follower_stdio =
                    || unsafe { Stdio::from_raw_fd(pty.slave.as_raw_fd()) };

                // ДАЛЬНЕЙШИЙ ЗАПУСК БЕЗ FORK ПРОЦЕССА
                // это означает что дочерний процесс не будет еще раз разделятся
                // Command будет выполняться под pid этого дочернего процесса и буквально станет им
                // осуществляется всё это с помощью exec()
                let e = std::process::Command::new(program)
                    .args(args)
                    .stdin(new_follower_stdio())
                    .stdout(new_follower_stdio())
                    .stderr(new_follower_stdio())
                    .exec();

                error!("child error: {e}");

                Err(e.into())
            }
            Ok(ForkResult::Parent { child }) => {
                // // эта програма исполняется только в родительском процессе
                let stdin = std::io::stdin().lock();
                let termios = get_termios(stdin.as_raw_fd())?;
                let mut termios_modify = get_termios(stdin.as_raw_fd())?;
                set_keypress_mode(&mut termios_modify);
                set_termios(stdin.as_raw_fd(), &termios_modify)?;

                // регистрирую сигналы ОС для обработки в приложении
                let mut mask = SigSet::empty();
                mask.add(nix::sys::signal::SIGINT);
                mask.add(nix::sys::signal::SIGTERM);
                mask.add(nix::sys::signal::SIGCHLD);
                mask.add(nix::sys::signal::SIGSTOP);
                mask.add(nix::sys::signal::SIGWINCH);

                trace!("mask.thread_block()");
                mask.thread_block().expect("pthread_sigmask(SigmaskHow::SIG_BLOCK, Some(self), None) error");

                trace!("nix::sys::signalfd::SignalFd::with_flags(&mask, nix::sys::signalfd::SfdFlags::SFD_NONBLOCK | nix::sys::signalfd::SfdFlags::SFD_CLOEXEC);");
                let signal_fd = SignalFd::with_flags(
                    &mask,
                    SfdFlags::SFD_NONBLOCK | SfdFlags::SFD_CLOEXEC,
                )
                .expect("SignalFd error");

                let res = Self {
                    pty,
                    termios,
                    buf: RefCell::new([0; 1024]),
                    stdin,
                    signal_fd,
                    child,
                };

                Ok(res)
            }
            Err(e) => {
                error!(
                    "{:?}: {:?}: Fork failed: {}",
                    std::thread::current().id(),
                    std::time::SystemTime::now(),
                    e
                );
                Err(UnixError::NixErrorno())
            }
        };

        status
    }

    fn deinit(&mut self) -> Result<(), UnixError> {
        // Восстанавливаем исходные атрибуты терминала
        // termios.c_lflag = self.termios.c_lflag;
        // termios.c_iflag = self.termios.c_iflag;
        // termios.c_oflag = self.termios.c_oflag;
        // termios.c_cflag = self.termios.c_cflag;
        // termios.c_cc = self.termios.c_cc;

        trace!("termios resore: {:#?}", self.termios);
        
        set_termios(self.stdin.as_raw_fd(), &self.termios)?;

        // drop(self.pty.master.as_fd());
        // drop(self.pty.slave.as_fd());
        // drop(self.signal_fd.as_fd());
        // drop(self.stdin.as_fd());
        // drop(self.child.as_raw());

        Ok(())
    }
}

impl Drop for UnixApp {
    fn drop(&mut self) {
        if let Err(e) = self.deinit() {
            error!("deinit error: {:#?}", e);
        }
    }
}

#[derive(Debug)]
pub enum UnixEvent<'a> {
    Ptyin(Ref<'a, [u8]>, usize),
    Stdin(Ref<'a, [u8]>, usize),
    Timeout,
    ChildExited(Pid, i32),
    ChildSignaled(Pid, Signal, bool),
    StdIoError(std::io::Error),
    NixErrorno(nix::errno::Errno),
    EventNotCapture,
}

impl<'a> std::fmt::Display for UnixEvent<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "UnixEvent")
    }
}

impl<'a> From<std::io::Error> for UnixEvent<'a> {
    fn from(e: std::io::Error) -> Self {
        UnixEvent::StdIoError(e)
    }
}

impl<'a> From<nix::errno::Errno> for UnixEvent<'a> {
    fn from(e: nix::errno::Errno) -> Self {
        UnixEvent::NixErrorno(e)
    }
}

// impl<'a> From<WaitStatus> for UnixEvent<'a> {
//     fn from(e: WaitStatus) -> Self {
//         UnixEvent::WaitStatus(e)
//     }
// }



#[derive(Debug)]
pub enum UnixError {
    StdIoError(std::io::Error),

    NixErrorno(),

    // ArgumentError(String),
    ExitCodeError(i32),
    // Ok,

    // ShutdownSendError,

    // ChildTerminatedBySignal,
}

impl std::fmt::Display for UnixError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "NixError")
    }
}

impl std::error::Error for UnixError {

}

impl From<std::io::Error> for UnixError {
    fn from(error: std::io::Error) -> Self {
        UnixError::StdIoError(error)
    }
}

impl From<nix::errno::Errno> for UnixError {
    fn from(_: nix::errno::Errno) -> Self {
        UnixError::NixErrorno()
    }
}

impl From<i32> for UnixError {
    fn from(error: i32) -> Self {
        UnixError::ExitCodeError(error)
    }
}

impl<'a> SpecifiedApp<'a, UnixEvent<'a>> for UnixApp {
    fn poll(&'a self, timeout: i32) -> UnixEvent<'a> {
        let mut is_timeout = false;
        let timeout = match timeout {
            -1 => PollTimeout::NONE,
            0 => PollTimeout::ZERO,
            i32::MAX => PollTimeout::MAX,
            n => PollTimeout::try_from(n).unwrap(),
        };
        trace!("poll(&mut fds, {:?})", timeout);

        // набор файловых указателей, которые будут обработаны poll
        let mut fds = [
            PollFd::new(self.signal_fd.as_fd(), PollFlags::POLLIN),
            PollFd::new(self.pty.master.as_fd(), PollFlags::POLLIN),
            PollFd::new(self.stdin.as_fd(), PollFlags::POLLIN),
        ];

        match poll(&mut fds, timeout) {
            Err(e) => {
                // error poll calling
                error!("poll calling error: {}", e);
                // break Err(e.into());
                return e.into()
            }
            Ok(0) => {
                // timeout
                trace!("poll timeout: Ok(0)");
                is_timeout = true;
            }
            Ok(n) => {
                // match n events
                trace!("poll match {} events", n);
            }
        };

        // trace!("fds: {:#?}", self.fds);
        trace!("check child process {} is running...", self.child);
        match waitpid(self.child, Some(WaitPidFlag::WNOHANG)) {
            Err(ECHILD) => {
                error!(
                    "the process {} is not a child of the process: {:?}",
                    self.child,
                    std::thread::current().id()
                );
                return UnixEvent::NixErrorno(ECHILD);
            }
            Err(EINTR) => {
                error!("waitpid error: {}", EINTR);
                return UnixEvent::NixErrorno(EINTR);
            }
            Err(e) => {
                error!("waitpid error: {}", e);
                return e.into()
            }
            Ok(WaitStatus::Exited(pid, status)) => {
                info!("WaitStatus::Exited(pid: {:?}, status: {:?}", pid, status);
                return UnixEvent::ChildExited(pid, status);
            }
            Ok(WaitStatus::Signaled(pid, sig, _dumped)) => {
                info!(
                    "WaitStatus::Signaled(pid: {:?}, sig: {:?}, dumped: {:?})",
                    pid, sig, _dumped
                );

                return UnixEvent::ChildSignaled(pid, sig, _dumped);
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

        if is_timeout {
            return UnixEvent::Timeout;
        }

        trace!("check OS signal event...");
        if let Some(PollFlags::POLLIN) = fds[0].revents() {
            trace!("match OS signal: {:#?}", fds[0].revents());
            match self.signal_fd.read_signal() {
                Ok(Some(sig)) => {
                    trace!("Some(res) = read_signal()");

                    match Signal::try_from(sig.ssi_signo as i32) {
                        Ok(Signal::SIGINT) => {
                            info!("recv SIGINT");
                            trace!("kill({}, SIGINT", self.child);
                            if let Err(nix::errno::Errno::ESRCH) =
                                kill(self.child, Signal::SIGINT)
                            {
                                error!("pid {} doesnt exists or zombie", self.child);
                            }
                        }
                        Ok(Signal::SIGTERM) => {
                            info!("recv SIGTERM");
                            trace!("kill({}, SIGTERM", self.child);
                            if let Err(nix::errno::Errno::ESRCH) =
                                kill(self.child, Signal::SIGTERM)
                            {
                                error!("pid {} doesnt exists or zombie", self.child);
                            }
                        }
                        Ok(Signal::SIGCHLD) => {
                            info!("recv SIGCHLD");
                        }
                        Ok(Signal::SIGWINCH) => {
                            info!("recv SIGWINCH");
                            if let Ok(size) = get_termsize(self.stdin.as_raw_fd()) {
                                trace!("set termsize: {:#?}", size);
                                let res = set_termsize(self.pty.slave.as_raw_fd(), size);
                                trace!("set_termsize: {:#?}", res);
                            }
                        }
                        Ok(Signal::SIGSTOP) => {
                            info!("recv SIGSTOP");
                        }
                        Ok(sig) => {
                            info!("recv signal {}", sig);
                        }
                        Err(e) => {
                            error!("recv unknown signal");
                            error!("{e}");
                        }
                    }
                }
                Err(nix::errno::Errno::EAGAIN) => {
                    trace!("Err(nix::errno::Errno::EAGAIN) = read_signal(), SFD_NONBLOCK flag is set");
                }
                Ok(None) => {
                    trace!(
                        "Ok(None) = read_signal(), SFD_NONBLOCK flag is set possible"
                    );
                }
                Err(e) => {
                    trace!("Err(e) = read_signal()");
                    error!("{}", e);
                }
            }
            trace!("read OS signal after");
        }

        trace!("check pty events...");
        if let Some(PollFlags::POLLIN) = fds[1].revents() {
            trace!("match pty event");
            trace!("read(pty.master.as_raw_fd(), &mut buf[..])");

            let res = {
                read(self.pty.master.as_raw_fd(), self.buf.borrow_mut().as_mut())
            };
            match res {
                Err(nix::errno::Errno::EAGAIN) => {
                    // SFD_NONBLOCK mode is set
                    trace!("Err(nix::errno::Errno::EAGAIN) = read(pty.master), SFD_NONBLOCK flag is set");
                }
                Err(e) => {
                    // error
                    trace!("Err(e) = read(pty.master)");
                    error!("pty.master read error");
                    error!("{}", e);
                }
                Ok(0) => {
                    // EOF
                    trace!("Ok(0) = read(pty.master)");
                }
                Ok(n) => {
                    // read n bytes
                    trace!("Ok({n}) = read(pty.master)");
                    trace!("utf8: {}", String::from_utf8_lossy(&self.buf.borrow()[..n]));
                    // self.handler.handle_ptyin(&self.buf[..n]);
                    // let sdf = self.buf.as_ptr();
                    return UnixEvent::Ptyin(self.buf.borrow(), n);
                    // self.write_stdout(&self.buf[..n]);
                }
            }
        }

        trace!("check stdin events...");
        if let Some(PollFlags::POLLIN) = fds[2].revents() {
            trace!("read(stdin)");
            let res = {
                read(self.stdin.as_raw_fd(), self.buf.borrow_mut().as_mut())
            };
            match res {
                Err(nix::errno::Errno::EAGAIN) => {
                    // SFD_NONBLOCK mode is set
                    trace!("Err(nix::errno::Errno::EAGAIN) = read(stdin), SFD_NONBLOCK flag is set");
                }
                Err(e) => {
                    // error
                    trace!("Err(e) = read(stdin)");
                    error!("stdin read error");
                    error!("{}", e);
                }
                Ok(0) => {
                    // EOF
                    trace!("Ok(0) = read(stdin)");
                }
                Ok(n) => {
                    trace!("Ok({n}) = read(stdin)");
                    trace!("utf8: {}", String::from_utf8_lossy(&self.buf.borrow()[..n]));

                    return UnixEvent::Stdin(self.buf.borrow(), n);
                    // self.write_pty(&self.buf[..n]);
                    // self.handler.handle_stdin(&self.buf[..n]);
                }
            }
        }

        UnixEvent::EventNotCapture
    }


    fn write_stdout(&self, buf: &[u8]) {
        let mut stdout = std::io::stdout().lock();

        if let Err(e) = stdout.write_all(&buf) {
            trace!("Err(e) = stdout.write_all(&buf[..n])");
            error!("stdout write error");
            error!("{e}");
        }
        if let Err(e) = stdout.flush() {
            trace!("Err(e) = stdout.write_all(&buf[..n])");
            error!("stdout write error");
            error!("{e}");
        }
    }

    fn write_pty(&self, buf: &[u8]) {
        if let Err(e) = write(self.pty.master.as_fd(), &buf) {
            trace!("write(pty.master.as_fd()");
            error!("error writing to pty");
            error!("{e}");
        }
    }
}

