// use crate::buffer::Buffer;
// use std::collections::HashMap;
// use std::os::fd::{AsFd, BorrowedFd, OwnedFd, RawFd};
// use std::os::unix::io::{AsRawFd, FromRawFd};

// use nix::pty::openpty;
// use nix::sys::eventfd::EventFd;
// use nix::unistd::Pid;
// use nix::unistd::{fork, ForkResult};
// use std::ffi::OsStr;
// use std::os::unix::process::CommandExt;
// use std::process::Stdio;

// use nix::fcntl;
// use nix::libc;
// use nix::poll::PollFlags;
// use nix::sys::signal::{SigSet, Signal};
// use nix::sys::signalfd::{siginfo, SfdFlags, SignalFd};
// use nix::sys::termios::{self, ControlFlags, InputFlags, LocalFlags, OutputFlags, SetArg, Termios};
// use nix::sys::timer::{Expiration, TimerSetTimeFlags};
// use nix::sys::timerfd::{ClockId, TimerFd, TimerFlags};

use crate::{AppShutdown, UnixPoll};

// #[derive(Debug)]
// #[repr(C)]
// pub enum FileType {
//     Stdin {
//         fd: std::io::Stdin,
//         buf: Buffer,
//         termios: Termios,
//     },
//     Stdout {
//         fd: std::io::Stdout,
//         buf: Buffer,
//     },
//     Stderr {
//         fd: std::io::Stderr,
//         buf: Buffer,
//     },
//     SignalFd {
//         fd: SignalFd,
//         buf: Buffer,
//     },
//     PtyMaster {
//         master: OwnedFd,
//         buf: Buffer,
//         slave: OwnedFd,
//         child: Pid,
//     },
//     TimerFd {
//         fd: TimerFd,
//         buf: Buffer,
//     },
//     EventFd {
//         fd: EventFd,
//         buf: Buffer,
//     },
// }

// impl std::fmt::Display for FileType {
//     fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
//         match self {
//             FileType::Stdin { fd, buf, .. } => {
//                 write!(
//                     f,
//                     "Stdin(fd: {}, buf_size: {})",
//                     fd.as_raw_fd(),
//                     buf.get_data_len()
//                 )
//             }
//             FileType::Stdout { fd, buf } => {
//                 write!(
//                     f,
//                     "Stdout(fd: {}, buf_size: {})",
//                     fd.as_raw_fd(),
//                     buf.get_data_len()
//                 )
//             }
//             FileType::Stderr { fd, buf } => {
//                 write!(
//                     f,
//                     "Stderr(fd: {}, buf_size: {})",
//                     fd.as_raw_fd(),
//                     buf.get_data_len()
//                 )
//             }
//             FileType::SignalFd { fd, buf } => {
//                 write!(
//                     f,
//                     "SignalFd(fd: {}, buf_size: {})",
//                     fd.as_raw_fd(),
//                     buf.get_data_len()
//                 )
//             }
//             FileType::TimerFd { fd, buf } => {
//                 write!(
//                     f,
//                     "TimerFd(fd: {}, buf_size: {})",
//                     fd.as_fd().as_raw_fd(),
//                     buf.get_data_len()
//                 )
//             }
//             FileType::EventFd { fd, buf } => {
//                 write!(
//                     f,
//                     "EventFd(fd: {}, buf_size: {})",
//                     fd.as_fd().as_raw_fd(),
//                     buf.get_data_len()
//                 )
//             }
//             FileType::PtyMaster {
//                 master, buf, child, ..
//             } => {
//                 write!(
//                     f,
//                     "PtyMaster(fd: {}, buf_size: {}, child_pid: {})",
//                     master.as_raw_fd(),
//                     buf.get_data_len(),
//                     child
//                 )
//             }
//         }
//     }
// }

// impl FileType {
//     pub fn as_fd(&self) -> BorrowedFd {
//         match self {
//             FileType::Stdin { fd, .. } => fd.as_fd(),
//             FileType::Stdout { fd, .. } => fd.as_fd(),
//             FileType::Stderr { fd, .. } => fd.as_fd(),
//             FileType::SignalFd { fd, .. } => fd.as_fd(),
//             FileType::PtyMaster { master, .. } => master.as_fd(),
//             FileType::TimerFd { fd, .. } => fd.as_fd(),
//             FileType::EventFd { fd, .. } => fd.as_fd(),
//         }
//     }

//     pub fn as_raw_fd(&self) -> i32 {
//         match self {
//             FileType::Stdin { fd, .. } => fd.as_raw_fd(),
//             FileType::Stdout { fd, .. } => fd.as_raw_fd(),
//             FileType::Stderr { fd, .. } => fd.as_raw_fd(),
//             FileType::SignalFd { fd, .. } => fd.as_raw_fd(),
//             FileType::PtyMaster { master, .. } => master.as_raw_fd(),
//             FileType::TimerFd { fd, .. } => fd.as_fd().as_raw_fd(),
//             FileType::EventFd { fd, .. } => fd.as_fd().as_raw_fd(),
//         }
//     }

//     pub fn make_events(&self) -> PollFlags {
//         match self {
//             FileType::Stdin { .. } => {
//                 PollFlags::POLLIN | PollFlags::POLLERR | PollFlags::POLLHUP | PollFlags::POLLNVAL
//             }
//             FileType::Stdout { .. } => {
//                 PollFlags::POLLOUT | PollFlags::POLLERR | PollFlags::POLLHUP | PollFlags::POLLNVAL
//             }
//             FileType::Stderr { .. } => {
//                 PollFlags::POLLOUT | PollFlags::POLLERR | PollFlags::POLLHUP | PollFlags::POLLNVAL
//             }
//             FileType::SignalFd { .. } => {
//                 PollFlags::POLLIN | PollFlags::POLLERR | PollFlags::POLLHUP | PollFlags::POLLNVAL
//             }
//             FileType::TimerFd { .. } => {
//                 PollFlags::POLLIN | PollFlags::POLLERR | PollFlags::POLLHUP | PollFlags::POLLNVAL
//             }
//             FileType::PtyMaster { .. } => {
//                 PollFlags::POLLIN
//                     | PollFlags::POLLOUT
//                     | PollFlags::POLLERR
//                     | PollFlags::POLLHUP
//                     | PollFlags::POLLNVAL
//             }
//             FileType::EventFd { .. } => {
//                 PollFlags::POLLIN | PollFlags::POLLERR | PollFlags::POLLHUP | PollFlags::POLLNVAL
//             }
//         }
//     }

//     pub fn get_mut_buf(&mut self) -> &mut Buffer {
//         match self {
//             FileType::Stdin { buf, .. } => buf,
//             FileType::Stdout { buf, .. } => buf,
//             FileType::Stderr { buf, .. } => buf,
//             FileType::SignalFd { buf, .. } => buf,
//             FileType::PtyMaster { buf, .. } => buf,
//             FileType::TimerFd { buf, .. } => buf,
//             FileType::EventFd { buf, .. } => buf,
//         }
//     }
// }

#[derive(Debug)]
#[repr(C)]
pub struct UnixContext {
    pub poll: UnixPoll,
    pub shutdown: AppShutdown,
}

impl UnixContext {
    pub fn new(poll_timeout: i32) -> Self {
        // Создаем контейнер для дескрипторов, который будет опрашиваться через poll
        Self {
            poll: UnixPoll::new(poll_timeout),
            shutdown: AppShutdown::default(),
        }
    }

    // pub fn event_pocess(&mut self) -> i32 {
    //     // trace!("poll(&mut fds, {:?})", poll_timeout);

    //     let poller = self.make_pollfd();
    //     let res = unsafe {
    //         libc::poll(
    //             poller.as_mut_ptr().cast(),
    //             poller.len() as libc::nfds_t,
    //             poll_timeout,
    //         )
    //     };

    //     // trace!("poll result: {:?}", res);

    //     res
    // }
    // pub fn get_signal_raw_fd(&mut self) -> Option<RawFd> {
    //     self.fds.values().find_map(|x| match x {
    //         FileType::SignalFd { fd, .. } => Some(fd.as_raw_fd()),
    //         _ => None,
    //     })
    // }

    // fn is_valid_fd(&self, fd: RawFd) -> bool {
    //     let mut res = fcntl::fcntl(fd, fcntl::F_GETFD);

    //     // запрашиваю до тех пор, пока приходит EINTR
    //     // так как это означает что вызов fcntl был прерван сигналом и надо повторить попытку
    //     while let Err(nix::Error::EINTR) = res {
    //         res = fcntl::fcntl(fd, fcntl::F_GETFD);
    //     }

    //     if res.is_ok() {
    //         return true;
    //     }

    //     false
    // }

    // pub fn add_signal_fd_if_not_exists(&mut self) -> Result<(), UnixError> {
    //     if let Some(fd) = self.get_signal_raw_fd() {
    //         if self.is_valid_fd(fd) {
    //             return Ok(());
    //         } else {
    //             self.fds.remove(&fd);
    //         }
    //     }

    //     let mut mask = SigSet::empty();

    //     // добавляю в обработчик все сигналы
    //     for signal in Signal::iterator() {
    //         if matches!(signal, Signal::SIGKILL | Signal::SIGSTOP) {
    //             continue;
    //         }

    //         mask.add(signal);
    //     }

    //     let mut new_mask = SigSet::thread_get_mask()
    //         .map_err(|e| UnixError::SignalFdError(format!("failed get thread mask: {:#?}", e)))?;
    //     for s in mask.into_iter() {
    //         new_mask.add(s);
    //     }

    //     new_mask
    //         .thread_block()
    //         .map_err(|e| UnixError::SignalFdError(format!("failed set thread mask: {:#?}", e)))?;

    //     let fd: SignalFd =
    //         SignalFd::with_flags(&new_mask, SfdFlags::SFD_NONBLOCK | SfdFlags::SFD_CLOEXEC)
    //             .map_err(|e| {
    //                 UnixError::SignalFdError(format!("signalfd create failed error: {:#?}", e))
    //             })?;

    //     let buffer_length = std::mem::size_of::<siginfo>();
    //     let buf = Buffer::try_new(buffer_length).map_err(|_e| {
    //         UnixError::AllocationError(format!(
    //             "signal fd buffer allocation error: {} bytes",
    //             buffer_length
    //         ))
    //     })?;
    //     // let buf = Buffer::new(buffer_length);

    //     self.fds
    //         .insert(fd.as_raw_fd(), FileType::SignalFd { fd, buf });

    //     Ok(())
    // }

    // // Установка терминала в режим non-canonical
    // fn set_keypress_mode(termios: &mut Termios) {
    //     termios.input_flags &= !(InputFlags::IGNBRK
    //         | InputFlags::BRKINT
    //         | InputFlags::PARMRK
    //         | InputFlags::ISTRIP
    //         | InputFlags::INLCR
    //         | InputFlags::IGNCR
    //         | InputFlags::ICRNL
    //         | InputFlags::IXON);
    //     termios.output_flags &= !OutputFlags::OPOST;
    //     termios.local_flags &= !(LocalFlags::ECHO
    //         | LocalFlags::ECHONL
    //         | LocalFlags::ICANON
    //         | LocalFlags::ISIG
    //         | LocalFlags::IEXTEN);
    //     termios.control_flags &= !(ControlFlags::CSIZE | ControlFlags::PARENB);
    //     termios.control_flags |= ControlFlags::CS8;
    //     termios.control_chars[0] = 0;
    //     termios.control_chars[1] = 0;
    // }

    // pub fn reg_stdin_non_canonical_mode_if_not_exists(
    //     &mut self,
    //     buffer_length: usize,
    // ) -> Result<(), UnixError> {
    //     // перевожу stdin в режим non canonical для побайтовой обработки вводимых данных
    //     // добавляю в контейнер fds для дальнейшего отслеживания событий через poll
    //     let fd = std::io::stdin();

    //     let termios = termios::tcgetattr(&fd)
    //         .map_err(|e| UnixError::StdInRegisterError(format!("failed get termios: {:#?}", e)))?;
    //     let mut termios_modify = termios.clone();
    //     Self::set_keypress_mode(&mut termios_modify);
    //     termios::tcsetattr(&fd, SetArg::TCSANOW, &termios_modify).map_err(|e| {
    //         UnixError::StdInRegisterError(format!("failed set noncanonical mode stdin: {:#?}", e))
    //     })?;

    //     let buf = Buffer::try_new(buffer_length).map_err(|_e| {
    //         UnixError::AllocationError(format!(
    //             "stdin buffer allocation error: {} bytes",
    //             buffer_length
    //         ))
    //     })?;
    //     // let buf = Buffer::new(buffer_length);

    //     self.fds
    //         .insert(fd.as_raw_fd(), FileType::Stdin { fd, buf, termios });

    //     Ok(())
    // }

    // pub fn reg_stdout_if_not_exists(&mut self, buffer_length: usize) -> Result<(), UnixError> {
    //     let fd = std::io::stdout();

    //     let buf = Buffer::try_new(buffer_length).map_err(|_e| {
    //         UnixError::AllocationError(format!(
    //             "stdout buffer allocation error: {} bytes",
    //             buffer_length
    //         ))
    //     })?;
    //     // let buf = Buffer::new(buffer_length);

    //     // let fd: OwnedFd = unsafe { OwnedFd::from_raw_fd(libc::dup(fd.as_raw_fd())) };
    //     self.fds
    //         .insert(fd.as_raw_fd(), FileType::Stdout { fd, buf });

    //     Ok(())
    // }

    // pub fn reg_stderr_if_not_exists(&mut self, buffer_length: usize) -> Result<(), UnixError> {
    //     let fd = std::io::stderr();

    //     let buf = Buffer::try_new(buffer_length).map_err(|_e| {
    //         UnixError::AllocationError(format!(
    //             "stderr buffer allocation error: {} bytes",
    //             buffer_length
    //         ))
    //     })?;
    //     // let buf = Buffer::new(buffer_length);
    //     self.fds
    //         .insert(fd.as_raw_fd(), FileType::Stderr { fd, buf });

    //     Ok(())
    // }

    // pub fn reg_pty_child<S, I>(
    //     &mut self,
    //     program: S,
    //     args: Option<I>,
    //     buffer_length: usize,
    // ) -> Result<(), UnixError>
    // where
    //     I: IntoIterator<Item = S>,
    //     S: AsRef<OsStr>,
    // {
    //     // Создаем псевдотерминал (PTY)
    //     let pty = openpty(None, None)
    //         .map_err(|e| UnixError::PTYOpenError(format!("openpty error: {}", e)))?;

    //     // fork() - создает дочерний процесс из текущего
    //     // parent блок это продолжение текущего запущенного процесса
    //     // child блок это то, что выполняется в дочернем процессе
    //     // все окружение дочернего процесса наследуется из родительского
    //     let status = match unsafe { fork() } {
    //         Ok(ForkResult::Child) => {
    //             let master = pty.master.try_clone().map_err(|e| {
    //                 UnixError::PTYOpenError(format!("failed clone pty master: {:#?}", e))
    //             })?;

    //             // Перенаправляем стандартный ввод, вывод и ошибки в псевдотерминал
    //             unsafe { nix::libc::ioctl(master.as_raw_fd(), nix::libc::TIOCNOTTY) };
    //             unsafe { nix::libc::setsid() };
    //             unsafe { nix::libc::ioctl(pty.slave.as_raw_fd(), nix::libc::TIOCSCTTY) };
    //             // эта программа исполняется только в дочернем процессе
    //             // родительский процесс в это же время выполняется и что то делает

    //             // lambda функция для перенаправления stdio
    //             let new_follower_stdio = || unsafe { Stdio::from_raw_fd(pty.slave.as_raw_fd()) };

    //             // ДАЛЬНЕЙШИЙ ЗАПУСК БЕЗ FORK ПРОЦЕССА
    //             // это означает что дочерний процесс не будет еще раз разделятся
    //             // Command будет выполняться под pid этого дочернего процесса и буквально станет им
    //             // осуществляется всё это с помощью exec()
    //             let mut cmd = std::process::Command::new(program);
    //             if let Some(args) = args {
    //                 cmd.args(args);
    //             }

    //             let e = cmd
    //                 .stdin(new_follower_stdio())
    //                 .stdout(new_follower_stdio())
    //                 .stderr(new_follower_stdio())
    //                 .exec();

    //             Err(UnixError::PTYCommandError(format!("exec failed: {:#?}", e)))
    //         }
    //         Ok(ForkResult::Parent { child }) => {
    //             let buf = Buffer::try_new(buffer_length).map_err(|_e| {
    //                 UnixError::AllocationError(format!(
    //                     "pty buffer allocation error: {} bytes",
    //                     buffer_length
    //                 ))
    //             })?;

    //             self.fds.insert(
    //                 pty.master.as_raw_fd(),
    //                 FileType::PtyMaster {
    //                     master: pty.master,
    //                     buf,
    //                     slave: pty.slave,
    //                     child,
    //                 },
    //             );

    //             Ok(())
    //         }
    //         Err(e) => Err(UnixError::PTYOpenError(format!(
    //             "{:?}: {:?}: Fork failed: {}",
    //             std::thread::current().id(),
    //             std::time::SystemTime::now(),
    //             e
    //         ))),
    //     };

    //     status
    // }

    // // Создадим timerfd с задержкой
    // fn create_timer(&mut self, expiration: Expiration, buffer_length: usize) -> Result<(), UnixError> {
    //     // Другие варианты часов в Linux
    //     // CLOCK_MONOTONIC - Это тип часов, используемый для измерения времени. Он не зависит от системного времени (в отличие от CLOCK_REALTIME). Это означает, что его значение не изменяется при изменении времени системы, что полезно для измерения интервалов.
    //     // CLOCK_REALTIME – системные часы, могут изменяться при корректировке времени (например, NTP или вручную).
    //     // CLOCK_BOOTTIME – как CLOCK_MONOTONIC, но учитывает время сна (suspend).
    //     // CLOCK_MONOTONIC_RAW – “сырой” монотонный таймер без коррекций частоты CPU.
    //     // CLOCK_PROCESS_CPUTIME_ID – измеряет CPU-время только текущего процесса.
    //     // CLOCK_THREAD_CPUTIME_ID – измеряет CPU-время только текущего потока.
    //     let fd = TimerFd::new(ClockId::CLOCK_MONOTONIC, TimerFlags::TFD_NONBLOCK | TimerFlags::TFD_CLOEXEC).map_err(|e| {
    //         UnixError::TimerFdError(format!("TimerFd create failed error: {:#?}", e))
    //     })?;

    //     fd.set(expiration, TimerSetTimeFlags::empty()).map_err(|e| {
    //         UnixError::TimerFdError(format!("TimerFd set expiration {:#?} failed error: {:#?}", expiration, e))
    //     })?;

    //     let buf = Buffer::try_new(buffer_length).map_err(|_e| {
    //         UnixError::AllocationError(format!(
    //             "timerfd buffer allocation error: {} bytes",
    //             buffer_length
    //         ))
    //     })?;

    //     self.fds.insert(
    //         fd.as_fd().as_raw_fd(),
    //         FileType::TimerFd {
    //             fd,
    //             buf,
    //         },
    //     );

    //     Ok(())
    // }

    // pub fn make_pollfd(&mut self) -> &mut [libc::pollfd] {
    //     let poll_fds = self
    //         .fds
    //         .values()
    //         .map(|x| libc::pollfd {
    //             fd: x.as_raw_fd().as_raw_fd(),
    //             events: x.make_events().bits(),
    //             revents: PollFlags::empty().bits(),
    //         })
    //         .collect();

    //     self.pollfds = poll_fds;

    //     self.pollfds.as_mut_slice()
    // }

    // pub fn get_fd(&self, raw_fd: RawFd) -> &FileType {
    //     self.fds.get(&raw_fd).unwrap()
    // }

    // pub fn get_mut_fd(&mut self, raw_fd: RawFd) -> &mut FileType {
    //     self.fds.get_mut(&raw_fd).unwrap()
    // }

    // pub fn get_mut_buf(&mut self, raw_fd: RawFd) -> &mut Buffer {
    //     self.get_mut_fd(raw_fd).get_mut_buf()
    // }
}
