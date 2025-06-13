use nix::errno::Errno;
use nix::libc;
use std::os::fd::RawFd;
use thiserror::Error;

use abstractions::buffer::Buffer;

/// Результат или ошибка записи в файловый дескриптор
#[derive(Error, Debug)]
pub enum WriteResult {
    /// Успешная запись данных
    ///
    /// Хранит количество записанных байт.
    #[error("Successfully wrote {0} bytes")]
    Success(usize),

    /// Достигнут EOF или соединение закрыто
    ///
    /// Обычно означает, что запись невозможна (write() вернул 0).
    #[error("EOF or closed connection on file descriptor {fd}")]
    Eof {
        fd: RawFd,
    },

    /// Временная блокировка записи (например, EAGAIN, EWOULDBLOCK)
    ///
    /// Можно повторить попытку позже.
    #[error("Write would block on file descriptor {fd} (EAGAIN or EWOULDBLOCK)")]
    WouldBlock {
        fd: RawFd,
    },

    /// Операция была прервана сигналом (EINTR)
    ///
    /// Рекомендуется повторить попытку.
    #[error("Write interrupted by signal on file descriptor {fd} (EINTR)")]
    Interrupted {
        fd: RawFd,
    },

    /// Дескриптор недействителен или соединение закрыто
    ///
    /// Может означать EBADF, ENOTCONN, ECONNRESET и подобные ошибки.
    #[error("Invalid or closed file descriptor {fd}")]
    InvalidFd {
        fd: RawFd,
    },

    /// Критическая ошибка записи
    ///
    /// Требует немедленного внимания (например, ENOMEM, EIO и др.)
    #[error("Fatal write error on file descriptor {fd}: {msg}")]
    Fatal {
        fd: RawFd,
        msg: String,
    },

    /// Нет данных для записи в буфере
    #[error("Write buffer is empty")]
    BufferEmpty,
}

/// Пишет данные из буфера в файловый дескриптор
///
/// # Аргументы
/// * `fd` — файловый дескриптор для записи
/// * `buffer` — буфер с данными для записи
///
/// # Возвращает
/// Результат операции в виде `WriteResult`
pub fn write_fd(fd: RawFd, buffer: &mut Buffer) -> WriteResult {
    // Проверяем, есть ли данные для записи
    if buffer.get_data_len() == 0 {
        return WriteResult::BufferEmpty;
    }

    // Получаем срез данных для записи
    let src = buffer.as_mut_data_slice();

    // Системный вызов write()
    let res = unsafe {
        libc::write(
            fd,
            src.as_ptr() as *const libc::c_void,
            src.len(),
        )
    };

    let res = Errno::result(res).map(|r| r as usize);

    match res {
        Ok(0) => {
            // EOF или соединение закрыто
            WriteResult::Eof { fd }
        }
        Ok(n) => {
            // Успешная запись n байт
            // Сдвигаем буфер (удаляем записанные данные)
            buffer.consume(n);

            WriteResult::Success(n)
        }
        Err(e) if e == Errno::EAGAIN || e == Errno::EWOULDBLOCK => {
            WriteResult::WouldBlock { fd }
        }
        Err(Errno::EINTR) => {
            WriteResult::Interrupted { fd }
        }
        Err(e)
            if e == Errno::EBADF
                || e == Errno::ENOTCONN
                || e == Errno::ECONNRESET
                || e == Errno::ETIMEDOUT
                || e == Errno::ENXIO =>
        {
            WriteResult::InvalidFd { fd }
        }
        Err(e) => {
            WriteResult::Fatal {
                fd,
                msg: format!("write() failed: {}", e),
            }
        }
    }
}