use nix::poll::PollFlags;
use std::os::fd::RawFd;
use std::os::raw::c_int;
use std::os::unix::io::AsRawFd;
use std::sync::Arc;
use nix::fcntl;
use nix::sys::wait::{waitpid, WaitPidFlag, WaitStatus};
use nix::unistd::Pid;
use nix::sys::signal::{SigSet, Signal};
use nix::sys::signalfd::{siginfo, SfdFlags, SignalFd};

use thiserror::Error;

use abstractions::{PluginRust, error, info, trace, warn};
use common::read_fd::{read_fd, ReadResult};
use abstractions::buffer::{Buffer, BufferError};
use common::UnixContext;

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
#[derive(Debug)]
pub struct SignalFdPlugin {
    fd: SignalFd,
    buf: Buffer,
    expected_size: usize,         // Ожидаемый размер структуры siginfo
    error_count: usize,    // Счетчик ошибок для отслеживания повторяющихся проблем (защищен мьютексом)
    max_errors: usize,            // Максимальное количество ошибок до завершения плагина
    recovery_attempts: usize, // Счетчик попыток восстановления (защищен мьютексом)
    max_recovery_attempts: usize, // Максимальное количество попыток восстановления
    ctx: Arc<UnixContext>,
}

impl SignalFdPlugin {
    pub fn new(ctx: Arc<UnixContext>) -> Result<Self, String> {
        info!(ctx, "signal: plugin initializing");

        let (fd, buf, expected_size) = match Self::get_signal_fd(&ctx) {
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
            ctx,
        })
    }

    // Метод для восстановления файлового дескриптора
    fn recover_fd(&mut self) -> Result<(), String> {
        // Сначала удаляем старый файловый дескриптор из poll
        if !self.ctx.poll.remove_fd(self.fd.as_raw_fd()) {
            warn!(self.ctx, "Failed to remove old signal fd {} from poll, it may have already been removed",
                self.fd.as_raw_fd()
            );
        }

        // Сбрасываем буфер
        self.buf.clear();

        // Создаем новый файловый дескриптор
        let (new_fd, new_buf, expected_size) = Self::get_signal_fd(&self.ctx)?;

        // Обновляем поля в структуре
        // Примечание: в потокобезопасной версии мы должны использовать мьютексы
        // для защиты этих полей, но для простоты оставим как есть
        self.fd = new_fd;
        self.buf = new_buf;
        self.expected_size = expected_size;

        info!(self.ctx, "Signal fd recreated successfully with fd {}",
            self.fd.as_raw_fd()
        );

        Ok(())
    }

    // Обработка событий с детальной обработкой ошибок
    fn handle_events(&mut self) -> Result<(), PluginError> {
        // Проверяем, есть ли события на нашем файловом дескрипторе
        let mut should_process_signal = false;

        // Проверяем, есть ли наш файловый дескриптор в poll
        if !self.ctx.poll.has_fd(self.fd.as_raw_fd()) {
            return Err(PluginError::RecoverableFdError(format!(
                "Signal fd {} not found in poll",
                self.fd.as_raw_fd()
            )));
        }

        // Получаем revents для нашего fd
        if let Some(revents) = self.ctx.poll.get_revents(self.fd.as_raw_fd()) {
            if revents > 0 {
                if PollFlags::from_bits(revents).is_none() {
                    return Err(PluginError::ReadError(format!(
                        "Unknown revents: {} on signal fd {}",
                        revents,
                        self.fd.as_raw_fd()
                    )));
                }

                let revents_flags = PollFlags::from_bits(revents).unwrap();
                // Обрабатываем ошибки файлового дескриптора
                if revents_flags.contains(PollFlags::POLLERR) {
                    // POLLERR может указывать на различные проблемы с файловым дескриптором
                    // Для signalfd это может быть временная проблема, попробуем восстановиться
                    return Err(PluginError::RecoverableFdError(format!(
                        "POLLERR on signal fd {}",
                        self.fd.as_raw_fd()
                    )));
                }
                if revents_flags.contains(PollFlags::POLLNVAL) {
                    // POLLNVAL означает, что файловый дескриптор недействителен
                    // Нужно пересоздать его
                    return Err(PluginError::RecoverableFdError(format!(
                        "POLLNVAL on signal fd {}",
                        self.fd.as_raw_fd()
                    )));
                }
                if revents_flags.contains(PollFlags::POLLHUP) {
                    // POLLHUP означает, что соединение закрыто
                    // Для signalfd это странно, но попробуем восстановиться
                    return Err(PluginError::RecoverableFdError(format!(
                        "POLLHUP on signal fd {}",
                        self.fd.as_raw_fd()
                    )));
                }

                // Обрабатываем данные, если они доступны
                if revents_flags.contains(PollFlags::POLLIN) {
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
            
            // Сбрасываем revents после обработки
            self.ctx.poll.reset_revents(self.fd.as_raw_fd());
        }

        // Обрабатываем сигнал после того, как закончили работу с fd
        if should_process_signal {
            let res = self.process_signal();
            self.buf.clear();
            if let Err(_e) = res {
                return Err(_e)
            }
        }

        Ok(())
    }

    /// Обрабатывает полученный сигнал
    fn process_signal(&mut self) -> Result<(), PluginError> {
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
                self.ctx.shutdown.shutdown_smart();
                self.ctx.shutdown.set_code(0);
                self.ctx.shutdown.set_message(
                    format!("{signal} from pid: {pid} (uid: {uid})")
                );
            }
            Signal::SIGINT => {
                // info!("Received SIGINT, initiating fast shutdown");
                self.ctx.shutdown.shutdown_fast();
                self.ctx.shutdown.set_code(0);
                self.ctx.shutdown.set_message(
                    format!("{signal} from pid: {pid} (uid: {uid})")
                );
            }
            Signal::SIGQUIT => {
                // info!("Received SIGQUIT, initiating immediate shutdown");
                self.ctx.shutdown.shutdown_immediate();
                self.ctx.shutdown.set_code(0);
                self.ctx.shutdown.set_message(
                    format!("{signal} from pid: {pid} (uid: {uid})")
                );
            }
            Signal::SIGCHLD => {
                // trace!("Received SIGCHLD: status: {ssi_status} (ssi_utime: {ssi_utime}, ssi_stime: {ssi_stime})");

                match self.waitpid(Pid::from_raw(pid as i32)) {
                    Ok(status) => {
                        trace!(self.ctx, "waitpid({}) = {:#?}", pid, status)
                    },
                    Err(e) => {
                        warn!(self.ctx, "waitpid({}) failed: {:#?}", pid, e)
                    },
                }
            }
            Signal::SIGHUP => {
                // Получен SIGHUP, обычно используется для перезагрузки конфигурации
                info!(self.ctx, "Received SIGHUP, triggering configuration reload");
                self.ctx.reload_config.set_reload_needed();
            }
            Signal::SIGUSR1 => {
                // Пользовательский сигнал 1, можно использовать для специфических действий
                info!(self.ctx, "Received SIGUSR1 from pid: {} (uid: {})", pid, uid);
                // Здесь можно добавить специфическую обработку
            }
            Signal::SIGUSR2 => {
                // Пользовательский сигнал 2, можно использовать для специфических действий
                info!(self.ctx, "Received SIGUSR2 from pid: {} (uid: {})", pid, uid);
                // Здесь можно добавить специфическую обработку
            }
            _ => {
                // Обработка других сигналов
                info!(self.ctx, "Received signal {:?} from pid: {} (uid: {})", signal, pid, uid);
            }
        }

        Ok(())
    }

    // Вспомогательный метод для ожидания завершения дочернего процесса
    fn waitpid(&self, pid: Pid) -> Result<WaitStatus, nix::Error> {
        waitpid(pid, Some(WaitPidFlag::WNOHANG))
    }

    // Создает файловый дескриптор для сигналов
    fn get_signal_fd(ctx: &UnixContext) -> Result<(SignalFd, Buffer, usize), String> {
        let mut mask = SigSet::empty();

        // Добавляем в обработчик все сигналы, кроме SIGKILL и SIGSTOP
        for signal in Signal::iterator() {
            if matches!(signal, Signal::SIGKILL | Signal::SIGSTOP) {
                continue;
            }

            mask.add(signal);
        }

        // Блокируем сигналы, чтобы они не обрабатывались стандартным обработчиком
        let mut new_mask = match SigSet::thread_get_mask() {
            Ok(mask) => mask,
            Err(e) => return Err(format!("Failed to get thread mask: {}", e)),
        };

        for s in mask.into_iter() {
            new_mask.add(s);
        }

        if let Err(e) = new_mask.thread_block() {
            return Err(format!("Failed to set thread mask: {}", e));
        }

        // Создаем файловый дескриптор для сигналов
        let fd = match SignalFd::with_flags(&new_mask, SfdFlags::SFD_NONBLOCK | SfdFlags::SFD_CLOEXEC) {
            Ok(fd) => fd,
            Err(e) => return Err(format!("Failed to create signal fd: {}", e)),
        };

        // Проверяем, что файловый дескриптор действителен
        if !Self::is_valid_fd(fd.as_raw_fd()) {
            return Err(format!("Created signal fd {} is invalid", fd.as_raw_fd()));
        }

        // Создаем буфер для чтения сигналов
        let buffer_length = std::mem::size_of::<siginfo>();
        let buf = Buffer::new(buffer_length);

        // Регистрируем файловый дескриптор в poll
        let flags = PollFlags::POLLIN | PollFlags::POLLERR | PollFlags::POLLHUP | PollFlags::POLLNVAL;
        ctx.poll.add_fd(fd.as_raw_fd(), flags.bits());

        Ok((fd, buf, buffer_length))
    }

    // Проверяет, является ли файловый дескриптор действительным
    fn is_valid_fd(fd: RawFd) -> bool {
        let mut res = fcntl::fcntl(fd, fcntl::F_GETFD);

        // Запрашиваем до тех пор, пока приходит EINTR
        // так как это означает что вызов fcntl был прерван сигналом и надо повторить попытку
        while let Err(nix::Error::EINTR) = res {
            res = fcntl::fcntl(fd, fcntl::F_GETFD);
        }

        res.is_ok()
    }
}

impl Drop for SignalFdPlugin {
    fn drop(&mut self) {
        info!(self.ctx, "signal: plugin cleaning up");
        
        // Удаляем файловый дескриптор из poll
        if !self.ctx.poll.remove_fd(self.fd.as_raw_fd()) {
            warn!(self.ctx, "Failed to remove signal fd {} from poll during cleanup", 
                self.fd.as_raw_fd()
            );
        }
        
        // Разблокируем сигналы, если это необходимо
        // Примечание: в большинстве случаев это не нужно делать,
        // так как при завершении процесса все ресурсы освобождаются автоматически
    }
}

impl PluginRust<UnixContext> for SignalFdPlugin {
    fn handle(&mut self) -> c_int {
        // Проверяем, нужно ли завершить работу
        if self.ctx.shutdown.is_stoping() {
            return 1; // Сигнализируем о завершении плагина
        }
        
        // Если нет событий poll, ничего не делаем
        if self.ctx.poll.get_result() <= 0 {
            return 0;
        }
        
        // Обрабатываем события и возвращаем результат
        match self.handle_events() {
            Ok(_) => {
                // Успешная обработка, сбрасываем счетчик ошибок
                self.error_count = 0;
                
                // Сбрасываем счетчик попыток восстановления при успехе
                if self.recovery_attempts > 0 {
                    self.recovery_attempts = 0;
                }
                
                0 // Успешное выполнение
            },
            Err(err) => {
                // Увеличиваем счетчик ошибок
                self.error_count += 1;
                
                // Если это восстанавливаемая ошибка, пытаемся восстановить fd
                if let PluginError::RecoverableFdError(msg) = &err {
                    error!(self.ctx, "Recoverable error in signal plugin: {}", msg);
                    
                    self.recovery_attempts += 1;
                    
                    if self.recovery_attempts <= self.max_recovery_attempts {
                        // Пытаемся восстановить fd
                        match self.recover_fd() {
                            Ok(_) => {
                                info!(self.ctx, "Successfully recovered signal fd after {} attempts", 
                                    self.recovery_attempts
                                );
                                return 0; // Продолжаем работу
                            },
                            Err(e) => {
                                error!(self.ctx, "Failed to recover signal fd: {}", e);
                                // Продолжаем обработку ошибки
                            }
                        }
                    } else {
                        error!(self.ctx, "Too many recovery attempts ({}), giving up", 
                            self.recovery_attempts
                        );
                    }
                }
                
                // Если превышен лимит ошибок, возвращаем код ошибки
                if self.error_count > self.max_errors {
                    error!(self.ctx, "Too many errors ({}) in signal plugin, shutting down", 
                        self.error_count
                    );
                    return 1; // Код ошибки, который приведет к удалению плагина
                }
                
                // Логируем ошибку
                error!(self.ctx, "Error in signal plugin: {}", err);
                
                // Возвращаем код в зависимости от типа ошибки
                err.to_return_code()
            }
        }
    }
}

#[no_mangle]
pub extern "Rust" fn register_rust_plugin(ctx: Arc<UnixContext>) -> Result<Box<dyn PluginRust<UnixContext>>, String> {
    let plugin = SignalFdPlugin::new(ctx)?;
    let plugin = Box::new(plugin);
    
    Ok(plugin)
}
