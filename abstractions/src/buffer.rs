use std::mem::size_of;
use thiserror::Error;

#[derive(Debug)]
pub struct Buffer {
    buf: Vec<u8>,
    data_len: usize,
    // Смещение от начала: сколько байт уже прочитано/записано
    data_offset: usize,
}

/// Ошибки, которые могут возникнуть при работе с буфером (чтение, преобразование и т.д.)
#[derive(Debug, Error)]
pub enum BufferError {
    /// Ошибка возникает, если текущая длина данных в буфере меньше, чем размер запрашиваемой структуры.
    /// Например, попытка прочитать `signalfd_siginfo` (128 байт), когда в буфере только 4 байта.
    #[error("Buffer is too small to read structure: required {required} bytes, but only {available} available")]
    DataLenIsLessReadableType {
        /// Требуемый размер структуры в байтах
        required: usize,

        /// Фактически доступный размер данных в буфере
        available: usize,

        /// Имя типа структуры, которую пытались прочитать (например, "signalfd_siginfo")
        type_name: &'static str,
    },

    /// Ошибка выравнивания.
    /// Некоторые типы (например, структуры) требуют определённого выравнивания памяти для корректной интерпретации.
    /// Если указатель на начало буфера не соответствует выравниванию типа, чтение будет небезопасным.
    #[error("Buffer alignment error: required alignment {required}, but pointer is misaligned")]
    AlignError {
        /// Требуемое выравнивание в байтах (обычно зависит от архитектуры и структуры)
        required: usize,

        /// Имя типа, для которого проверялось выравнивание
        type_name: &'static str,
    },
}

impl Buffer {
    pub fn new(capacity: usize) -> Self {
        let mut buf = vec![0; capacity];
        // Заполняем нулями до capacity (для безопасного доступа)
        buf.resize(capacity, 0);

        Buffer {
            buf,
            data_len: 0,
            data_offset: 0,
        }
    }

    pub fn from_vec(vec: Vec<u8>) -> Self {
        let data_len = vec.len();
        Buffer {
            buf: vec,
            data_len,
            data_offset: 0,
        }
    }

    /// Получить срез данных для чтения
    pub fn as_data_slice(&self) -> &[u8] {
        &self.buf[self.data_offset..self.data_len]
    }

    /// Получить срез данных для записи
    pub fn as_mut_data_slice(&mut self) -> &mut [u8] {
        &mut self.buf[self.data_offset..self.data_len]
    }

    /// Получить срез свободного места для чтения
    pub fn as_free_slice(&mut self) -> &[u8] {
        &self.buf[self.data_len..]
    }

    /// Получить срез свободного места для записи
    pub fn as_mut_free_slice(&mut self) -> &mut [u8] {
        &mut self.buf[self.data_len..]
    }

    /// Удалить первые `n` байт из буфера (сдвинуть offset)
    pub fn consume(&mut self, n: usize) {
        self.data_offset += n;
        if self.data_offset >= self.data_len {
            self.clear(); // всё потреблено — сбрасываем полностью
        }
    }

    /// Попробовать вычитать структуру из начала буфера
    pub fn try_read_struct<T>(&self) -> Result<&T, BufferError> {
        let size = size_of::<T>();
        let align = std::mem::align_of::<T>();

        let available = self.data_len.saturating_sub(self.data_offset);

        if available < size {
            return Err(BufferError::DataLenIsLessReadableType {
                required: size,
                available,
                type_name: std::any::type_name::<T>(),
            });
        }

        let ptr = unsafe { self.buf.as_ptr().add(self.data_offset) };

        if ptr.align_offset(align) != 0 {
            return Err(BufferError::AlignError {
                required: align,
                type_name: std::any::type_name::<T>(),
            });
        }

        let reference = unsafe { &*(ptr as *const T) };
        Ok(reference)
    }

    pub fn clear(&mut self) {
        self.data_len = 0;
        self.data_offset = 0;
    }

    pub fn get_data_len(&self) -> usize {
        self.data_len - self.data_offset
    }

    pub fn set_data_len(&mut self, data_len: usize) -> Result<(), String> {
        if data_len > self.buf.len() {
            return Err(format!(
                "data_len ({data_len}) exceeds physical buffer size ({})",
                self.buf.len()
            ));
        }
        self.data_len = data_len;

        Ok(())
    }

    pub fn grow_data_len(&mut self, n: usize) -> Result<(), String> {
        self.set_data_len(self.get_offset() + self.get_data_len() + n)
    }

    pub fn resize(&mut self, new_size: usize) {
        self.buf.resize(new_size, 0);
    }

    pub fn capacity(&self) -> usize {
        self.buf.capacity()
    }

    pub fn get_offset(&self) -> usize {
        self.data_offset
    }
}

impl From<&str> for Buffer {
    fn from(s: &str) -> Self {
        let bytes = s.as_bytes().to_vec();
        let data_len = bytes.len();
        Buffer {
            buf: bytes,
            data_len,
            data_offset: 0,
        }
    }
}

impl From<String> for Buffer {
    fn from(s: String) -> Self {
        let bytes = s.as_bytes().to_vec();
        let data_len = bytes.len();
        Buffer {
            buf: bytes,
            data_len,
            data_offset: 0,
        }
    }
}