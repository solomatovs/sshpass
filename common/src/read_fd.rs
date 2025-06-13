use nix::errno::Errno;
use nix::libc;
use std::os::fd::RawFd;
use thiserror::Error;

use abstractions::buffer::Buffer;

/// Результат или ошибка чтения из файлового дескриптора
#[derive(Error, Debug)]
pub enum ReadResult {
    /// Успешное чтение данных
    ///
    /// Хранит количество прочитанных байт.
    #[error("Successfully read {0} bytes")]
    Success(usize),

    /// Достигнут конец потока (EOF)
    ///
    /// Возвращается, когда `read()` возвращает 0.
    #[error("EOF reached on file descriptor {fd}")]
    Eof {
        /// Файловый дескриптор
        fd: RawFd,
    },

    /// Временная блокировка чтения (например, EAGAIN, EWOULDBLOCK)
    ///
    /// Можно повторить попытку позже.
    #[error("Read would block on file descriptor {fd} (EAGAIN or EWOULDBLOCK)")]
    WouldBlock { fd: RawFd },

    /// Операция была прервана сигналом (EINTR)
    ///
    /// Рекомендуется повторить попытку.
    #[error("Read interrupted by signal on file descriptor {fd} (EINTR)")]
    Interrupted { fd: RawFd },

    /// Дескриптор недействителен или соединение закрыто
    ///
    /// Может означать EBADF, ENOTCONN, ECONNRESET и подобные ошибки.
    #[error("Invalid or closed file descriptor {fd}")]
    InvalidFd { fd: RawFd },

    /// Критическая ошибка чтения
    ///
    /// Требует немедленного внимания (например, ENOMEM, EIO и др.)
    #[error("Fatal read error on file descriptor {fd}: {msg}")]
    Fatal { fd: RawFd, msg: String },

    /// Буфер не имеет свободного места для записи
    #[error("buffer for fd {fd} is full ({data_len} bytes)")]
    BufferIsFull { fd: RawFd, data_len: usize },
}

/// Читает данные из файлового дескриптора в буфер
///
/// # Аргументы
/// * `fd` - Файловый дескриптор для чтения
/// * `buffer` - Буфер для записи данных
///
/// # Возвращает
/// Результат операции чтения в виде `ReadResult`
pub fn read_fd(fd: RawFd, buffer: &mut Buffer) -> ReadResult {
    // Получаем срез свободного места для записи в буфере
    let dst = buffer.as_mut_free_slice();

    if dst.is_empty() {
        return ReadResult::BufferIsFull {
            fd,
            data_len: buffer.get_data_len(),
        };
    }

    // Выполняем системный вызов read (одна попытка)
    let res = unsafe { libc::read(fd, dst.as_mut_ptr() as *mut libc::c_void, dst.len()) };

    let res = Errno::result(res).map(|r| r as usize);

    match res {
        Ok(0) => {
            // EOF — конец файла или соединение закрыто
            ReadResult::InvalidFd { fd }
        }
        Ok(n) => {
            // Успешно прочитано n байт
            if let Err(e) = buffer.grow_data_len(n) {
                return ReadResult::Fatal { fd, msg: e };
            }

            ReadResult::Success(n)
        }
        Err(e) if e == Errno::EAGAIN || e == Errno::EWOULDBLOCK => {
            // Неблокирующий режим — данные пока недоступны
            ReadResult::WouldBlock { fd }
        }
        Err(Errno::EINTR) => {
            // Операция прервана сигналом
            ReadResult::Interrupted { fd }
        }
        Err(e)
            if e == Errno::EBADF
                || e == Errno::ENOTCONN
                || e == Errno::ECONNRESET
                || e == Errno::ETIMEDOUT
                || e == Errno::ENXIO =>
        {
            ReadResult::InvalidFd { fd }
        }
        Err(e) => {
            // Все остальные ошибки считаются критическими
            ReadResult::Fatal {
                fd,
                msg: format!("read error: {}", e),
            }
        }
    }
}
