// use crate::buffer::{Buffer, BufferRaw};
// use std::alloc::AllocError;

// /// C-совместимая структура для хранения информации о файловом дескрипторе
// #[derive(Debug)]
// #[repr(C)]
// pub struct FdBufferRaw {
//     pub read: BufferRaw,
//     pub write: BufferRaw,
// }

// /// Rust-обертка для информации о файловом дескрипторе
// #[derive(Debug)]
// pub struct FdBuffer {
//     raw: FdBufferRaw,
//     read: Buffer,
//     write: Buffer,
// }

// impl FdBuffer {
//     /// Создает новый экземпляр FdInfo
//     /// Возвращает Result для обработки ошибок аллокации
//     pub fn new(read_buffer_size: usize, write_buffer_size: usize) -> Result<Self, AllocError> {
//         // Создаем буферы с обработкой ошибок
//         let read = Buffer::new(read_buffer_size)?;
//         let write = Buffer::new(write_buffer_size)?;

//         let raw = FdBufferRaw {
//             read: read.create_raw(),
//             write: write.create_raw(),
//         };

//         Ok(Self {
//             raw,
//             read,
//             write,
//         })
//     }

//     /// Создает новый экземпляр с пустыми буферами (для случаев, когда аллокация не нужна)
//     pub fn new_empty(max_read_capacity: usize, max_write_capacity: usize) -> Self {
//         let read = Buffer::new_empty(max_read_capacity);
//         let write = Buffer::new_empty(max_write_capacity);

//         let raw = FdBufferRaw {
//             read: read.create_raw(),
//             write: write.create_raw(),
//         };

//         Self {
//             raw,
//             read,
//             write,
//         }
//     }
    
//     /// Получает ссылку на C-совместимую структуру
//     pub fn as_raw(&self) -> &FdBufferRaw {
//         &self.raw
//     }

//     /// Получает изменяемую ссылку на C-совместимую структуру
//     pub fn as_raw_mut(&mut self) -> &mut FdBufferRaw {
//         &mut self.raw
//     }

//     /// Обновляет raw структуру из буферов
//     pub fn update_raw(&mut self) {
//         self.raw.read = self.read.create_raw();
//         self.raw.write = self.write.create_raw();
//     }

//     /// Получает буфер для чтения
//     pub fn read_buffer(&self) -> &Buffer {
//         &self.read
//     }

//     /// Получает изменяемый буфер для чтения
//     pub fn read_buffer_mut(&mut self) -> &mut Buffer {
//         &mut self.read
//     }

//     /// Получает буфер для записи
//     pub fn write_buffer(&self) -> &Buffer {
//         &self.write
//     }

//     /// Получает изменяемый буфер для записи
//     pub fn write_buffer_mut(&mut self) -> &mut Buffer {
//         &mut self.write
//     }
// }

// // Реализация AsRef для FdInfo
// impl AsRef<FdBufferRaw> for FdBuffer {
//     fn as_ref(&self) -> &FdBufferRaw {
//         &self.raw
//     }
// }

// // Реализация AsMut для FdInfo
// impl AsMut<FdBufferRaw> for FdBuffer {
//     fn as_mut(&mut self) -> &mut FdBufferRaw {
//         &mut self.raw
//     }
// }
