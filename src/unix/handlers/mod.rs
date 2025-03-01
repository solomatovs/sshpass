use crate::unix::FileType;
use crate::unix::UnixContext;

use log::{debug, error, info, trace};
use nix::errno::Errno;
use nix::poll::PollFlags;
use nix::sys::signalfd::siginfo;
use nix::sys::wait::{waitpid, WaitPidFlag, WaitStatus};
use nix::unistd::read;
use nix::unistd::Pid;
use std::os::fd::RawFd;


pub trait PollHandler<C> {
    fn handle(&mut self, res: i32);
}

pub trait PollErrorHandler<C, E> {
    fn handle(&mut self, app: &mut C, err: E);
}

pub trait PollReventHandler<C> {
    fn handle(&mut self, app: &mut C, number_events: i32);
}

pub trait SignalFdEventHandler<C> {
    fn handle(&mut self, app: &mut C, raw_fd: RawFd, revents: PollFlags);
}

pub trait StdinEventHandler<C> {
    fn handle(&mut self, app: &mut C, raw_fd: RawFd, revents: PollFlags);
}

pub trait StdoutEventHandler<C> {
    fn handle(&mut self, app: &mut C, raw_fd: RawFd, revents: PollFlags);
}

pub trait StderrEventHandler<C> {
    fn handle(&mut self, app: &mut C, raw_fd: RawFd, revents: PollFlags);
}
pub trait PtyEventHandler<C> {
    fn handle(&mut self, app: &mut C, raw_fd: RawFd, revents: PollFlags);
}

pub trait PollInReadHandler<C> {
    fn handle(&mut self, app: &mut C, raw_fd: RawFd, revents: PollFlags);
}

pub trait PollOutHandler<C> {
    fn handle(&mut self, app: &mut C, raw_fd: RawFd, revents: PollFlags);
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

    pub fn stop_code(&self) -> i32 {
        self.context.shutdown.stop_code()
    }

    pub fn stop_message(&self) -> String {
        self.context.shutdown.stop_message()
    }

    pub fn poll(&mut self, timeout: i32) -> i32 {
        self.context.event_pocess(timeout)
    }

    pub fn is_stoped(&self) -> bool {
        self.context.shutdown.is_stoped()
    }
}

impl PollHandler<UnixContext> for DefaultPollMiddleware {
    fn handle(&mut self, res: i32) {
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
}

pub struct PollErrorMiddleware {}
impl PollErrorMiddleware {
    pub fn new() -> Self {
        Self { 
            
        }
    }
}

impl PollErrorHandler<UnixContext, nix::Error> for PollErrorMiddleware {
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

    pub fn reg_signalfd(&mut self, handler: Box<dyn SignalFdEventHandler<UnixContext>>) {
        self.signalfd = Some(handler);
    }

    pub fn reg_stdin(&mut self, handler: Box<dyn StdinEventHandler<UnixContext>>) {
        self.stdin = Some(handler);
    }

    pub fn reg_stdout(&mut self, handler: Box<dyn StdoutEventHandler<UnixContext>>) {
        self.stdout = Some(handler);
    }

    pub fn reg_stderr(&mut self, handler: Box<dyn StderrEventHandler<UnixContext>>) {
        self.stderr = Some(handler);
    }

    pub fn reg_pty(&mut self, handler: Box<dyn PtyEventHandler<UnixContext>>) {
        self.pty = Some(handler);
    }
}

impl PollReventHandler<UnixContext> for DefaultPollReventMiddleware {
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
                h.handle(app, raw_fd, revents);
            }
        }
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
                h.handle(app, raw_fd, revents);

                // let siginfo = self.map_to_siginfo(buf);
                // debug!("siginfo = {:#?}", siginfo);

                // let signal = Signal::try_from(siginfo.ssi_signo as i32).unwrap();
                // debug!("signal = {:#?}", signal);

                // if matches!(signal, Signal::SIGINT | Signal::SIGTERM) {
                //     app.shutdown.shutdown_starting(0, None);
                //     return;
                // }

                // if matches!(signal, Signal::SIGCHLD) {
                //     let res = self.waitpid(Pid::from_raw(siginfo.ssi_pid as i32));
                //     trace!("waitpid({}) = {:#?}", siginfo.ssi_pid, res);
                // }
                
            }
        }
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
                h.handle(app, raw_fd, revents);
            }
        }
    }
}

pub struct DefaultPollInReadHandler {
}

impl DefaultPollInReadHandler {
    pub fn new() -> Self {
        Self { 

        }
    }
}

impl PollInReadHandler<UnixContext> for DefaultPollInReadHandler {
    fn handle(&mut self, app: &mut UnixContext, raw_fd: RawFd, revents: PollFlags) {
        trace!("fd {} ready for reading", raw_fd);

        let buf = app.get_mut_buf(raw_fd);

        // Читаем данные и обрабатываем их
        match read(raw_fd, buf.get_mut_all_slice()) {
            Ok(n) => {
                // read n bytes
                trace!("read = Ok({n}) bytes");
                buf.set_len(n);
            }
            Err(Errno::EAGAIN) => {
                // дескриптор установлен в неблокирующий режим, но данных пока нет. Верно просто пропускать и ждать следующего срабатывания poll.
                trace!(
                    "non-blocking reading mode is enabled (SFD_NONBLOCK). fd {:?} doesn't data",
                    raw_fd,
                );
                // continue;
            }
            Err(Errno::EBADF) => {
                // Аргумент fd не является допустимым дескриптором файла, открытым для чтения.
                // Это может значить, что он был закрыт или никогда не существовал.
                // Удалить его из списка наблюдаемых дескрипторов.
            }
            Err(Errno::EINTR) => {
                // Операция чтения была прервана из-за получения сигнала, и данные не были переданы.
                // Здесь можно просто повторить read
            }
            Err(Errno::EINVAL) => {
                // Файл является обычным или блочным специальным файлом, а аргумент смещение отрицательный. Смещение файла должно оставаться неизменным.
                // если возникает, стоит логировать, так как это признак ошибки в коде (например, передан неверный аргумент offset).
            }
            Err(Errno::ECONNRESET) => {
                // Была предпринята попытка чтения из сокета, и соединение было принудительно закрыто его партнёром.
                // соединение было закрыто принудительно, нужно закрыть дескриптор и удалить его из списка.
            }
            Err(Errno::ENOTCONN) => {
                // Была предпринята попытка чтения из сокета, который не подключен.
                // сокет не подключен, тоже стоит удалить fd.
            }
            Err(Errno::ETIMEDOUT) => {
                // Была предпринята попытка чтения из сокета, и произошел тайм-аут передачи.
                // тайм-аут соединения. Если это TCP-сокет, вероятно, соединение закрылось → удалить fd.
            }
            Err(Errno::EIO) => {
                // Произошла физическая ошибка ввода-вывода.
                // Это может быть связано с проблемами на уровне железа, стоит логировать и удалить fd.
            }
            Err(Errno::ENOBUFS) => {
                // В системе было недостаточно ресурсов для выполнения этой операции.
                // нехватка ресурсов. Можно попробовать повторить позже, но если ошибка повторяется, логировать и, возможно, завершить работу (в зависимости от критичности).
            }
            Err(Errno::ENOMEM) => {
                // Для выполнения запроса недостаточно памяти
                // нехватка ресурсов. Можно попробовать повторить позже, но если ошибка повторяется, логировать и, возможно, завершить работу (в зависимости от критичности).
            }
            Err(Errno::ENXIO) => {
                // Был отправлен запрос несуществующему устройству или запрос выходил за рамки возможностей устройства.
                // устройство не существует или запрос вне его диапазона. Вероятно, fd устарел, его следует удалить.
            }
            Err(e) => {
                error!("read = Err({})", e);
            }
        }
    }
}

pub struct DefaultPollOutHandler {
}

impl DefaultPollOutHandler {
    pub fn new() -> Self {
        Self { 

        }
    }
}

impl PollOutHandler<UnixContext> for DefaultPollOutHandler {
    fn handle(&mut self, app: &mut UnixContext, raw_fd: RawFd, revents: PollFlags) {
        trace!("fd {} ready for writing", raw_fd);
    }
}

pub struct DefaultPollErrHandler {
}

impl DefaultPollErrHandler {
    pub fn new() -> Self {
        Self { }
    }
}

impl PollErrHandler<UnixContext> for DefaultPollErrHandler {
    fn handle(&mut self, app: &mut UnixContext, raw_fd: RawFd, revents: PollFlags) {
        trace!("fd {}: POLLERR (I/O error)", raw_fd);
    }
}

pub struct DefaultPollNvalHandler {

}

impl DefaultPollNvalHandler {
    pub fn new() -> Self {
        Self { }
    }
}

impl PollNvalHandler<UnixContext> for DefaultPollNvalHandler {
    fn handle(&mut self, app: &mut UnixContext, raw_fd: RawFd, revents: PollFlags) {
        trace!("fd {}: POLLNVAL (invalid descriptor)", raw_fd);
    }
}

pub struct DefaultPollHupHandler {
}

impl DefaultPollHupHandler {
    pub fn new() -> Self {
        Self { }
    }
}

impl PollHupHandler<UnixContext> for DefaultPollHupHandler {
    fn handle(&mut self, app: &mut UnixContext, raw_fd: RawFd, revents: PollFlags) {
        trace!("fd {}: POLLHUP (peer closed connection)", raw_fd);
    }
}
