use std::borrow::BorrowMut;
use std::boxed::Box;
use std::cell::{Ref, RefCell};
use std::io::Stdin;
use std::os::fd::RawFd;
use std::os::unix::io::{AsRawFd, FromRawFd};
use std::os::unix::process::CommandExt;
use std::process::Stdio;

use nix::errno::Errno::EAGAIN;
use nix::pty::{openpty, OpenptyResult};
use nix::sys::signal::{SigSet, Signal};
use nix::sys::signalfd::{siginfo, SfdFlags, SignalFd};
// use nix::sys::wait::{waitpid, WaitPidFlag, WaitStatus};
use nix::unistd::{fork, ForkResult};
use nix::{
    poll::{PollFlags, PollTimeout},
    unistd::read,
};

use termios::Termios;
use termios::{
    tcsetattr, BRKINT, CS8, CSIZE, ECHO, ECHONL, ICANON, ICRNL, IEXTEN, IGNBRK, IGNCR, INLCR, ISIG,
    ISTRIP, IXON, OPOST, PARENB, PARMRK, TCSANOW, VMIN, VTIME,
};

use clap::parser::ValuesRef;
use clap::ArgMatches;
use log::{error, trace};

use crate::unix::fds::{Fds, FdsInfo};
use crate::unix::unix_error::UnixError;
use crate::unix::unix_event::UnixEvent;

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
    tcsetattr(stdin_fild, TCSANOW, termios)?;
    Ok(())
}

fn get_termios(stdin_fild: i32) -> std::io::Result<Termios> {
    Termios::from_fd(stdin_fild)
}

fn _get_termsize(stdin_fild: i32) -> std::io::Result<Box<nix::libc::winsize>> {
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

// pub fn _set_termsize(fd: i32, mut size: Box<nix::libc::winsize>) -> std::io::Result<()> {
//     let ret = unsafe { nix::libc::ioctl(fd, nix::libc::TIOCSWINSZ, &mut *size) };

//     match ret {
//         0 => Ok(()),
//         _ => Err(std::io::Error::last_os_error()),
//     }
// }
pub fn _set_termsize(fd: i32, mut size: nix::libc::winsize) -> std::io::Result<()> {
    let ret = unsafe { nix::libc::ioctl(fd, nix::libc::TIOCSWINSZ, &mut size) };

    match ret {
        0 => Ok(()),
        _ => Err(std::io::Error::last_os_error()),
    }
}

#[derive(Debug)]
pub struct Buffer {
    buf: RefCell<Vec<u8>>,
}

impl Buffer {
    pub fn new(size: usize) -> Self {
        Self {
            buf: RefCell::new(vec![0; size]),
        }
    }

    pub fn get_slice_len(&self, len: usize) -> std::cell::Ref<[u8]> {
        std::cell::Ref::map(self.buf.borrow(), |vec| &vec[..len])
    }

    /// Получает изменяемый срез
    pub fn get_mut_slice(&self) -> std::cell::RefMut<[u8]> {
        std::cell::RefMut::map(self.buf.borrow_mut(), |vec| vec.as_mut_slice())
    }
}

#[derive(Debug)]
pub struct UnixApp {
    fds: Fds,
    poll_timeout: PollTimeout,
    // buf: UnsafeCell<[u8; 4096]>,
    buf: Buffer,
}

impl UnixApp {
    pub fn reg_pty_child(
        &mut self,
        program: &String,
        args: Option<ValuesRef<String>>,
    ) -> Result<(), UnixError> {
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
                if let Some(args) = args {
                    cmd.args(args);
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
                self.fds
                    .borrow_mut()
                    .push_pty_fd(pty, child, PollFlags::POLLIN);
                // self.buf.push(vec![0; 1024]);

                Ok(())
            }
            Err(e) => {
                error!(
                    "{:?}: {:?}: Fork failed: {}",
                    std::thread::current().id(),
                    std::time::SystemTime::now(),
                    e
                );
                Err(e.into())
            }
        };

        status
    }

    pub fn reg_non_canonical_stdin(&mut self) -> Result<(), UnixError> {
        // перевожу stdin в режим non canonical для побайтовой обработки вводимых данных
        // добавляю в контейнер fds для дальнейшего отслеживания событий через poll
        let termios = get_termios(std::io::stdin().lock().as_raw_fd())?;

        Self::set_non_canonical_stdin()?;
        self.fds
            .borrow_mut()
            .push_stdin_fd(std::io::stdin(), termios, PollFlags::POLLIN);

        Ok(())
    }

    pub fn set_non_canonical_stdin() -> Result<(), UnixError> {
        let stdin = std::io::stdin();
        let lock = stdin.lock();
        let mut termios_modify = get_termios(lock.as_raw_fd())?;
        set_keypress_mode(&mut termios_modify);
        set_termios(lock.as_raw_fd(), &termios_modify)?;

        Ok(())
    }

    // pub fn set_termios_stdin(termios: &Termios) -> Result<(), UnixError> {
    //     let stdin = std::io::stdin();
    //     let lock = stdin.lock();

    //     set_termios(lock.as_raw_fd(), &termios)?;

    //     Ok(())
    // }

    pub fn reg_signals(&mut self, mask: SigSet) -> Result<(), UnixError> {
        let mut new_mask = SigSet::thread_get_mask()?;
        for s in mask.into_iter() {
            new_mask.add(s);
        }

        self.fds.borrow_mut().remove_signal_fd();

        new_mask.thread_block()?;

        let signal_fd =
            SignalFd::with_flags(&new_mask, SfdFlags::SFD_NONBLOCK | SfdFlags::SFD_CLOEXEC)?;
        self.fds
            .borrow_mut()
            .push_signal_fd(signal_fd, PollFlags::POLLIN);

        Ok(())
    }

    pub fn new(args: ArgMatches) -> Result<Self, UnixError> {
        // Создаем контейнер для дескрипторов, которые будут опрашиваться через poll
        let mut res = Self {
            fds: Fds::new(),
            // buf: UnsafeCell::new([0; 4096]),
            buf: Buffer::new(4096),
            poll_timeout: PollTimeout::NONE,
        };

        let mut mask = SigSet::empty();
        // mask.add(nix::sys::signal::SIGINT);
        // mask.add(nix::sys::signal::SIGTERM);
        // mask.add(nix::sys::signal::SIGCHLD);
        // mask.add(nix::sys::signal::SIGSTOP);
        // mask.add(nix::sys::signal::SIGWINCH);
        // mask.add(nix::sys::signal::SIGUSR1);
        // mask.add(nix::sys::signal::SIGUSR2);
        for signal in Signal::iterator() {
            mask.add(signal);
        }
        res.reg_signals(mask)?;

        let program = args.get_one::<String>("program").unwrap();
        let program_args = args.get_many::<String>("program_args");
        res.reg_pty_child(program, program_args)?;

        res.reg_non_canonical_stdin()?;

        Ok(res)
    }

    fn deinit(&mut self) -> Result<(), UnixError> {
        trace!("deinit fds...");
        for fd in self.fds.iter() {
            match fd {
                FdsInfo::Signal { fd: _, .. } => {}
                FdsInfo::PtyMaster { fd: _, _pid: _, .. } => {}
                FdsInfo::Stdin { fd, termios, .. } => {
                    // Восстанавливаем исходные атрибуты терминала
                    trace!("termios restore: {:#?}", termios);
                    let res = set_termios(fd.as_raw_fd(), termios);
                    trace!("termios restore: {:?}", res);
                }
            }
        }
        trace!("deinit fds");

        Ok(())
    }

    // fn _waitpid(child: &Pid) -> Option<UnixEvent> {
    //     trace!("check child process {} is running...", child);
    //     match waitpid(
    //         *child,
    //         Some(
    //             WaitPidFlag::WNOHANG
    //                 | WaitPidFlag::WSTOPPED
    //                 | WaitPidFlag::WCONTINUED
    //                 | WaitPidFlag::WUNTRACED,
    //         ),
    //     ) {
    //         Err(e) => {
    //             error!("waitpid error: {}", e);
    //             return Some(e.into());
    //         }
    //         Ok(WaitStatus::Exited(pid, status)) => {
    //             info!("WaitStatus::Exited(pid: {:?}, status: {:?}", pid, status);
    //             return Some(UnixEvent::ChildExited(pid, status));
    //         }
    //         Ok(WaitStatus::Signaled(pid, sig, _dumped)) => {
    //             info!(
    //                 "WaitStatus::Signaled(pid: {:?}, sig: {:?}, dumped: {:?})",
    //                 pid, sig, _dumped
    //             );

    //             return Some(UnixEvent::ChildSignaled(pid, sig, _dumped));
    //         }
    //         Ok(WaitStatus::Stopped(pid, sig)) => {
    //             debug!("WaitStatus::Stopped(pid: {:?}, sig: {:?})", pid, sig);
    //         }
    //         Ok(WaitStatus::StillAlive) => {
    //             trace!("WaitStatus::StillAlive");
    //         }
    //         Ok(WaitStatus::Continued(pid)) => {
    //             trace!("WaitStatus::Continued(pid: {:?})", pid);
    //         }
    //         Ok(WaitStatus::PtraceEvent(pid, sig, c)) => {
    //             trace!(
    //                 "WaitStatus::PtraceEvent(pid: {:?}, sig: {:?}, c: {:?})",
    //                 pid,
    //                 sig,
    //                 c
    //             );
    //         }
    //         Ok(WaitStatus::PtraceSyscall(pid)) => {
    //             trace!("WaitStatus::PtraceSyscall(pid: {:?})", pid);
    //         }
    //     }

    //     None
    // }

    // match Signal::try_from(sig.ssi_signo as i32) {
    //     Ok(Signal::SIGINT) => {
    //         info!("recv SIGINT");
    //         return Ok(Some(UnixEvent::Signal(sig)));
    //         // trace!("kill({}, SIGINT", self.child);
    //         // if let Err(ESRCH) = kill(self.child, Signal::SIGINT)
    //         // {
    //         //     error!("pid {} doesnt exists or zombie", self.child);
    //         // }
    //     }
    //     Ok(Signal::SIGTERM) => {
    //         info!("recv SIGTERM");
    //         return Ok(Some(UnixEvent::Signal(sig)));
    //         // trace!("kill({}, SIGTERM", self.child);
    //         // if let Err(ESRCH) = kill(self.child, Signal::SIGTERM)
    //         // {
    //         //     error!("pid {} doesnt exists or zombie", self.child);
    //         // }
    //     }
    //     Ok(Signal::SIGCHLD) => {
    //         info!("recv SIGCHLD");
    //         return Ok(Some(UnixEvent::SignalChildStatus(sig)));
    //         // return self.waitpid();
    //     }
    //     Ok(Signal::SIGWINCH) => {
    //         info!("recv SIGWINCH");
    //         return Ok(Some(UnixEvent::SignalToResize(sig)));
    //         // if let Ok(size) = get_termsize(self.stdin.as_raw_fd()) {
    //         //     trace!("set termsize: {:#?}", size);
    //         //     let res = set_termsize(self.pty.slave.as_raw_fd(), size);
    //         //     trace!("set_termsize: {:#?}", res);
    //         // }
    //     }
    //     Ok(Signal::SIGTSTP) => {
    //         info!("recv SIGTSTP");
    //         return Ok(Some(UnixEvent::SignalStop(sig)));
    //     }
    //     Ok(signal) => {
    //         info!("recv signal {:#?}", signal);
    //         return Ok(Some(UnixEvent::SignalUnknown(sig)));
    //     }
    //     Err(e) => {
    //         error!("recv unknown signal");
    //         error!("{e}");
    //         return Err(e.into())
    //     }
    // }
    // unsafe fn get_mut_from_immutble<T>(reference: &T) -> &mut T {
    //     let const_ptr = reference as *const T;
    //     let mut_ptr = const_ptr as *mut T;
    //     &mut *mut_ptr
    // }

    fn read_event(fd: RawFd, buf: &mut [u8]) -> Result<usize, nix::errno::Errno> {
        trace!("try read({:?}, buf)", fd);

        let res = { read(fd, buf) };
        match res {
            Err(EAGAIN) => {
                // non block
                trace!(
                    "non-blocking reading mode is enabled (SFD_NONBLOCK). fd {} doesn't data",
                    fd
                );
                Ok(0)
            }
            Err(e) => {
                // error
                error!("read = Err({})", e);
                Err(e)
            }
            Ok(0) => {
                // EOF
                trace!("read = Ok(0) bytes (EOF)");
                Ok(0)
            }
            Ok(n) => {
                // read n bytes
                trace!("read = Ok({n}) bytes");
                Ok(n)
            }
        }
    }

    fn map_ref_to_siginfo(bytes: Ref<[u8]>) -> Ref<siginfo> {
        Ref::map(bytes, |slice| {
            // Преобразуем срез байт в ссылку на siginfo
            assert!(
                slice.len() >= std::mem::size_of::<siginfo>(),
                "Slice too small"
            );
            unsafe { &*(slice.as_ptr() as *const siginfo) }
        })
    }

    fn match_signal_event(&self, fd: &SignalFd) -> Result<UnixEvent, UnixError> {
        let res = Self::read_event(fd.as_raw_fd(), &mut self.buf.get_mut_slice());
        match res {
            Err(e) => {
                // error
                trace!("signal match Err({:?})", e);
                Err(e.into())
            }
            Ok(0) => {
                // EOF
                trace!("signal match Ok(0) bytes");
                Ok(UnixEvent::ReadZeroBytes)
            }
            Ok(n) => {
                // read n bytes
                trace!("signal match Ok({n}) bytes");
                trace!("try convert to struct siginfo");
                let buf = self.buf.get_slice_len(n);
                let res = Self::map_ref_to_siginfo(buf);

                let signal = Signal::try_from(res.ssi_signo as i32);
                if let Err(e) = signal {
                    error!("Error converting received bytes to the Signal struct: {e}");
                    return Err(e.into());
                }

                let signal = signal.unwrap();
                let res = UnixEvent::Signal(signal, res);
                Ok(res)
            }
        }
    }

    fn match_pty_event(&self, fd: &OpenptyResult) -> Result<UnixEvent, UnixError> {
        let res = Self::read_event(fd.master.as_raw_fd(), &mut self.buf.get_mut_slice());
        match res {
            Err(e) => {
                // error
                trace!("pty match Err({:?})", e);
                Err(e.into())
            }
            Ok(0) => {
                // EOF
                trace!("pty match Ok(0) bytes");
                Ok(UnixEvent::ReadZeroBytes)
            }
            Ok(n) => {
                // read n bytes
                trace!("pty match Ok({n}) bytes");
                let buf = self.buf.get_slice_len(n);
                let res = UnixEvent::PtyMaster(buf);
                Ok(res)
            }
        }
    }

    fn match_stdin_event(&self, fd: &Stdin) -> Result<UnixEvent, UnixError> {
        let res = Self::read_event(fd.as_raw_fd(), &mut self.buf.get_mut_slice());
        match res {
            Err(e) => {
                // error
                trace!("stdin match Err({:?})", e);
                Err(e.into())
            }
            Ok(0) => {
                // EOF
                trace!("stdin match Ok(0) bytes");
                Ok(UnixEvent::ReadZeroBytes)
            }
            Ok(n) => {
                // read n bytes
                trace!("stdin match Ok({n}) bytes");
                let buf = self.buf.get_slice_len(n);
                let res = UnixEvent::Stdin(buf);
                Ok(res)
            }
        }
    }

    pub fn system_event(&mut self) -> Result<UnixEvent, UnixError> {
        trace!("poll(&mut fds, {:?})", self.poll_timeout);
        match self.fds.borrow_mut().poll(self.poll_timeout) {
            Err(e) => {
                error!("poll calling error: {}", e);
                return Err(e.into());
            }
            Ok(0) => {
                // timeout
                trace!("poll timeout: Ok(0)");
                return Ok(UnixEvent::PollTimeout);
            }
            Ok(n) => {
                // match n events
                trace!("poll match {} events", n);
            }
        };

        // trace!("{:#?}", self.fds);

        // Извлекаем необходимую информацию из итератора
        if let Some(fd) = self.fds.iter_only_revent().next() {
            match fd {
                FdsInfo::Signal { fd, .. } => {
                    return self.match_signal_event(fd);
                }
                FdsInfo::PtyMaster { fd, .. } => {
                    return self.match_pty_event(fd);
                }
                FdsInfo::Stdin { fd, .. } => {
                    return self.match_stdin_event(fd);
                }
            }
        }

        Err(UnixError::PollEventNotHandle)
    }

    // pub fn write_to_stdout(&self, buf: &[u8]) {
    //     let mut stdout = std::io::stdout().lock();

    //     if let Err(e) = stdout.write_all(buf) {
    //         trace!("Err(e) = stdout.write_all(&buf[..n])");
    //         error!("stdout write error");
    //         error!("{e}");
    //     }
    //     if let Err(e) = stdout.flush() {
    //         trace!("Err(e) = stdout.write_all(&buf[..n])");
    //         error!("stdout write error");
    //         error!("{e}");
    //     }
    // }

    // pub fn send_to(&self, index: usize, buf: &[u8]) -> Result<(), UnixError> {
    //     let res = match self.fds.get_fd_by_index(index) {
    //         Some(FdsInfo::PtyMaster { fd, .. }) => {
    //             write(&fd.master, buf).map_err(|e| e.into()).map(|_| ())
    //         }
    //         Some(FdsInfo::Signal { fd, .. }) => {
    //             Err(UnixError::FdReadOnly)
    //         }
    //         Some(FdsInfo::Stdin { fd, .. }) => {
    //             let _ = fd.lock();
    //             write(fd.as_fd(), buf).map_err(|e| e.into()).map(|_| ())
    //         }
    //         None => {
    //             Err(UnixError::FdReadOnly)
    //         }
    //     };

    //     res
    // }
}

impl Drop for UnixApp {
    fn drop(&mut self) {
        if let Err(e) = self.deinit() {
            error!("deinit error: {:#?}", e);
        }
    }
}
