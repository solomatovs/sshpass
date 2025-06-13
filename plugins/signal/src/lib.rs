// use log::{error, info, trace, warn};
use nix::poll::PollFlags;
use std::os::fd::RawFd;
use std::os::raw::c_int;
use std::os::unix::io::AsRawFd;
use nix::fcntl;
use nix::sys::wait::{waitpid, WaitPidFlag, WaitStatus};
use nix::unistd::Pid;
use nix::sys::signal::{SigSet, Signal};
use nix::sys::signalfd::{siginfo, SfdFlags, SignalFd};

use thiserror::Error;

use abstractions::{PluginRegistrator, PluginRust, ShutdownType, UnixContext, error, info, trace, warn};
// use common::init_log::init_log;
use common::read_fd::{read_fd, ReadResult};
use abstractions::buffer::{Buffer, BufferError};

// Определяем типы ошибок, которые могут возникнуть в плагине
#[derive(Debug, Error)]
enum PluginError {
    // Ошибки файлового дескриптора, которые можно исправить пересозданием
    #[error("Recoverable fd error: {0}")]
    RecoverableFdError(String),
    // Ошибки чтения
    #[error("Read error: {0}")]
    ReadError(String),
    // Ошибки обработки данных
    #[error("Processing error: {0}")]
    ProcessingError(String),
    // Критические ошибки, требующие завершения работы плагина
    #[error("Fatal error: {0}")]
    Fatal(String),
}

impl PluginError {
    // Преобразует ошибку в код возврата для функции handle
    fn to_return_code(&self) -> c_int {
        match self {
            // Для критических ошибок возвращаем 1, что приведет к удалению плагина
            PluginError::Fatal(_) => 1,
            // Для остальных ошибок возвращаем 0, чтобы продолжить работу
            _ => 0,
        }
    }
}

// Определяем структуру для нашего плагина в Rust-стиле
#[repr(C)]
pub struct SignalFdPlugin {
    fd: SignalFd,
    buf: Buffer,
    expected_size: usize,         // Ожидаемый размер структуры siginfo
    error_count: usize,           // Счетчик ошибок для отслеживания повторяющихся проблем
    max_errors: usize,            // Максимальное количество ошибок до завершения плагина
    recovery_attempts: usize,     // Счетчик попыток восстановления
    max_recovery_attempts: usize, // Максимальное количество попыток восстановления
}

impl SignalFdPlugin {
    pub fn new(ctx: &mut UnixContext) -> Result<Self, String> {
        info!(ctx, "signal: plugin initializing");

        let (fd, buf, expected_size) = match Self::get_signal_fd(ctx) {
            Ok(x) => x,
            Err(e) => {
                return Err(format!("Error getting signal fd: {}", e));
            }
        };

        Ok(SignalFdPlugin {
            fd,
            buf,
            expected_size,
            error_count: 0,
            max_errors: 5, // Максимальное количество последовательных ошибок
            recovery_attempts: 0,
            max_recovery_attempts: 3, // Максимальное количество попыток восстановления
        })
    }

    // Метод для восстановления файлового дескриптора
    fn recover_fd(&mut self, ctx: &mut UnixContext) -> Result<(), String> {
        // Сначала удаляем старый файловый дескриптор из poll
        if !ctx.poll.remove_fd(self.fd.as_raw_fd()) {
            warn!(ctx, "Failed to remove old signal fd {} from poll, it may have already been removed",
                self.fd.as_raw_fd()
            );
        }

        // Сбрасываем буфер
        self.buf.clear();

        // Создаем новый файловый дескриптор
        let (new_fd, new_buf, expected_size) = Self::get_signal_fd(ctx)?;

        // Обновляем поля в структуре
        self.fd = new_fd;
        self.buf = new_buf;
        self.expected_size = expected_size;

        info!(ctx, "Signal fd recreated successfully with fd {}",
            self.fd.as_raw_fd()
        );

        Ok(())
    }

    // Обработка событий с детальной обработкой ошибок
    fn handle_events(&mut self, ctx: &mut UnixContext) -> Result<(), PluginError> {
        // Проверяем, есть ли события на нашем файловом дескрипторе
        let mut should_process_signal = false;

        if let Some(fd) = ctx.poll.get_fd_mut(self.fd.as_raw_fd()) {
            if fd.revents > 0 {
                if PollFlags::from_bits(fd.revents).is_none() {
                    return Err(PluginError::ReadError(format!(
                        "Unknown revents: {} on signal fd {}",
                        fd.revents,
                        self.fd.as_raw_fd()
                    )));
                }

                let revents = PollFlags::from_bits(fd.revents).unwrap();
                // Обрабатываем ошибки файлового дескриптора
                if revents.contains(PollFlags::POLLERR) {
                    // POLLERR может указывать на различные проблемы с файловым дескриптором
                    // Для signalfd это может быть временная проблема, попробуем восстановиться
                    return Err(PluginError::RecoverableFdError(format!(
                        "POLLERR on signal fd {}",
                        self.fd.as_raw_fd()
                    )));
                }
                if revents.contains(PollFlags::POLLNVAL) {
                    // POLLNVAL означает, что файловый дескриптор недействителен
                    // Нужно пересоздать его
                    return Err(PluginError::RecoverableFdError(format!(
                        "POLLNVAL on signal fd {}",
                        self.fd.as_raw_fd()
                    )));
                }
                if revents.contains(PollFlags::POLLHUP) {
                    // POLLHUP означает, что соединение закрыто
                    // Для signalfd это странно, но попробуем восстановиться
                    return Err(PluginError::RecoverableFdError(format!(
                        "POLLHUP on signal fd {}",
                        self.fd.as_raw_fd()
                    )));
                }

                // Обрабатываем данные, если они доступны
                if revents.contains(PollFlags::POLLIN) {
                    // Пытаемся прочитать данные
                    match read_fd(self.fd.as_raw_fd(), &mut self.buf) {
                        // all read
                        ReadResult::Success(n) if n == self.expected_size => {
                            should_process_signal = true;
                        }
                        // partial read
                        ReadResult::Success(n) => {
                            // неожиданное кол-во байт прочитано
                            // попробую выполнить recover signal fd
                            let error = format!(
                                "Unexpected number of bytes read: {} for signalfd: {}. the {} byte is expected to arrive",
                                n,
                                self.fd.as_raw_fd(),
                                self.expected_size,
                            );
                            return Err(PluginError::RecoverableFdError(error));
                        }
                        // buffer is full
                        ReadResult::BufferIsFull { fd: _, data_len } => {
                            // буфер почему то меньше чем нужно для чтения signal_fd
                            if data_len < self.expected_size {
                                // увеличиваем буфер
                                self.buf.resize(self.expected_size - data_len);
                            } else {
                                // если буфер уже заполнен и при этом прочитанные данные больше чем ожидамо, то надо обнулить буфер, что бы прочитать signal_fd
                                self.buf.clear();
                            }
                        }
                        ReadResult::WouldBlock { fd: _ } => {
                            // файловый дескриптор заблокирован, прочитаем данные в следующий раз
                            // poll вернет событие повторно, можно не переживать
                            // trace!("Would block on signal fd {}", self.fd.as_raw_fd());
                        }
                        ReadResult::Interrupted { fd: _ } => {
                            // чтение было прервано в процессе из-за получения сигнала, прочитаем данные в следующий раз
                            // poll вернет событие повторно, можно не переживать
                            // trace!("Read interrupted on signal fd {}", self.fd.as_raw_fd());
                        }
                        ReadResult::InvalidFd { fd: _ } => {
                            return Err(PluginError::RecoverableFdError(format!(
                                "Invalid signal fd {}",
                                self.fd.as_raw_fd()
                            )));
                        }
                        ReadResult::Eof { fd: _ } => {
                            return Err(PluginError::Fatal(format!(
                                "signal fd eof {}",
                                self.fd.as_raw_fd(),
                            )));
                        }
                        ReadResult::Fatal { fd: _, msg } => {
                            return Err(PluginError::ReadError(format!(
                                "signal fd fatal {}: {}",
                                self.fd.as_raw_fd(),
                                msg
                            )));
                        }
                    }
                }
            }
            fd.revents = 0;
        } else {
            // Файловый дескриптор не найден в poll
            return Err(PluginError::RecoverableFdError(format!(
                "Signal fd {} not found in poll",
                self.fd.as_raw_fd()
            )));
        }

        // Обрабатываем сигнал после того, как закончили работу с fd
        if should_process_signal {
            let res = self.process_signal(ctx);
            self.buf.clear();
            if let Err(_e) = res {
                return Err(_e)
            }
        }

        Ok(())
    }

    /// Обрабатывает полученный сигнал
    fn process_signal(&mut self, ctx: &mut UnixContext) -> Result<(), PluginError> {
        // Преобразуем буфер в структуру siginfo
        let (signal, siginfo) = {
            let siginfo = match self.buf.try_read_struct::<siginfo>() {
                Err(BufferError::AlignError { required, type_name }) => {
                    self.buf.clear();
                    return Err(PluginError::ProcessingError(format!(
                        "Failed to read siginfo from buffer: align error: required: {required}, type_name: {type_name}"
                    )));
                }
                Err(BufferError::DataLenIsLessReadableType { required, available, type_name }) => {
                    self.buf.clear();
                    return Err(PluginError::ProcessingError(format!(
                        "Failed to read siginfo from buffer: data len is less than required: required: {required}, available: {available}, type_name: {type_name}"
                    )));
                }
                Ok(s) => s,
            };

            match Signal::try_from(siginfo.ssi_signo as i32) {
                Ok(s) => (s, siginfo),
                Err(_) => {
                    return Err(PluginError::ProcessingError(format!(
                        "Failed to convert signal number to enum: {}",
                        siginfo.ssi_signo
                    )));
                }
            }
        };

        let pid = siginfo.ssi_pid;
        let uid = siginfo.ssi_uid;
        // Обработка различных сигналов
        match signal {
            Signal::SIGTERM => {
                // info!("Received SIGTERM, initiating smart shutdown");
                ctx.shutdown.shutdown_smart();
                ctx.shutdown.set_code(0);
                ctx.shutdown.set_message(
                    format!("{signal} from pid: {pid} (uid: {uid})")
                );
            }
            Signal::SIGINT => {
                // info!("Received SIGINT, initiating fast shutdown");
                ctx.shutdown.shutdown_fast();
                ctx.shutdown.set_code(0);
                ctx.shutdown.set_message(
                    format!("{signal} from pid: {pid} (uid: {uid})")
                );
            }
            Signal::SIGQUIT => {
                // info!("Received SIGQUIT, initiating immediate shutdown");
                ctx.shutdown.shutdown_immediate();
                ctx.shutdown.set_code(0);
                ctx.shutdown.set_message(
                    format!("{signal} from pid: {pid} (uid: {uid})")
                );
            }
            Signal::SIGCHLD => {
                // trace!("Received SIGCHLD: status: {ssi_status} (ssi_utime: {ssi_utime}, ssi_stime: {ssi_stime})");

                match self.waitpid(Pid::from_raw(pid as i32)) {
                    Ok(status) => {
                        trace!(ctx, "waitpid({}) = {:#?}", pid, status)
                    },
                    Err(e) => {
                        warn!(ctx, "waitpid({}) failed: {:#?}", pid, e)
                    },
                }
            }
            _ => {
                trace!(ctx, "Received signal {signal} (no special handling)");
            }
        }

        Ok(())
    }

    /// Ожидает завершения дочернего процесса
    fn waitpid(&self, pid: Pid) -> Result<WaitStatus, nix::Error> {
        waitpid(pid, Some(WaitPidFlag::WNOHANG))
    }

    fn _is_valid_fd(&self, fd: RawFd) -> bool {
        let mut res = fcntl::fcntl(fd, fcntl::F_GETFD);

        // запрашиваю до тех пор, пока приходит EINTR
        // так как это означает что вызов fcntl был прерван сигналом и надо повторить попытку
        while let Err(nix::Error::EINTR) = res {
            res = fcntl::fcntl(fd, fcntl::F_GETFD);
        }

        if res.is_ok() {
            return true;
        }

        false
    }

    fn get_signal_fd(ctx: &mut UnixContext) -> Result<(SignalFd, Buffer, usize), String> {
        // Определяем размер структуры siginfo
        let expected_size = std::mem::size_of::<siginfo>();

        // Создаем буфер для структуры siginfo
        let buf = Buffer::new(expected_size);

        let mut mask = SigSet::empty();

        // добавляю в обработчик все сигналы
        for signal in Signal::iterator() {
            if matches!(signal, Signal::SIGKILL | Signal::SIGSTOP) {
                continue;
            }

            mask.add(signal);
        }

        let mut new_mask =
            SigSet::thread_get_mask().map_err(|e| format!("failed get thread mask: {:#?}", e))?;
        for s in mask.into_iter() {
            new_mask.add(s);
        }

        new_mask
            .thread_block()
            .map_err(|e| format!("failed set thread mask: {:#?}", e))?;

        let fd: SignalFd =
            SignalFd::with_flags(&new_mask, SfdFlags::SFD_NONBLOCK | SfdFlags::SFD_CLOEXEC)
                .map_err(|e| format!("signalfd create failed error: {:#?}", e))?;

        let flags =
            PollFlags::POLLIN | PollFlags::POLLERR | PollFlags::POLLHUP | PollFlags::POLLNVAL;

        ctx.poll.add_fd(fd.as_raw_fd(), flags.bits());

        Ok((fd, buf, expected_size))
    }
}

impl PluginRust<UnixContext> for SignalFdPlugin {
    fn free(&mut self, ctx: &mut UnixContext) -> c_int {
        info!(ctx, "signal: plugin cleaning up");

        if !ctx.poll.remove_fd(self.fd.as_raw_fd()) {
            error!(ctx, 
                "Failed to remove signal fd {} from poll",
                self.fd.as_raw_fd()
            );
            return 1;
        }

        0 // 0 означает успешное освобождение
    }

    fn handle(&mut self, ctx: &mut UnixContext) -> c_int {
        if ctx.shutdown.is_stoping() {
            return ShutdownType::Stoped.to_int();
        }

        if ctx.poll.get_result() == 0 {
            return 0;
        }

        // Обрабатываем события и возвращаем результат
        match self.handle_events(ctx) {
            Ok(_) => {
                // Успешная обработка, сбрасываем счетчик ошибок
                self.error_count = 0;
                0 // Успешное выполнение
            }
            Err(err) => {
                // Проверяем, можно ли восстановить работу после ошибки
                match &err {
                    PluginError::RecoverableFdError(msg) => {
                        // Пытаемся восстановить файловый дескриптор
                        if self.recovery_attempts < self.max_recovery_attempts {
                            warn!(ctx, "Attempting to recover from error: {}", msg);
                            if let Err(recovery_err) = self.recover_fd(ctx) {
                                error!(ctx, "Failed to recover: {}", recovery_err);
                                self.error_count += 1;
                            } else {
                                info!(ctx, "Successfully recovered signal fd");
                                self.recovery_attempts += 1;
                                self.error_count = 0; // Сбрасываем счетчик ошибок после успешного восстановления
                            }
                            0 // Продолжаем работу после попытки восстановления
                        } else {
                            error!(ctx, "Too many recovery attempts ({}), giving up",
                                self.recovery_attempts
                            );
                            1 // Слишком много попыток восстановления, завершаем плагин
                        }
                    }
                    _ => {
                        // Увеличиваем счетчик ошибок
                        self.error_count += 1;

                        // Если превышен лимит ошибок, возвращаем код ошибки
                        if self.error_count > self.max_errors {
                            error!(ctx, "Too many errors ({}) in signal plugin, shutting down",
                                self.error_count
                            );
                            return 1; // Код ошибки, который приведет к удалению плагина
                        }

                        // Возвращаем код в зависимости от типа ошибки
                        err.to_return_code()
                    }
                }
            }
        }
    }
}

/// Creates a new instance of SignalFdPlugin.
///
/// # Safety
///
/// The caller must ensure that:
/// - `ctx` is a valid, non-null pointer to a properly initialized `UnixContext`
/// - The `UnixContext` pointed to by `ctx` remains valid for the duration of the call
/// - The `UnixContext` is not being mutably accessed from other parts of the code during this call
#[no_mangle]
pub extern "Rust" fn register_rust_plugin(
    registrator: &mut PluginRegistrator<UnixContext>,
) -> Result<(), String> {
    let plugin = SignalFdPlugin::new(registrator.get_context())?;
    let plugin = Box::new(plugin);

    registrator.add_plugin(plugin);

    Ok(())
}
