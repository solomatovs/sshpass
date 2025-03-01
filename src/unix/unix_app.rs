use std::alloc::{self, AllocError, Layout};
use std::collections::HashMap;
use std::ops::{Deref, DerefMut};
// use std::borrow::{Borrow, BorrowMut};
// use std::boxed::Box;
// use std::cell::{Ref, RefCell};
// use std::io::{Read, Stdin, StdinLock};
// use std::ops::{Deref, DerefMut};
use nix::pty::openpty;
use nix::unistd::Pid;
use nix::unistd::{fork, ForkResult};
use std::ffi::OsStr;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd, RawFd};
use std::os::unix::io::{AsRawFd, FromRawFd};
use std::os::unix::process::CommandExt;
use std::process::Stdio;

use nix::libc;
use nix::poll::PollFlags;
use nix::sys::signalfd::{siginfo, SfdFlags, SignalFd};

use nix::sys::termios::{self, ControlFlags, InputFlags, LocalFlags, OutputFlags, SetArg, Termios};

use log::trace;

use nix::sys::signal::{SigSet, Signal};

use std::time::Instant;

use crate::unix::PollHandler;

use std::fmt;

#[derive(Debug)]
pub enum UnixError {
    AllocationError(String),
    OpenPTYError(String),
    CommandError(String),
    SignalFdError(String),
    StdInRegisterError(String),
}

impl fmt::Display for UnixError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "NixError")
    }
}

impl std::error::Error for UnixError {}

#[derive(Debug)]
pub struct AppShutdown {
    is_stoped: bool,
    is_stop: bool,
    stop_time: Option<Instant>,
    stop_code: Option<i32>,
    stop_message: Option<String>,
}

impl AppShutdown {
    pub fn new() -> Self {
        Self {
            is_stoped: false,
            is_stop: false,
            stop_time: None,
            stop_code: None,
            stop_message: None,
        }
    }

    pub fn is_stop(&self) -> bool {
        self.is_stop
    }

    pub fn is_stoped(&self) -> bool {
        self.is_stoped
    }

    pub fn shutdown_starting(&mut self, stop_code: i32, error: Option<String>) {
        self.stop_time = Some(Instant::now());
        self.is_stop = true;
        self.is_stoped = false;
        self.stop_code = Some(stop_code);
        self.stop_message = error;
    }

    pub fn shutdown_complited(&mut self) {
        self.is_stop = false;
        self.is_stoped = true;
    }

    pub fn shutdown_cancel(&mut self) {
        self.is_stop = false;
        self.is_stoped = false;
        self.stop_time = None;
        self.stop_code = None;
        self.stop_message = None;
    }

    pub fn stop_code(&self) -> i32 {
        self.stop_code.unwrap_or(255)
    }

    pub fn stop_message(&self) -> String {
        self.stop_message.as_deref().unwrap_or("no message").into()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Buffer {
    buf: Vec<u8>,
    len: usize,
}

impl Buffer {
    pub fn try_new(size: usize) -> Result<Self, AllocError> {
        // Обработка случая с нулевой емкостью
        if size == 0 {
            return Ok(Self {
                buf: Vec::new(),
                len: 0,
            });
        }

        // Проверка на переполнение при выделении памяти
        let layout = match Layout::array::<u8>(size) {
            Ok(layout) => layout,
            Err(_) => return Err(AllocError),
        };

        unsafe {
            // Попытка выделить память
            let ptr = alloc::alloc(layout);

            // Проверка на ошибку аллокации
            if ptr.is_null() {
                return Err(AllocError);
            }

            // Преобразование в Vec
            let buf = Vec::from_raw_parts(ptr, 0, size);
            Ok(Self { buf, len: 0 })
        }
    }

    pub fn set_len(&mut self, len: usize) {
        self.len = len.min(self.buf.capacity());
    }

    pub fn get_slice(&self) -> &[u8] {
        &self.buf[..self.len]
    }

    pub fn get_mut_slice(&mut self) -> &mut [u8] {
        &mut self.buf[..self.len]
    }

    pub fn get_mut_all_slice(&mut self) -> &mut [u8] {
        &mut self.buf
    }
}

impl Deref for Buffer {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        &self.buf[..self.len]
    }
}

impl DerefMut for Buffer {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.buf[..self.len]
    }
}

#[derive(Debug, Clone)]
pub struct BufferPool {
    buffers: Vec<Buffer>,
    max_size: usize,
    buffer_size: usize,
}

impl BufferPool {
    pub fn try_new(max_size: usize, buffer_size: usize) -> Result<Self, AllocError> {
        Ok(Self {
            buffers: Vec::new(), // Пустой вектор не вызовет ошибку аллокации
            max_size,
            buffer_size,
        })
    }

    pub fn try_add_buffer(&mut self, buffer: Buffer) -> Result<(), AllocError> {
        if self.buffers.len() < self.max_size {
            // try_reserve для одного элемента
            self.buffers.try_reserve(1).map_err(|_| AllocError)?;
            self.buffers.push(buffer);
        }
        Ok(())
    }

    // Этот метод не требует изменений, так как не аллоцирует память
    pub fn get_next_buffer(&mut self) -> Option<Buffer> {
        self.buffers.pop()
    }

    pub fn try_allocate_buffer(&mut self) -> Result<Option<Buffer>, AllocError> {
        if self.buffers.len() < self.max_size {
            Buffer::try_new(self.buffer_size).map(Some)
        } else {
            Ok(None)
        }
    }
}

impl IntoIterator for BufferPool {
    type Item = Buffer;
    type IntoIter = std::vec::IntoIter<Buffer>;

    fn into_iter(self) -> Self::IntoIter {
        self.buffers.into_iter()
    }
}

impl<'a> IntoIterator for &'a BufferPool {
    type Item = &'a Buffer;
    type IntoIter = std::slice::Iter<'a, Buffer>;

    fn into_iter(self) -> Self::IntoIter {
        self.buffers.iter()
    }
}

impl<'a> IntoIterator for &'a mut BufferPool {
    type Item = &'a mut Buffer;
    type IntoIter = std::slice::IterMut<'a, Buffer>;

    fn into_iter(self) -> Self::IntoIter {
        self.buffers.iter_mut()
    }
}

#[derive(Debug)]
pub enum FileType {
    Stdin {
        fd: std::io::Stdin,
        buf: Buffer,
        termios: Termios,
    },
    Stdout {
        fd: std::io::Stdout,
        buf: Buffer,
    },
    Stderr {
        fd: std::io::Stderr,
        buf: Buffer,
    },
    SignalFd {
        fd: SignalFd,
        buf: Buffer,
    },
    PtyMaster {
        master: OwnedFd,
        buf: Buffer,
        slave: OwnedFd,
        child: Pid,
    },
}

impl std::fmt::Display for FileType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FileType::Stdin { fd, buf, .. } => {
                write!(f, "Stdin(fd: {}, buf_size: {})", fd.as_raw_fd(), buf.len)
            }
            FileType::Stdout { fd, buf } => {
                write!(f, "Stdout(fd: {}, buf_size: {})", fd.as_raw_fd(), buf.len)
            }
            FileType::Stderr { fd, buf } => {
                write!(f, "Stderr(fd: {}, buf_size: {})", fd.as_raw_fd(), buf.len)
            }
            FileType::SignalFd { fd, buf } => {
                write!(f, "SignalFd(fd: {}, buf_size: {})", fd.as_raw_fd(), buf.len)
            }
            FileType::PtyMaster {
                master, buf, child, ..
            } => {
                write!(
                    f,
                    "PtyMaster(fd: {}, buf_size: {}, child_pid: {})",
                    master.as_raw_fd(),
                    buf.len,
                    child
                )
            }
        }
    }
}

impl FileType {
    pub fn as_fd(&self) -> BorrowedFd {
        match self {
            FileType::Stdin { fd, .. } => fd.as_fd(),
            FileType::Stdout { fd, .. } => fd.as_fd(),
            FileType::Stderr { fd, .. } => fd.as_fd(),
            FileType::SignalFd { fd, .. } => fd.as_fd(),
            FileType::PtyMaster { master, .. } => master.as_fd(),
        }
    }

    pub fn as_raw_fd(&self) -> i32 {
        match self {
            FileType::Stdin { fd, .. } => fd.as_raw_fd(),
            FileType::Stdout { fd, .. } => fd.as_raw_fd(),
            FileType::Stderr { fd, .. } => fd.as_raw_fd(),
            FileType::SignalFd { fd, .. } => fd.as_raw_fd(),
            FileType::PtyMaster { master, .. } => master.as_raw_fd(),
        }
    }

    pub fn make_events(&self) -> PollFlags {
        match self {
            FileType::Stdin { .. } => {
                PollFlags::POLLIN | PollFlags::POLLERR | PollFlags::POLLHUP | PollFlags::POLLNVAL
            }
            FileType::Stdout { .. } => {
                PollFlags::POLLOUT | PollFlags::POLLERR | PollFlags::POLLHUP | PollFlags::POLLNVAL
            }
            FileType::Stderr { .. } => {
                PollFlags::POLLOUT | PollFlags::POLLERR | PollFlags::POLLHUP | PollFlags::POLLNVAL
            }
            FileType::SignalFd { .. } => {
                PollFlags::POLLIN | PollFlags::POLLERR | PollFlags::POLLHUP | PollFlags::POLLNVAL
            }
            FileType::PtyMaster { .. } => {
                PollFlags::POLLIN
                    | PollFlags::POLLOUT
                    | PollFlags::POLLERR
                    | PollFlags::POLLHUP
                    | PollFlags::POLLNVAL
            }
        }
    }

    pub fn get_mut_buf(&mut self) -> &mut Buffer {
        match self {
            FileType::Stdin { buf, .. } => buf,
            FileType::Stdout { buf, .. } => buf,
            FileType::Stderr { buf, .. } => buf,
            FileType::SignalFd { buf, .. } => buf,
            FileType::PtyMaster { buf, .. } => buf,
        }
    }
}

#[derive(Debug)]
pub enum PollEvent {
    Timeout,
    Event(RawFd),
}

pub enum UnixEvent {
    Stdin(usize),
    Stdout(usize),
    Stderr(usize),
    PtyMaster(usize),
    Signal(usize),
    NotHandle,
}

// #[derive(Debug)]
pub struct UnixContext {
    fds: HashMap<RawFd, FileType>,
    pub pollfds: Vec<libc::pollfd>,
    pub shutdown: AppShutdown,
}

impl UnixContext {
    pub fn new() -> Self {
        // Создаем контейнер для дескрипторов, который будет опрашиваться через poll
        Self {
            fds: HashMap::new(), // Пустой HashMap не вызовет ошибку аллокации
            pollfds: Vec::new(),
            shutdown: AppShutdown::new(),
        }
    }

    // pub fn reg_handler(&mut self, handler: impl PollHandler<UnixContext> + 'static) {
    //     self.handler = Some(Box::new(handler));
    // }

    pub fn bootstrap_base(&mut self, buffer_size: usize) {
        self.reg_stdin_non_canonical_mode_if_not_exists(buffer_size)
            .and_then(|_| self.reg_stdout_if_not_exists(buffer_size))
            .and_then(|_| self.reg_stderr_if_not_exists(buffer_size))
            .and_then(|_| self.reg_signals_if_not_exists())
            .map_err(|e| {
                self.shutdown
                    .shutdown_starting(-1, Some(format!("error bootstraping app: {:#?}", e)));
            });
    }

    pub fn bootstrap_child<S, I>(&mut self, program: S, args: Option<I>, buffer_length: usize)
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.reg_pty_child(program, args, buffer_length)
            .map_err(|e| {
                self.shutdown
                    .shutdown_starting(-1, Some(format!("error bootstraping app: {:#?}", e)));
            });
    }

    pub fn reg_signals_if_not_exists(&mut self) -> Result<(), UnixError> {
        let mut mask = SigSet::empty();

        // добавляю в обработчик все сигналы
        for signal in Signal::iterator() {
            mask.add(signal);
        }

        let mut new_mask = SigSet::thread_get_mask()
            .map_err(|e| UnixError::SignalFdError(format!("failed get thread mask: {:#?}", e)))?;
        for s in mask.into_iter() {
            new_mask.add(s);
        }

        new_mask
            .thread_block()
            .map_err(|e| UnixError::SignalFdError(format!("failed set thread mask: {:#?}", e)))?;

        let fd = SignalFd::with_flags(&new_mask, SfdFlags::SFD_NONBLOCK | SfdFlags::SFD_CLOEXEC)
            .map_err(|e| {
                UnixError::SignalFdError(format!("signalfd create failed error: {:#?}", e))
            })?;

        let size = std::mem::size_of::<siginfo>();
        let buf = Buffer::try_new(size).map_err(|_e| {
            UnixError::AllocationError(format!("signal fd buffer allocation error: {} bytes", size))
        })?;

        self.fds
            .insert(fd.as_raw_fd(), FileType::SignalFd { fd, buf });

        Ok(())
    }

    // Установка терминала в режим non-canonical
    fn set_keypress_mode(termios: &mut Termios) {
        termios.input_flags &= !(InputFlags::IGNBRK
            | InputFlags::BRKINT
            | InputFlags::PARMRK
            | InputFlags::ISTRIP
            | InputFlags::INLCR
            | InputFlags::IGNCR
            | InputFlags::ICRNL
            | InputFlags::IXON);
        termios.output_flags &= !OutputFlags::OPOST;
        termios.local_flags &= !(LocalFlags::ECHO
            | LocalFlags::ECHONL
            | LocalFlags::ICANON
            | LocalFlags::ISIG
            | LocalFlags::IEXTEN);
        termios.control_flags &= !(ControlFlags::CSIZE | ControlFlags::PARENB);
        termios.control_flags |= ControlFlags::CS8;
        termios.control_chars[0] = 0;
        termios.control_chars[1] = 0;
    }

    pub fn reg_stdin_non_canonical_mode_if_not_exists(
        &mut self,
        buffer_length: usize,
    ) -> Result<(), UnixError> {
        // перевожу stdin в режим non canonical для побайтовой обработки вводимых данных
        // добавляю в контейнер fds для дальнейшего отслеживания событий через poll
        let fd = std::io::stdin();

        let termios = termios::tcgetattr(&fd)
            .map_err(|e| UnixError::StdInRegisterError(format!("failed get termios: {:#?}", e)))?;
        let mut termios_modify = termios.clone();
        Self::set_keypress_mode(&mut termios_modify);
        termios::tcsetattr(&fd, SetArg::TCSANOW, &termios_modify).map_err(|e| {
            UnixError::StdInRegisterError(format!("failed set noncanonical mode stdin: {:#?}", e))
        })?;

        let buf = Buffer::try_new(buffer_length).map_err(|_e| {
            UnixError::AllocationError(format!(
                "stdin buffer allocation error: {} bytes",
                buffer_length
            ))
        })?;

        self.fds
            .insert(fd.as_raw_fd(), FileType::Stdin { fd, buf, termios });

        Ok(())
    }

    pub fn reg_stdout_if_not_exists(&mut self, buffer_length: usize) -> Result<(), UnixError> {
        let fd = std::io::stdout();

        let buf = Buffer::try_new(buffer_length).map_err(|_e| {
            UnixError::AllocationError(format!(
                "stdout buffer allocation error: {} bytes",
                buffer_length
            ))
        })?;

        self.fds
            .insert(fd.as_raw_fd(), FileType::Stdout { fd, buf });

        Ok(())
    }

    pub fn reg_stderr_if_not_exists(&mut self, buffer_length: usize) -> Result<(), UnixError> {
        let fd = std::io::stderr();

        let buf = Buffer::try_new(buffer_length).map_err(|_e| {
            UnixError::AllocationError(format!(
                "stderr buffer allocation error: {} bytes",
                buffer_length
            ))
        })?;

        self.fds
            .insert(fd.as_raw_fd(), FileType::Stderr { fd, buf });

        Ok(())
    }

    pub fn reg_pty_child<S, I>(
        &mut self,
        program: S,
        args: Option<I>,
        buffer_length: usize,
    ) -> Result<(), UnixError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        // Создаем псевдотерминал (PTY)
        let pty = openpty(None, None)
            .map_err(|e| UnixError::OpenPTYError(format!("openpty error: {}", e)))?;

        // fork() - создает дочерний процесс из текущего
        // parent блок это продолжение текущего запущенного процесса
        // child блок это то, что выполняется в дочернем процессе
        // все окружение дочернего процесса наследуется из родительского
        let status = match unsafe { fork() } {
            Ok(ForkResult::Child) => {
                let master = pty.master.try_clone().map_err(|e| {
                    UnixError::OpenPTYError(format!("failed clone pty master: {:#?}", e))
                })?;

                // Перенаправляем стандартный ввод, вывод и ошибки в псевдотерминал
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

                Err(UnixError::CommandError(format!("exec failed: {:#?}", e)))
            }
            Ok(ForkResult::Parent { child }) => {
                let buf = Buffer::try_new(buffer_length).map_err(|_e| {
                    UnixError::AllocationError(format!(
                        "pty buffer allocation error: {} bytes",
                        buffer_length
                    ))
                })?;

                // Добавляем дескриптор PTY для мониторинга событий
                self.fds.insert(
                    pty.master.as_raw_fd(),
                    FileType::PtyMaster {
                        master: pty.master,
                        buf,
                        slave: pty.slave,
                        child,
                    },
                );

                Ok(())
            }
            Err(e) => Err(UnixError::OpenPTYError(format!(
                "{:?}: {:?}: Fork failed: {}",
                std::thread::current().id(),
                std::time::SystemTime::now(),
                e
            ))),
        };

        status
    }

    pub fn make_pollfd(&mut self) -> &mut [libc::pollfd] {
        let poll_fds = self
            .fds
            .values()
            .map(|x| libc::pollfd {
                fd: x.as_raw_fd().as_raw_fd(),
                events: x.make_events().bits(),
                revents: PollFlags::empty().bits(),
            })
            .collect();

        self.pollfds = poll_fds;

        self.pollfds.as_mut_slice()
    }

    pub fn get_fd(&self, raw_fd: RawFd) -> &FileType {
        self.fds.get(&raw_fd).unwrap()
    }

    pub fn get_mut_fd(&mut self, raw_fd: RawFd) -> &mut FileType {
        self.fds.get_mut(&raw_fd).unwrap()
    }

    pub fn get_mut_buf(&mut self, raw_fd: RawFd) -> &mut Buffer {
        self.get_mut_fd(raw_fd).get_mut_buf()
    }

    // pub fn stop_code(&self) -> i32 {
    //     self.shutdown.stop_code()
    // }

    // pub fn is_stoped(&self) -> bool {
    //     self.shutdown.is_stoped()
    // }

    pub fn event_pocess(
        &mut self,
        poll_timeout: i32,
        // poll_handler: &mut impl PollHandler<UnixApp>,
    ) -> i32 {
        trace!("poll(&mut fds, {:?})", poll_timeout);

        let poller = self.make_pollfd();
        let res = unsafe {
            libc::poll(
                poller.as_mut_ptr().cast(),
                poller.len() as libc::nfds_t,
                poll_timeout,
            )
        };

        trace!("poll result: {:?}", res);

        // poll_handler.handle(self, res);

        res
    }
}
