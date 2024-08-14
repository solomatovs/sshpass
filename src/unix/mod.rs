use std::borrow::BorrowMut;
use std::io::{Stdin, StdinLock, Write};
use std::os::fd::BorrowedFd;
use std::os::unix::io::{AsFd, AsRawFd, FromRawFd};
use std::os::unix::process::CommandExt;
use std::process::Stdio;
use std::rc::Rc;
use std::boxed::Box;
use std::cell::{Ref, RefMut, RefCell};

use clap::parser::ValuesRef;
use nix::sys::signal::{kill, SigSet, SigmaskHow, Signal};
use nix::sys::wait::{waitpid, WaitPidFlag, WaitStatus};
use nix::libc;
use nix::pty::{openpty, OpenptyResult};
use nix::unistd::{fork, ForkResult, Pid};
use nix::sys::signalfd::{SignalFd, SfdFlags};
use nix::errno::Errno::{EINTR, ECHILD, EAGAIN, ESRCH};
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
use clap::ArgMatches;

use crate::app::NativeApp;


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


#[derive(Debug)]
pub enum FdsInfo<'fd> {
    Signal {
        fd: SignalFd,
        buf: &'fd [u8]
    },
    Stdin {
        fd: StdinLock<'fd>,
        termios: Termios,
        buf: &'fd [u8]
    },
    PtyChild {
        fds: OpenptyResult,
        pid: Pid,
        buf: &'fd [u8]
    },
}

pub struct Fds<'fd> {
    fds: Vec<FdsInfo<'fd>>,
    poll_fds: RefCell<Vec<PollFd<'fd>>>,
}

impl Fds<'_> {
    pub fn new() -> Self {
        Self {
            fds: vec![],
            poll_fds: RefCell::new(vec![]),
        }
    }

    pub fn get_mut_poll_fd(&mut self) -> &mut [PollFd] {
        todo!();
        // self.poll_fds.as_mut()
    }

    pub fn push_pty_fd(&mut self, pty_fd: OpenptyResult, child: Pid, events: PollFlags) {
        let res = PollFd::new(
            unsafe { BorrowedFd::borrow_raw(pty_fd.master.as_raw_fd()) },
            events,
        );

        // add two array
        self.fds.push(FdsInfo::PtyChild {
            fds: pty_fd,
            pid: child,
            buf: &[0],
        });
        self.poll_fds.borrow_mut().push(res);
    }

    pub fn push_signal_fd(&mut self, signal_fd: SignalFd, events: PollFlags) {
        let res = PollFd::new(
            unsafe { BorrowedFd::borrow_raw(signal_fd.as_raw_fd()) },
            events,
        );

        // add two array
        self.fds.push(FdsInfo::Signal {
            fd: signal_fd,
            buf: &[0],
        });
        self.poll_fds.borrow_mut().push(res);
    }

    pub fn push_stdin_lock(&mut self, stdin: StdinLock<'static>, termios: Termios, events: PollFlags) {
        let res = PollFd::new(
            unsafe { BorrowedFd::borrow_raw(stdin.as_raw_fd()) },
            events,
        );

        // add two array
        self.fds.push(FdsInfo::Stdin{
            fd: stdin,
            termios: termios,
            buf: &[0],
        });
        self.poll_fds.borrow_mut().push(res);
    }

    pub fn remove_signal_fd(&mut self) {
        let indexes: Vec<usize> = self.fds
            .iter()
            .enumerate()
            .filter(|(_, item)| {
                match item {
                    FdsInfo::Signal { fd: _, buf: _ } => false,
                    _ => true,
                }
            })
            .map(|(i, _)| i)
            .collect()
        ;

        for i in indexes {
            self.fds.remove(i);
            self.poll_fds.borrow_mut().remove(i);
        }
    }
}

pub struct UnixApp<'app> {
    fds: Fds<'app>,
    // buf: RefCell<[u8; 1024]>,
}

impl UnixApp<'_> {

    pub fn reg_pty_child(fds: &mut Fds, program: &String, args: Option<ValuesRef<String>>) -> Result<(), UnixError> {
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
                
                // lambda функция для перенаправления stdio
                let new_follower_stdio = || unsafe { Stdio::from_raw_fd(pty.slave.as_raw_fd()) };

                // ДАЛЬНЕЙШИЙ ЗАПУСК БЕЗ FORK ПРОЦЕССА
                // это означает что дочерний процесс не будет еще раз разделятся
                // Command будет выполняться под pid этого дочернего процесса и буквально станет им
                // осуществляется всё это с помощью exec()
                let mut cmd = std::process::Command::new(program);
                if args.is_some() {
                    cmd.args(args.unwrap());
                }

                let e = cmd
                    .stdin(new_follower_stdio())
                    .stdout(new_follower_stdio())
                    .stderr(new_follower_stdio())
                    .exec();

                error!("child error: {e}");

                Err(e.into())
            }
            Ok(ForkResult::Parent { child }) => {
                // эта исполняется только в родительском процессе
                // возвращаю pty дескриптор для отслеживания событий через poll
                fds.push_pty_fd(pty, child, PollFlags::POLLIN);

                Ok(())
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

    pub fn reg_non_canonical_stdin(fds: &mut Fds) -> Result<(), UnixError> {
        // stdin дескриптор блокирую полностью
        // перевожу его в режим non canonical для побайтовой обработки вводимых данных
        // добавляю в контейнер fds для дальнейшего отслеживания событий через poll
        let stdin = std::io::stdin().lock();
        let termios = get_termios(stdin.as_raw_fd())?;
        let mut termios_modify = get_termios(stdin.as_raw_fd())?;
        set_keypress_mode(&mut termios_modify);
        set_termios(stdin.as_raw_fd(), &termios_modify)?;
        fds.push_stdin_lock(stdin, termios, PollFlags::POLLIN);

        Ok(())
    }

    pub fn reg_signals(fds: &mut Fds, mask: SigSet) -> Result<(), UnixError> {
        let mut new_mask = SigSet::thread_get_mask()?;
        for s in mask.into_iter() {
            new_mask.add(s);
        }

        fds.remove_signal_fd();
        
        let signal_fd = SignalFd::with_flags(&new_mask, SfdFlags::SFD_NONBLOCK | SfdFlags::SFD_CLOEXEC)?;
        fds.push_signal_fd(signal_fd, PollFlags::POLLIN);

        Ok(())
    }

    pub fn new(args: ArgMatches) -> Result<Self, UnixError> {
        // Создаем контейнер для дескрипторов, которые будут опрашиваться через poll
        let mut fds = Fds::new();

        let mut mask = SigSet::empty();
        mask.add(nix::sys::signal::SIGINT);
        mask.add(nix::sys::signal::SIGTERM);
        mask.add(nix::sys::signal::SIGCHLD);
        mask.add(nix::sys::signal::SIGSTOP);
        mask.add(nix::sys::signal::SIGWINCH);
        Self::reg_signals(&mut fds, mask);

        let program = args.get_one::<String>("program").unwrap();
        let program_args = args.get_many::<String>("program_args");
        Self::reg_pty_child(&mut fds, program, program_args);

        Self::reg_non_canonical_stdin(&mut fds);

        Ok(Self {
            fds,
        })
    }

    fn deinit(&mut self) -> Result<(), UnixError> {
        // Восстанавливаем исходные атрибуты терминала
        // termios.c_lflag = self.termios.c_lflag;
        // termios.c_iflag = self.termios.c_iflag;
        // termios.c_oflag = self.termios.c_oflag;
        // termios.c_cflag = self.termios.c_cflag;
        // termios.c_cc = self.termios.c_cc;

        // trace!("termios resore: {:#?}", self.termios);
        
        // set_termios(self.stdin.as_raw_fd(), &self.termios)?;

        // drop(self.pty.master.as_fd());
        // drop(self.pty.slave.as_fd());
        // drop(self.signal_fd.as_fd());
        // drop(self.stdin.as_fd());
        // drop(self.child.as_raw());

        Ok(())
    }

    fn waitpid(child: &Pid) -> Option<UnixEvent> {
        trace!("check child process {} is running...", child);
        match waitpid(*child, Some(WaitPidFlag::WNOHANG | WaitPidFlag::WSTOPPED | WaitPidFlag::WCONTINUED | WaitPidFlag::WUNTRACED)) {
            Err(e) => {
                error!("waitpid error: {}", e);
                return Some(e.into())
            }
            Ok(WaitStatus::Exited(pid, status)) => {
                info!("WaitStatus::Exited(pid: {:?}, status: {:?}", pid, status);
                return Some(UnixEvent::ChildExited(pid, status));
            }
            Ok(WaitStatus::Signaled(pid, sig, _dumped)) => {
                info!(
                    "WaitStatus::Signaled(pid: {:?}, sig: {:?}, dumped: {:?})",
                    pid, sig, _dumped
                );

                return Some(UnixEvent::ChildSignaled(pid, sig, _dumped));
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

        None
    }

    // fn signal_event(&self) -> Option<UnixEvent> {
    //     match self.signal_fd.read_signal() {
    //         Ok(Some(sig)) => {
    //             trace!("Some(res) = read_signal()");

    //             match Signal::try_from(sig.ssi_signo as i32) {
    //                 Ok(Signal::SIGINT) => {
    //                     info!("recv SIGINT");
    //                     trace!("kill({}, SIGINT", self.child);
    //                     if let Err(ESRCH) = kill(self.child, Signal::SIGINT)
    //                     {
    //                         error!("pid {} doesnt exists or zombie", self.child);
    //                     }
    //                 }
    //                 Ok(Signal::SIGTERM) => {
    //                     info!("recv SIGTERM");
    //                     trace!("kill({}, SIGTERM", self.child);
    //                     if let Err(ESRCH) = kill(self.child, Signal::SIGTERM)
    //                     {
    //                         error!("pid {} doesnt exists or zombie", self.child);
    //                     }
    //                 }
    //                 Ok(Signal::SIGCHLD) => {
    //                     info!("recv SIGCHLD");
    //                     return self.waitpid();
    //                 }
    //                 Ok(Signal::SIGWINCH) => {
    //                     info!("recv SIGWINCH");
    //                     if let Ok(size) = get_termsize(self.stdin.as_raw_fd()) {
    //                         trace!("set termsize: {:#?}", size);
    //                         let res = set_termsize(self.pty.slave.as_raw_fd(), size);
    //                         trace!("set_termsize: {:#?}", res);
    //                     }
    //                 }
    //                 Ok(Signal::SIGSTOP) => {
    //                     info!("recv SIGSTOP");
    //                 }
    //                 Ok(sig) => {
    //                     info!("recv signal {}", sig);
    //                 }
    //                 Err(e) => {
    //                     error!("recv unknown signal");
    //                     error!("{e}");
    //                 }
    //             }
    //         }
    //         Err(EAGAIN) => {
    //             trace!("Err(nix::errno::Errno::EAGAIN) = read_signal(), SFD_NONBLOCK flag is set");
    //         }
    //         Ok(None) => {
    //             trace!(
    //                 "Ok(None) = read_signal(), SFD_NONBLOCK flag is set possible"
    //             );
    //         }
    //         Err(e) => {
    //             trace!("Err(e) = read_signal()");
    //             error!("{}", e);
    //         }
    //     }

    //     None
    // }

    // fn pty_event(&self) -> Option<UnixEvent> {
    //     trace!("read(pty.master.as_raw_fd(), &mut buf[..])");

    //     let res = {
    //         read(self.pty.master.as_raw_fd(), self.buf.borrow_mut().as_mut())
    //     };
    //     match res {
    //         Err(EAGAIN) => {
    //             // SFD_NONBLOCK mode is set
    //             trace!("Err(nix::errno::Errno::EAGAIN) = read(pty.master), SFD_NONBLOCK flag is set");
    //         }
    //         Err(e) => {
    //             // error
    //             trace!("Err(e) = read(pty.master)");
    //             error!("pty.master read error");
    //             error!("{}", e);
    //         }
    //         Ok(0) => {
    //             // EOF
    //             trace!("Ok(0) = read(pty.master)");
    //         }
    //         Ok(n) => {
    //             // read n bytes
    //             trace!("Ok({n}) = read(pty.master)");
    //             trace!("utf8: {}", String::from_utf8_lossy(&self.buf.borrow()[..n]));
    //             return Some(UnixEvent::Ptyin(self.buf.borrow(), n));
    //         }
    //     }

    //     None
    // }

    // fn stdin_event(&self) -> Option<UnixEvent> {
    //     trace!("read(stdin)");
    //     let res = {
    //         read(self.stdin.as_raw_fd(), self.buf.borrow_mut().as_mut())
    //     };
    //     match res {
    //         Err(nix::errno::Errno::EAGAIN) => {
    //             // SFD_NONBLOCK mode is set
    //             trace!("Err(nix::errno::Errno::EAGAIN) = read(stdin), SFD_NONBLOCK flag is set");
    //         }
    //         Err(e) => {
    //             // error
    //             trace!("Err(e) = read(stdin)");
    //             error!("stdin read error");
    //             error!("{}", e);
    //         }
    //         Ok(0) => {
    //             // EOF
    //             trace!("Ok(0) = read(stdin)");
    //         }
    //         Ok(n) => {
    //             trace!("Ok({n}) = read(stdin)");
    //             trace!("utf8: {}", String::from_utf8_lossy(&self.buf.borrow()[..n]));

    //             return Some(UnixEvent::Stdin(self.buf.borrow(), n));
    //             // self.write_pty(&self.buf[..n]);
    //             // self.handler.handle_stdin(&self.buf[..n]);
    //         }
    //     }

    //     None
    // }
}

impl Drop for UnixApp<'_> {
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


impl<'a, 'b> NativeApp<UnixEvent<'a>> for UnixApp<'b> {
    fn poll(&self, timeout: i32) -> UnixEvent<'a> {
        
        let mut is_timeout = false;
        let timeout = match timeout {
            -1 => PollTimeout::NONE,
            0 => PollTimeout::ZERO,
            i32::MAX => PollTimeout::MAX,
            n => PollTimeout::try_from(n).unwrap(),
        };
        trace!("poll(&mut fds, {:?})", timeout);

        // набор файловых указателей, которые будут обработаны poll
        // let mut fds = [
        //     // PollFd::new(self.signal_fd.as_fd(), PollFlags::POLLIN),
        //     // PollFd::new(self.pty.master.as_fd(), PollFlags::POLLIN),
        //     // PollFd::new(self.stdin.as_fd(), PollFlags::POLLIN),
        // ];

        let mut fds = self.fds.poll_fds.borrow_mut();
        let fds = fds.as_mut_slice();
        match poll(fds, timeout) {
            Err(e) => {
                error!("poll calling error: {}", e);
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

        // trace!("fds: {:#?}", fds);

        if is_timeout {
            return UnixEvent::Timeout;
        }

        // trace!("check OS signal event...");
        // let os_event = if let Some(PollFlags::POLLIN) = fds[0].revents() {
        //     trace!("match OS signal");
        //     self.signal_event()
        // } else {
        //     None
        // };

        // trace!("check pty events...");
        // let pty_event = if let Some(PollFlags::POLLIN) = fds[1].revents() {
        //     trace!("match pty event");
        //     self.pty_event()
        // } else {
        //     None
        // };

        // trace!("check stdin events...");
        // let stdin_event = if let Some(PollFlags::POLLIN) = fds[2].revents() {
        //     trace!("match stdin event");
        //     self.stdin_event()
        // } else {
        //     None
        // };

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
        // if let Err(e) = write(self.pty.master.as_fd(), &buf) {
        //     trace!("write(pty.master.as_fd()");
        //     error!("error writing to pty");
        //     error!("{e}");
        // }
    }
}
