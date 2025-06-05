use std::ops::{Deref, DerefMut};
use std::alloc::{self, AllocError, Layout};


#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Buffer {
    buf: Vec<u8>,
    data_len: usize,
    setup_len: usize,
}

impl Buffer {
    pub fn new(setup_len: usize) -> Self {
        Self {
            buf: vec![0; setup_len],
            data_len: 0,
            setup_len,
        }
    }

    pub fn try_new(setup_len: usize) -> Result<Self, AllocError> {
        // Обработка случая с нулевой емкостью
        if setup_len == 0 {
            return Ok(Self {
                buf: Vec::new(),
                data_len: 0,
                setup_len,
            });
        }

        // Проверка на переполнение при выделении памяти
        let layout = match Layout::array::<u8>(setup_len) {
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
            // это корректный вариант
            // let buf = Vec::from_raw_parts(ptr, setup_len, setup_len);
            // это для стресс тестирования, заранее выделил некорректный размер буфера, программа должна адаптироваться и менять значение буфера на нужное
            let buf = Vec::from_raw_parts(ptr, 0, 0);
            Ok(Self {
                buf,
                data_len: 0,
                setup_len,
            })
        }
    }

    pub fn set_data_len(&mut self, data_len: usize) {
        self.data_len = data_len;
    }

    pub fn get_data_len(&self) -> usize {
        self.data_len
    }

    pub fn get_setting_len(&mut self) -> usize {
        self.setup_len
    }

    pub fn get_buffer_len(&self) -> usize {
        self.buf.len()
    }

    pub fn reallocate(&mut self, set_size: usize) {
        self.buf.resize(set_size, 0);

        if self.data_len > set_size {
            // если данные больше нового размера буфера, то обнуляем data_len
            // так как этот размер неверен и при чтении можно получить ошибку
            self.data_len = 0;
        }

        self.setup_len = set_size;
    }

    pub fn get_data_slice(&self) -> &[u8] {
        &self.buf[..self.data_len]
    }

    pub fn get_mut_data_slice(&mut self) -> &mut [u8] {
        &mut self.buf[..self.data_len]
    }

    pub fn get_mut_buffer_slice(&mut self) -> &mut [u8] {
        &mut self.buf[..]
    }
}

impl Deref for Buffer {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        &self.buf[..self.data_len]
    }
}

impl DerefMut for Buffer {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.buf[..self.data_len]
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