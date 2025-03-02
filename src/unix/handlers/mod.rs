use crate::unix::FileType;
use crate::unix::{Buffer, UnixContext, UnixError};

use log::{debug, error, info, trace};

use std::collections::VecDeque;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd, RawFd};
use std::os::unix::io::{AsRawFd, FromRawFd};
use std::time::Instant;

use nix::errno::Errno;

use nix::fcntl;
use nix::libc;
use nix::unistd::{read, write, Pid};

use nix::poll::PollFlags;

use nix::sys::signal::{SigSet, Signal};
use nix::sys::signalfd::{siginfo, SfdFlags, SignalFd};
use nix::sys::wait::{waitpid, WaitPidFlag, WaitStatus};

pub trait PollHandler<C, E> {
    fn poll_processing(&mut self, res: i32);
    fn reg_poll_error(&mut self, handler: Box<dyn PollErrorHandler<C, E>>);
    fn reg_poll_revent(&mut self, handler: Box<dyn PollReventHandler<C>>);
}

pub trait PollErrorHandler<C, E> {
    fn handle(&mut self, app: &mut C, err: E);
}

pub trait PollReventHandler<C> {
    fn handle(&mut self, app: &mut C, number_events: i32);

    fn reg_signalfd(&mut self, handler: Box<dyn SignalFdEventHandler<UnixContext>>);
    fn reg_stdin(&mut self, handler: Box<dyn StdinEventHandler<UnixContext>>);
    fn reg_stdout(&mut self, handler: Box<dyn StdoutEventHandler<UnixContext>>);
    fn reg_stderr(&mut self, handler: Box<dyn StderrEventHandler<UnixContext>>);
    fn reg_pty(&mut self, handler: Box<dyn PtyEventHandler<UnixContext>>);
}

pub trait PollInReadHandler<C> {
    fn read(&mut self, app: &mut C, raw_fd: RawFd, revents: PollFlags);
}

pub trait PollOutHandler<C> {
    fn write(&mut self, app: &mut C, raw_fd: RawFd, revents: PollFlags);
}

pub trait PollErrHandler<C> {
    fn handle(&mut self, app: &mut C, raw_fd: RawFd, revents: PollFlags);
}

pub trait PollNvalHandler<C> {
    fn handle(&mut self, app: &mut C, raw_fd: RawFd, revents: PollFlags);
}

pub trait PollHupHandler<C> {
    fn handle(&mut self, app: &mut C, raw_fd: RawFd, revents: PollFlags);
}

pub trait SignalFdEventHandler<C> {
    fn handle(&mut self, app: &mut C, raw_fd: RawFd, revents: PollFlags);

    fn reg_pollin(&mut self, handler: Box<dyn PollInReadHandler<UnixContext>>);
    fn reg_pollerr(&mut self, handler: Box<dyn PollErrHandler<UnixContext>>);
    fn reg_pollhup(&mut self, handler: Box<dyn PollHupHandler<UnixContext>>);
    fn reg_pollnval(&mut self, handler: Box<dyn PollNvalHandler<UnixContext>>);
}

pub trait StdinEventHandler<C> {
    fn handle(&mut self, app: &mut C, raw_fd: RawFd, revents: PollFlags);

    fn reg_pollin(&mut self, handler: Box<dyn PollInReadHandler<UnixContext>>);
    fn reg_pollerr(&mut self, handler: Box<dyn PollErrHandler<UnixContext>>);
    fn reg_pollhup(&mut self, handler: Box<dyn PollHupHandler<UnixContext>>);
    fn reg_pollnval(&mut self, handler: Box<dyn PollNvalHandler<UnixContext>>);
}

pub trait StdoutEventHandler<C> {
    fn handle(&mut self, app: &mut C, raw_fd: RawFd, revents: PollFlags);

    fn reg_pollin(&mut self, handler: Box<dyn PollInReadHandler<UnixContext>>);
    fn reg_pollerr(&mut self, handler: Box<dyn PollErrHandler<UnixContext>>);
    fn reg_pollhup(&mut self, handler: Box<dyn PollHupHandler<UnixContext>>);
    fn reg_pollnval(&mut self, handler: Box<dyn PollNvalHandler<UnixContext>>);
}

pub trait StderrEventHandler<C> {
    fn handle(&mut self, app: &mut C, raw_fd: RawFd, revents: PollFlags);

    fn reg_pollin(&mut self, handler: Box<dyn PollInReadHandler<UnixContext>>);
    fn reg_pollerr(&mut self, handler: Box<dyn PollErrHandler<UnixContext>>);
    fn reg_pollhup(&mut self, handler: Box<dyn PollHupHandler<UnixContext>>);
    fn reg_pollnval(&mut self, handler: Box<dyn PollNvalHandler<UnixContext>>);
}
pub trait PtyEventHandler<C> {
    fn handle(&mut self, app: &mut C, raw_fd: RawFd, revents: PollFlags);

    fn reg_pollin(&mut self, handler: Box<dyn PollInReadHandler<UnixContext>>);
    fn reg_pollerr(&mut self, handler: Box<dyn PollErrHandler<UnixContext>>);
    fn reg_pollhup(&mut self, handler: Box<dyn PollHupHandler<UnixContext>>);
    fn reg_pollnval(&mut self, handler: Box<dyn PollNvalHandler<UnixContext>>);
}

pub struct DefaultPollMiddleware {
    context: UnixContext,
    error: Option<Box<dyn PollErrorHandler<UnixContext, nix::Error>>>,
    revent: Option<Box<dyn PollReventHandler<UnixContext>>>,
}

impl DefaultPollMiddleware {
    pub fn new(context: UnixContext) -> Self {
        Self {
            context,
            error: None,
            revent: None,
        }
    }

    pub fn exit_code(&self) -> i32 {
        self.context.shutdown.code().unwrap_or(0)
    }

    pub fn exit_message(&self) -> String {
        self.context
            .shutdown
            .message()
            .unwrap_or("no message".into())
    }

    pub fn poll(&mut self, timeout: i32) -> i32 {
        self.context.event_pocess(timeout)
    }

    pub fn is_stoped(&self) -> bool {
        self.context.shutdown.is_stoped()
    }

    pub fn event_processing(&mut self) {
        while let Some(task) = self.context.queue.pop_task() {
            println!("Удаляем {:?}", task);
        }
    }

    pub fn add_signals_if_not_exists(&mut self) {
        if let Err(err) = self.context.add_signal_fd_if_not_exists() {
            let (stop_code, message) = err.into();
            self.context
                .shutdown
                .shutdown_smart(stop_code, Some(message));
        }
    }

    pub fn reg_pty_child(
        &mut self,
        program: String,
        args: Option<Vec<String>>,
        buffer_length: usize,
    ) {
        if let Err(err) = self.context.reg_pty_child(program, args, buffer_length) {
            let (stop_code, message) = err.into();
            self.context
                .shutdown
                .shutdown_smart(stop_code, Some(message));
        }
    }

    pub fn reg_stdin_non_canonical_mode_if_not_exists(&mut self, buffer_length: usize) {
        if let Err(err) = self
            .context
            .reg_stdin_non_canonical_mode_if_not_exists(buffer_length)
        {
            let (stop_code, message) = err.into();
            self.context
                .shutdown
                .shutdown_smart(stop_code, Some(message));
        }
    }

    pub fn reg_stdout_if_not_exists(&mut self, buffer_length: usize) {
        if let Err(err) = self.context.reg_stdout_if_not_exists(buffer_length) {
            let (stop_code, message) = err.into();
            self.context
                .shutdown
                .shutdown_smart(stop_code, Some(message));
        }
    }

    pub fn reg_stderr_if_not_exists(&mut self, buffer_length: usize) {
        if let Err(err) = self.context.reg_stderr_if_not_exists(buffer_length) {
            let (stop_code, message) = err.into();
            self.context
                .shutdown
                .shutdown_smart(stop_code, Some(message));
        }
    }
}

impl PollHandler<UnixContext, nix::Error> for DefaultPollMiddleware {
    fn poll_processing(&mut self, res: i32) {
        match Errno::result(res) {
            // poll error, handling
            Err(e) => {
                if let Some(h) = &mut self.error {
                    h.handle(&mut self.context, e);
                }
            }
            // poll recv event, handling
            Ok(number_events) => {
                if let Some(h) = &mut self.revent {
                    h.handle(&mut self.context, number_events);
                }
            }
        }
    }

    fn reg_poll_error(&mut self, handler: Box<dyn PollErrorHandler<UnixContext, nix::Error>>) {
        self.error = Some(handler);
    }

    fn reg_poll_revent(&mut self, handler: Box<dyn PollReventHandler<UnixContext>>) {
        self.revent = Some(handler);
    }
}

pub struct DefaultPollErrorMiddleware {}
impl DefaultPollErrorMiddleware {
    pub fn new() -> Self {
        Self {}
    }
}

impl PollErrorHandler<UnixContext, nix::Error> for DefaultPollErrorMiddleware {
    fn handle(&mut self, app: &mut UnixContext, err: nix::Error) {
        match err {
            Errno::EINTR => {
                // Системный вызов был прерван сигналом. В случае, если процесс получает сигнал во время ожидания, выполнение может быть прервано, и будет возвращен код ошибки EINTR.
                // Обычно в таком случае можно просто повторить вызов poll, если это необходимо.
                // если в poll передается signalfd, который подписан на все возможные сигналы, то эта ошибка не придет
                // она придет только если пришел сигнал, который никак не обрабатывается в signalfd
                return;
            }
            Errno::EBADF => {
                // Обработка неверного файлового дескриптора
                // Один из файловых дескрипторов в массиве, переданном в poll, является неверным, закрытым или неоткрытым.
                // Для определения ошибочного дескриптора необходимо перебрать каждый и вызвать функцию fcntl(fd, F_GETFD)
            }
            Errno::EFAULT => {
                // Обработка неверного указателя
                // Если указатель на структуру pollfd (или другие указатели) указывает на недопустимый или некорректный адрес в памяти.
            }
            Errno::EINVAL => {
                // Обработка неверного параметра
                // Если количество файловых дескрипторов (nfds) меньше или равно 0, это приведет к ошибке EINVAL.
                // Если в структуре pollfd есть неверные значения для полей (например, невалидные флаги или дескрипторы).
            }
            Errno::ENOMEM => {
                // Обработка нехватки памяти
                // Если система не может выделить достаточно памяти для выполнения операции poll
            }
            _ => {
                // любая другая ошибка (общая ошибка)
            }
        }
    }
}

pub struct DefaultPollReventMiddleware {
    signalfd: Option<Box<dyn SignalFdEventHandler<UnixContext>>>,
    stdin: Option<Box<dyn StdinEventHandler<UnixContext>>>,
    stdout: Option<Box<dyn StdoutEventHandler<UnixContext>>>,
    stderr: Option<Box<dyn StderrEventHandler<UnixContext>>>,
    pty: Option<Box<dyn PtyEventHandler<UnixContext>>>,
}
impl DefaultPollReventMiddleware {
    pub fn new() -> Self {
        Self {
            signalfd: None,
            stdin: None,
            stdout: None,
            stderr: None,
            pty: None,
        }
    }
}

impl PollReventHandler<UnixContext> for DefaultPollReventMiddleware {
    fn reg_signalfd(&mut self, handler: Box<dyn SignalFdEventHandler<UnixContext>>) {
        self.signalfd = Some(handler);
    }

    fn reg_stdin(&mut self, handler: Box<dyn StdinEventHandler<UnixContext>>) {
        self.stdin = Some(handler);
    }

    fn reg_stdout(&mut self, handler: Box<dyn StdoutEventHandler<UnixContext>>) {
        self.stdout = Some(handler);
    }

    fn reg_stderr(&mut self, handler: Box<dyn StderrEventHandler<UnixContext>>) {
        self.stderr = Some(handler);
    }

    fn reg_pty(&mut self, handler: Box<dyn PtyEventHandler<UnixContext>>) {
        self.pty = Some(handler);
    }

    fn handle(&mut self, app: &mut UnixContext, number_events: i32) {
        trace!("number_events: {}", number_events);

        if number_events == 0 {
            // poll timeout if number_events is zero
            return;
        }

        // перебираем все pollfd в списке
        for pfd in app.pollfds.clone().iter_mut() {
            if pfd.revents == 0 {
                // события нет, переходим к следующему
                continue;
            }

            // забираем revent, в нем содержиться информация о событиях для этого дескриптора
            let revents = PollFlags::from_bits(pfd.revents).unwrap();

            // вытаскиваем fd
            match app.get_fd(pfd.fd) {
                FileType::Stdin { .. } => {
                    if let Some(h) = &mut self.stdin {
                        h.handle(app, pfd.fd, revents);
                    }
                }
                FileType::Stdout { .. } => {
                    if let Some(h) = &mut self.stdout {
                        h.handle(app, pfd.fd, revents);
                    }
                }
                FileType::Stderr { .. } => {
                    if let Some(h) = &mut self.stderr {
                        h.handle(app, pfd.fd, revents);
                    }
                }
                FileType::SignalFd { .. } => {
                    if let Some(h) = &mut self.signalfd {
                        h.handle(app, pfd.fd, revents);
                    }
                }
                FileType::PtyMaster { .. } => {
                    if let Some(h) = &mut self.pty {
                        h.handle(app, pfd.fd, revents);
                    }
                }
            }

            // обнуляем revents сразу же, так как в этом поле ядро linux пишет флаги произошедших событий
            // нужно что бы перед вызовом poll, это поле было обнулено
            pfd.revents = 0;
        }
    }
}

pub struct DefaultStdinHandler {
    pollin: Option<Box<dyn PollInReadHandler<UnixContext>>>,
    pollerr: Option<Box<dyn PollErrHandler<UnixContext>>>,
    pollhup: Option<Box<dyn PollHupHandler<UnixContext>>>,
    pollnval: Option<Box<dyn PollNvalHandler<UnixContext>>>,
}

impl DefaultStdinHandler {
    pub fn new() -> Self {
        Self {
            pollin: None,
            pollerr: None,
            pollhup: None,
            pollnval: None,
        }
    }
}

impl StdinEventHandler<UnixContext> for DefaultStdinHandler {
    fn handle(&mut self, app: &mut UnixContext, raw_fd: RawFd, revents: PollFlags) {
        if revents.contains(PollFlags::POLLERR) {
            if let Some(h) = &mut self.pollerr {
                h.handle(app, raw_fd, revents);
            }
        }
        if revents.contains(PollFlags::POLLNVAL) {
            if let Some(h) = &mut self.pollnval {
                h.handle(app, raw_fd, revents);
            }
        }
        if revents.contains(PollFlags::POLLHUP) {
            if let Some(h) = &mut self.pollhup {
                h.handle(app, raw_fd, revents);
            }
        }
        if revents.contains(PollFlags::POLLIN) {
            if let Some(h) = &mut self.pollin {
                h.read(app, raw_fd, revents);
            }
        }
    }

    fn reg_pollin(&mut self, handler: Box<dyn PollInReadHandler<UnixContext>>) {
        self.pollin = Some(handler);
    }
    fn reg_pollerr(&mut self, handler: Box<dyn PollErrHandler<UnixContext>>) {
        self.pollerr = Some(handler);
    }
    fn reg_pollhup(&mut self, handler: Box<dyn PollHupHandler<UnixContext>>) {
        self.pollhup = Some(handler);
    }
    fn reg_pollnval(&mut self, handler: Box<dyn PollNvalHandler<UnixContext>>) {
        self.pollnval = Some(handler);
    }
}

pub struct DefaultSignalfdMiddleware {
    pollin: Option<Box<dyn PollInReadHandler<UnixContext>>>,
    pollerr: Option<Box<dyn PollErrHandler<UnixContext>>>,
    pollhup: Option<Box<dyn PollHupHandler<UnixContext>>>,
    pollnval: Option<Box<dyn PollNvalHandler<UnixContext>>>,
}

impl DefaultSignalfdMiddleware {
    pub fn new() -> Self {
        Self {
            pollin: None,
            pollerr: None,
            pollhup: None,
            pollnval: None,
        }
    }

    pub fn map_to_siginfo<'a>(&mut self, buf: &'a mut [u8]) -> &'a mut siginfo {
        unsafe { &mut *(buf.as_ptr() as *mut siginfo) }
    }

    pub fn waitpid(&self, pid: Pid) -> nix::Result<WaitStatus> {
        trace!("check child process {} is running...", pid);

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

impl SignalFdEventHandler<UnixContext> for DefaultSignalfdMiddleware {
    fn handle(&mut self, app: &mut UnixContext, raw_fd: RawFd, revents: PollFlags) {
        if revents.contains(PollFlags::POLLERR) {
            if let Some(h) = &mut self.pollerr {
                h.handle(app, raw_fd, revents);
            }
        }
        if revents.contains(PollFlags::POLLNVAL) {
            if let Some(h) = &mut self.pollnval {
                h.handle(app, raw_fd, revents);
            }
        }
        if revents.contains(PollFlags::POLLHUP) {
            if let Some(h) = &mut self.pollhup {
                h.handle(app, raw_fd, revents);
            }
        }
        if revents.contains(PollFlags::POLLIN) {
            if let Some(h) = &mut self.pollin {
                h.read(app, raw_fd, revents);

                let (signal, ssi_pid, ssi_uid, ssi_status, ssi_utime, ssi_stime) = {
                    let buf = app.get_mut_buf(raw_fd);
                    let buf = self.map_to_siginfo(buf.get_mut_buffer_slice());
                    (
                        Signal::try_from(buf.ssi_signo as i32).unwrap(),
                        buf.ssi_pid,
                        buf.ssi_uid,
                        buf.ssi_status,
                        buf.ssi_utime,
                        buf.ssi_stime,
                    )
                };

                let message = format!("{signal} from pid: {ssi_pid} (uid: {ssi_uid})");

                debug!("{message}");

                if signal == Signal::SIGTERM {
                    app.shutdown.shutdown_smart(0, Some(message.clone()));
                }

                if signal == Signal::SIGINT {
                    app.shutdown.shutdown_fast(0, Some(message.clone()));
                }

                if signal == Signal::SIGQUIT {
                    app.shutdown.shutdown_immediate(0, Some(message.clone()));
                }

                if signal == Signal::SIGCHLD {
                    trace!("status: {ssi_status} (ssi_utime: {ssi_utime}, ssi_stime: {ssi_stime})");
                    let res = self.waitpid(Pid::from_raw(ssi_pid as i32));
                    trace!("waitpid({}) = {:#?}", ssi_pid, res);
                }
            }
        }
    }
    fn reg_pollin(&mut self, handler: Box<dyn PollInReadHandler<UnixContext>>) {
        self.pollin = Some(handler);
    }
    fn reg_pollerr(&mut self, handler: Box<dyn PollErrHandler<UnixContext>>) {
        self.pollerr = Some(handler);
    }
    fn reg_pollhup(&mut self, handler: Box<dyn PollHupHandler<UnixContext>>) {
        self.pollhup = Some(handler);
    }
    fn reg_pollnval(&mut self, handler: Box<dyn PollNvalHandler<UnixContext>>) {
        self.pollnval = Some(handler);
    }
}

pub struct DefaultPtyMiddleware {
    pollin: Option<Box<dyn PollInReadHandler<UnixContext>>>,
    pollerr: Option<Box<dyn PollErrHandler<UnixContext>>>,
    pollhup: Option<Box<dyn PollHupHandler<UnixContext>>>,
    pollnval: Option<Box<dyn PollNvalHandler<UnixContext>>>,
}

impl DefaultPtyMiddleware {
    pub fn new() -> Self {
        Self {
            pollin: None,
            pollerr: None,
            pollhup: None,
            pollnval: None,
        }
    }
}

impl PtyEventHandler<UnixContext> for DefaultPtyMiddleware {
    fn handle(&mut self, app: &mut UnixContext, raw_fd: RawFd, revents: PollFlags) {
        if revents.contains(PollFlags::POLLERR) {
            if let Some(h) = &mut self.pollerr {
                h.handle(app, raw_fd, revents);
            }
        }
        if revents.contains(PollFlags::POLLNVAL) {
            if let Some(h) = &mut self.pollnval {
                h.handle(app, raw_fd, revents);
            }
        }
        if revents.contains(PollFlags::POLLHUP) {
            if let Some(h) = &mut self.pollhup {
                h.handle(app, raw_fd, revents);
            }
        }
        if revents.contains(PollFlags::POLLIN) {
            if let Some(h) = &mut self.pollin {
                h.read(app, raw_fd, revents);
            }
        }
    }
    fn reg_pollin(&mut self, handler: Box<dyn PollInReadHandler<UnixContext>>) {
        self.pollin = Some(handler);
    }
    fn reg_pollerr(&mut self, handler: Box<dyn PollErrHandler<UnixContext>>) {
        self.pollerr = Some(handler);
    }
    fn reg_pollhup(&mut self, handler: Box<dyn PollHupHandler<UnixContext>>) {
        self.pollhup = Some(handler);
    }
    fn reg_pollnval(&mut self, handler: Box<dyn PollNvalHandler<UnixContext>>) {
        self.pollnval = Some(handler);
    }
}

pub struct DefaultPollInReadHandler {}

impl DefaultPollInReadHandler {
    pub fn new() -> Self {
        Self {}
    }
}

impl PollInReadHandler<UnixContext> for DefaultPollInReadHandler {
    fn read(&mut self, app: &mut UnixContext, raw_fd: RawFd, revents: PollFlags) {
        trace!("fd {} ready for reading", raw_fd);

        let buf = app.get_mut_buf(raw_fd);

        let mut retry = 10;

        while retry > 0 {
            // Читаем данные и обрабатываем их
            let res = read(raw_fd, buf.get_mut_buffer_slice());

            match res {
                Ok(n) => {
                    // read n bytes
                    trace!("read = Ok({n}) bytes");
                    buf.set_data_len(n);
                    retry = 0;
                }
                Err(Errno::EAGAIN) => {
                    // дескриптор установлен в неблокирующий режим, но данных пока нет. Верно просто пропускать и ждать следующего срабатывания poll.
                    trace!(
                        "non-blocking reading mode is enabled (SFD_NONBLOCK). fd {:?} doesn't data",
                        raw_fd,
                    );
                    buf.set_data_len(0);
                    retry = 0;
                }
                Err(Errno::EBADF) => {
                    // Аргумент fd не является допустимым дескриптором файла, открытым для чтения.
                    // Это может значить, что он был закрыт или никогда не существовал.
                    // Удалить его из списка наблюдаемых дескрипторов.
                    buf.set_data_len(0);
                    retry = 0;
                }
                Err(Errno::EINTR) => {
                    // Операция чтения была прервана из-за получения сигнала, и данные не были переданы.
                    // Здесь можно просто повторить read
                    buf.set_data_len(0);
                    retry = 0;
                }
                Err(Errno::EINVAL) => {
                    // Файл является обычным или блочным специальным файлом, а аргумент смещение отрицательный.
                    // ошибка может возникать если передан некорретный buf, например нулевой длинны
                    // если возникает, стоит логировать, так как это признак ошибки в коде (например, передан неверный аргумент offset).
                    trace!("fd {} EINVAL", raw_fd);
                    let setting_len = buf.get_setting_len();
                    let buffer_len = buf.get_buffer_len();

                    if retry > 0 {
                        trace!("buffer_len = {buffer_len}, setting_len = {setting_len}");
                        let new_buffer_len = if buffer_len < setting_len {
                            // если текущий буфер меньше, чем размер, который установил пользователь
                            //то увеличим его до размера, который установил пользователь
                            setting_len
                        } else {
                            // если текущий буфер больше размера, установленного пользователем,
                            // однако все равно не удалось прочитать данные и была получена ошибка EINVAL
                            // то надо попробовать увеличить размер буфера в 2 раза и повторить чтение
                            buffer_len * 2
                        };

                        trace!("set buffer_len to {new_buffer_len} and read fd: {raw_fd} retry");
                        buf.reallocate(new_buffer_len);
                        retry -= 1;
                    } else {
                        buf.set_data_len(0);
                        retry = 0;
                    }
                }
                Err(Errno::ECONNRESET) => {
                    // Была предпринята попытка чтения из сокета, и соединение было принудительно закрыто его партнёром.
                    // соединение было закрыто принудительно, нужно закрыть дескриптор и удалить его из списка.
                    buf.set_data_len(0);
                    retry = 0;
                }
                Err(Errno::ENOTCONN) => {
                    // Была предпринята попытка чтения из сокета, который не подключен.
                    // сокет не подключен, тоже стоит удалить fd.
                    buf.set_data_len(0);
                    retry = 0;
                }
                Err(Errno::ETIMEDOUT) => {
                    // Была предпринята попытка чтения из сокета, и произошел тайм-аут передачи.
                    // тайм-аут соединения. Если это TCP-сокет, вероятно, соединение закрылось → удалить fd.
                    buf.set_data_len(0);
                    retry = 0;
                }
                Err(Errno::EIO) => {
                    // Произошла физическая ошибка ввода-вывода.
                    // Это может быть связано с проблемами на уровне железа, стоит логировать и удалить fd.
                    buf.set_data_len(0);
                    retry = 0;
                }
                Err(Errno::ENOBUFS) => {
                    // В системе было недостаточно ресурсов для выполнения этой операции.
                    // нехватка ресурсов. Можно попробовать повторить позже, но если ошибка повторяется, логировать и, возможно, завершить работу (в зависимости от критичности).
                    buf.set_data_len(0);
                    retry = 0;
                }
                Err(Errno::ENOMEM) => {
                    // Для выполнения запроса недостаточно памяти
                    // нехватка ресурсов. Можно попробовать повторить позже, но если ошибка повторяется, логировать и, возможно, завершить работу (в зависимости от критичности).
                    buf.set_data_len(0);
                    retry = 0;
                }
                Err(Errno::ENXIO) => {
                    // Был отправлен запрос несуществующему устройству или запрос выходил за рамки возможностей устройства.
                    // устройство не существует или запрос вне его диапазона. Вероятно, fd устарел, его следует удалить.
                    buf.set_data_len(0);
                    retry = 0;
                }
                Err(e) => {
                    error!("read = Err({})", e);
                    buf.set_data_len(0);
                    retry = 0;
                }
            }
        }
    }
}

pub struct DefaultPollOutHandler {}

impl DefaultPollOutHandler {
    pub fn new() -> Self {
        Self {}
    }
}

impl PollOutHandler<UnixContext> for DefaultPollOutHandler {
    fn write(&mut self, app: &mut UnixContext, raw_fd: RawFd, revents: PollFlags) {
        trace!("fd {} ready for writing", raw_fd);
    }
}

pub struct DefaultPollErrHandler {}

impl DefaultPollErrHandler {
    pub fn new() -> Self {
        Self {}
    }
}

impl PollErrHandler<UnixContext> for DefaultPollErrHandler {
    fn handle(&mut self, app: &mut UnixContext, raw_fd: RawFd, revents: PollFlags) {
        trace!("fd {}: POLLERR (I/O error)", raw_fd);
    }
}

pub struct DefaultPollNvalHandler {}

impl DefaultPollNvalHandler {
    pub fn new() -> Self {
        Self {}
    }
}

impl PollNvalHandler<UnixContext> for DefaultPollNvalHandler {
    fn handle(&mut self, app: &mut UnixContext, raw_fd: RawFd, revents: PollFlags) {
        trace!("fd {}: POLLNVAL (invalid descriptor)", raw_fd);
    }
}

pub struct DefaultPollHupHandler {}

impl DefaultPollHupHandler {
    pub fn new() -> Self {
        Self {}
    }
}

impl PollHupHandler<UnixContext> for DefaultPollHupHandler {
    fn handle(&mut self, app: &mut UnixContext, raw_fd: RawFd, revents: PollFlags) {
        trace!("fd {}: POLLHUP (peer closed connection)", raw_fd);
    }
}
