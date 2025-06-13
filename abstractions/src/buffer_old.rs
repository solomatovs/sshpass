use std::alloc::{self, AllocError, Layout};
use std::ops::{Deref, DerefMut};
use std::ptr::NonNull;
use std::slice;
use std::mem::{size_of, align_of};



/// C-совместимая структура для буфера
#[repr(C)]
#[derive(Debug, Clone)]
pub struct BufferRaw {
    pub data: *mut u8,       // Указатель на данные
    pub capacity: usize,     // Общая емкость буфера
    pub max_capacity: usize, // Максимальный размер, до которого может вырасти буфер
    pub setup_len: usize,    // Длина, установленная при создании
    pub data_len: usize,     // Текущая длина данных
    pub offset: usize,       // Смещение от начала буфера до начала данных
}

/// Rust-обертка для удобной работы с буфером
#[derive(Debug)]
pub struct Buffer {
    raw: BufferRaw,
    // Используем NonNull для гарантии ненулевого указателя
    // Это поле приватное и используется только для Drop
    ptr: Option<NonNull<u8>>,
    layout: Option<Layout>, // Сохраняем Layout для корректного освобождения памяти
}

impl Clone for Buffer {
    fn clone(&self) -> Self {
        let mut new_buffer = if self.raw.capacity == 0 {
            Self::new_empty(self.raw.max_capacity)
        } else {
            match Self::with_max_capacity(self.raw.capacity, self.raw.max_capacity) {
                Ok(buf) => buf,
                Err(_) => panic!("Failed to allocate memory for buffer clone"),
            }
        };
        
        // Копируем данные, если они есть
        if self.raw.data_len > 0 {
            unsafe {
                std::ptr::copy_nonoverlapping(
                    self.raw.data.add(self.raw.offset),
                    new_buffer.raw.data,
                    self.raw.data_len
                );
            }
            new_buffer.raw.data_len = self.raw.data_len;
            // Не копируем offset, так как данные копируются с начала нового буфера
        }
        
        new_buffer
    }
}

impl Buffer {
    /// Создает новый буфер с начальным размером 0 и указанным максимальным размером
    pub fn new_empty(max_capacity: usize) -> Self {
        Self {
            raw: BufferRaw {
                data: std::ptr::null_mut(),
                capacity: 0,
                setup_len: 0,
                data_len: 0,
                max_capacity,
                offset: 0,
            },
            ptr: None,
            layout: None,
        }
    }

    /// Создает новый буфер с указанным начальным размером
    pub fn new(setup_len: usize) -> Result<Self, AllocError> {
        Self::with_max_capacity(setup_len, setup_len * 10) // По умолчанию максимальный размер в 10 раз больше начального
    }

    /// Создает новый буфер с указанным начальным и максимальным размером
    pub fn with_max_capacity(setup_len: usize, max_capacity: usize) -> Result<Self, AllocError> {
        // Обработка случая с нулевой емкостью
        if setup_len == 0 {
            return Ok(Self::new_empty(max_capacity));
        }

        // Проверка на переполнение при выделении памяти
        let layout = match Layout::array::<u8>(setup_len) {
            Ok(layout) => layout,
            Err(_) => return Err(AllocError),
        };

        // Выделяем память
        let ptr = unsafe { alloc::alloc(layout) };
        if ptr.is_null() {
            return Err(AllocError);
        }

        // Преобразуем в NonNull
        let ptr = match NonNull::new(ptr) {
            Some(p) => p,
            None => return Err(AllocError),
        };

        Ok(Self {
            raw: BufferRaw {
                data: ptr.as_ptr(),
                capacity: setup_len,
                setup_len,
                data_len: 0,
                max_capacity,
                offset: 0,
            },
            ptr: Some(ptr),
            layout: Some(layout),
        })
    }
    
    /// Создает новый буфер с указанным начальным размером
    pub fn try_new(setup_len: usize) -> Result<Self, AllocError> {
        Self::with_max_capacity(setup_len, setup_len * 10)
    }
    /// Создает новую структуру BufferRaw с теми же значениями, без перемещения
    pub fn create_raw(&self) -> BufferRaw {
        BufferRaw {
            data: self.raw.data,
            capacity: self.raw.capacity,
            data_len: self.raw.data_len,
            setup_len: self.raw.setup_len,
            max_capacity: self.raw.max_capacity,
            offset: self.raw.offset,
        }
    }

    /// Устанавливает длину данных в буфере
    /// Возвращает true, если длина установлена успешно
    pub fn set_data_len(&mut self, data_len: usize) -> bool {
        if self.raw.offset + data_len <= self.raw.capacity {
            self.raw.data_len = data_len;
            true
        } else {
            false
        }
    }

    /// Устанавливает смещение в буфере
    /// Возвращает true, если смещение установлено успешно
    pub fn set_offset(&mut self, offset: usize) -> bool {
        if offset + self.raw.data_len <= self.raw.capacity {
            self.raw.offset = offset;
            true
        } else {
            false
        }
    }

    /// Увеличивает смещение на указанное количество байт
    /// Возвращает true, если смещение увеличено успешно
    pub fn advance_offset(&mut self, bytes: usize) -> bool {
        if bytes <= self.raw.data_len {
            self.raw.offset += bytes;
            self.raw.data_len -= bytes;
            true
        } else {
            false
        }
    }

    /// Получает текущее смещение в буфере
    pub fn get_offset(&self) -> usize {
        self.raw.offset
    }

    /// Возвращает длину, установленную при создании
    pub fn get_setup_len(&self) -> usize {
        self.raw.setup_len
    }

    /// Возвращает текущую длину данных
    pub fn get_data_len(&self) -> usize {
        self.raw.data_len
    }

    /// Возвращает общую емкость буфера
    pub fn get_capacity(&self) -> usize {
        self.raw.capacity
    }

    /// Возвращает максимальный размер буфера
    pub fn get_max_capacity(&self) -> usize {
        self.raw.max_capacity
    }

    /// Пытается увеличить буфер до указанного размера, но не больше максимальной емкости
    /// Возвращает true, если буфер был успешно увеличен или уже имеет достаточный размер
    /// Возвращает false, если достигнут предел или не удалось выделить память
    pub fn try_grow(&mut self, target_capacity: usize) -> bool {
        if target_capacity <= self.raw.capacity {
            return true; // Буфер уже достаточного размера
        }

        let new_capacity = target_capacity.min(self.raw.max_capacity);

        if new_capacity <= self.raw.capacity {
            return false; // Не можем увеличить буфер (достигнут предел)
        }

        // Создаем новый Layout для нового размера
        let new_layout = match Layout::array::<u8>(new_capacity) {
            Ok(layout) => layout,
            Err(_) => return false, // Не можем создать Layout
        };
        
        // Выделяем новую память
        let new_ptr = unsafe { alloc::alloc(new_layout) };
        if new_ptr.is_null() {
            return false; // Не удалось выделить память
        }
        
        // Копируем данные из старого буфера с учетом смещения
        if !self.raw.data.is_null() && self.raw.data_len > 0 {
            unsafe {
                std::ptr::copy_nonoverlapping(
                    self.raw.data.add(self.raw.offset),
                    new_ptr,
                    self.raw.data_len
                );
            }
        }
        
        // Освобождаем старую память
        if let (Some(old_ptr), Some(old_layout)) = (self.ptr.take(), self.layout.take()) {
            unsafe {
                alloc::dealloc(old_ptr.as_ptr(), old_layout);
            }
        }
        
        // Обновляем указатели и размеры
        if let Some(new_ptr_non_null) = NonNull::new(new_ptr) {
            self.ptr = Some(new_ptr_non_null);
            self.layout = Some(new_layout);
            self.raw.data = new_ptr;
            self.raw.capacity = new_capacity;
            // Сбрасываем смещение, так как данные теперь в начале буфера
            self.raw.offset = 0;
            true
        } else {
            false
        }
    }

    /// Изменяет размер буфера на указанный
    /// Если новый размер превышает max_capacity, он будет ограничен этим значением
    pub fn reallocate(&mut self, new_capacity: usize) {
        let actual_capacity = new_capacity.min(self.raw.max_capacity);
        
        // Если новая емкость равна текущей, ничего не делаем
        if actual_capacity == self.raw.capacity {
            return;
        }
        
        // Если новая емкость равна 0, освобождаем память
        if actual_capacity == 0 {
            if let Some(ptr) = self.ptr.take() {
                if let Some(layout) = self.layout.take() {
                    unsafe {
                        alloc::dealloc(ptr.as_ptr(), layout);
                    }
                }
            }
            
            self.raw.data = std::ptr::null_mut();
            self.raw.capacity = 0;
            self.raw.data_len = 0;
            self.raw.offset = 0;
            return;
        }
        
        // Создаем новый Layout для нового размера
        let new_layout = match Layout::array::<u8>(actual_capacity) {
            Ok(layout) => layout,
            Err(_) => return, // Не можем создать Layout, ничего не делаем
        };
        
        // Выделяем новую память
        let new_ptr = unsafe { alloc::alloc(new_layout) };
        if new_ptr.is_null() {
            return; // Не удалось выделить память, оставляем буфер как есть
        }
        
        // Копируем данные из старого буфера с учетом смещения
        if !self.raw.data.is_null() && self.raw.data_len > 0 {
            let copy_len = self.raw.data_len.min(actual_capacity);
            unsafe {
                std::ptr::copy_nonoverlapping(
                    self.raw.data.add(self.raw.offset),
                    new_ptr,
                    copy_len
                );
            }
        }
        
        // Освобождаем старую память
        if let (Some(old_ptr), Some(old_layout)) = (self.ptr.take(), self.layout.take()) {
            unsafe {
                alloc::dealloc(old_ptr.as_ptr(), old_layout);
            }
        }
        
        // Обновляем указатели и размеры
        if let Some(new_ptr_non_null) = NonNull::new(new_ptr) {
            self.ptr = Some(new_ptr_non_null);
            self.layout = Some(new_layout);
            self.raw.data = new_ptr;
            self.raw.capacity = actual_capacity;
            
            // Если новый размер меньше текущей длины данных, обрезаем данные
            if self.raw.data_len > actual_capacity {
                self.raw.data_len = actual_capacity;
            }
            
            // Сбрасываем смещение, так как данные теперь в начале буфера
            self.raw.offset = 0;
        }
    }

    /// Сбрасывает буфер, устанавливая data_len и offset в 0
    pub fn reset(&mut self) {
        self.raw.data_len = 0;
        self.raw.offset = 0;
    }

    /// Сжимает буфер, перемещая данные в начало буфера
    /// Это полезно, если offset большой и нужно освободить место
    pub fn compact(&mut self) {
        if self.raw.offset == 0 || self.raw.data_len == 0 {
            return; // Нечего сжимать
        }
        
        unsafe {
            // Перемещаем данные в начало буфера
            std::ptr::copy(
                self.raw.data.add(self.raw.offset),
                self.raw.data,
                self.raw.data_len
            );
        }
        
        self.raw.offset = 0;
    }

    /// Возвращает срез с данными буфера
    pub fn get_data_slice(&self) -> &[u8] {
        if self.raw.data_len == 0 || self.raw.data.is_null() {
            return &[];
        }
        
        unsafe {
            slice::from_raw_parts(self.raw.data.add(self.raw.offset), self.raw.data_len)
        }
    }

    /// Возвращает изменяемый срез с данными буфера
    pub fn get_mut_data_slice(&mut self) -> &mut [u8] {
        if self.raw.data_len == 0 || self.raw.data.is_null() {
            return &mut [];
        }
        
        unsafe {
            slice::from_raw_parts_mut(self.raw.data.add(self.raw.offset), self.raw.data_len)
        }
    }

    /// Возвращает изменяемый срез со свободным местом в буфере
    pub fn get_mut_free_slice(&mut self) -> &mut [u8] {
        if self.raw.capacity == 0 || self.raw.data.is_null() || 
            self.raw.offset + self.raw.data_len >= self.raw.capacity {
            return &mut [];
        }
        
        unsafe {
            slice::from_raw_parts_mut(
                self.raw.data.add(self.raw.offset + self.raw.data_len),
                self.raw.capacity - (self.raw.offset + self.raw.data_len)
            )
        }
    }

    /// Получает указатель на свободное место в буфере
    pub fn get_mut_free_space_ptr(&mut self) -> *mut u8 {
        if self.raw.capacity == 0 || self.raw.data.is_null() || 
            self.raw.offset + self.raw.data_len >= self.raw.capacity {
            return std::ptr::null_mut();
        }
        
        unsafe { self.raw.data.add(self.raw.offset + self.raw.data_len) }
    }

    /// Получает ссылку на C-совместимую структуру
    pub fn as_raw(&self) -> &BufferRaw {
        &self.raw
    }

    /// Получает изменяемую ссылку на C-совместимую структуру
    pub fn as_raw_mut(&mut self) -> &mut BufferRaw {
        &mut self.raw
    }

    /// Проверяет, достиг ли буфер своего максимального размера
    pub fn is_at_max_capacity(&self) -> bool {
        self.raw.capacity >= self.raw.max_capacity
    }

    /// Проверяет, есть ли свободное место в буфере
    pub fn has_free_space(&self) -> bool {
        self.raw.offset + self.raw.data_len < self.raw.capacity
    }

    /// Возвращает количество свободного места в буфере
    pub fn free_space(&self) -> usize {
        self.raw.capacity - (self.raw.offset + self.raw.data_len)
    }

    /// Проверяет, имеет ли буфер нулевую емкость
    pub fn is_empty_capacity(&self) -> bool {
        self.raw.capacity == 0
    }
    
    /// Проверяет, нужно ли сжать буфер (если смещение занимает значительную часть буфера)
    pub fn should_compact(&self) -> bool {
        // Если смещение занимает более 25% буфера, рекомендуется сжатие
        self.raw.offset > 0 && self.raw.offset > self.raw.capacity / 4
    }

    /// Безопасно копирует структуру `T` в буфер (в конец текущих данных)
    pub fn push_struct<T: Copy>(&mut self, value: &T) -> bool {
        let size = size_of::<T>();
        let align = align_of::<T>();

        if size == 0 {
            return true; // пустая структура
        }

        // Обеспечим выравнивание и достаточную емкость
        let offset = self.raw.offset + self.raw.data_len;
        let aligned_offset = (offset + align - 1) & !(align - 1);
        let new_data_len = aligned_offset + size - self.raw.offset;

        if !self.try_grow(aligned_offset + size) {
            return false;
        }

        unsafe {
            let dst = self.raw.data.add(aligned_offset);
            std::ptr::copy_nonoverlapping(value as *const T as *const u8, dst, size);
        }

        self.raw.data_len = new_data_len;
        true
    }
    
    /// Читает структуру из буфера по смещению, возвращая ссылку на неё
    /// Возвращает `None`, если данных недостаточно или выход за границы
    pub fn read_struct<T>(&self) -> Option<&T>
    where
        T: Sized,
    {
        let start = self.raw.offset;
        let end = start.checked_add(std::mem::size_of::<T>())?;

        if end > self.raw.capacity {
            return None;
        }

        unsafe {
            let ptr = self.raw.data.add(start) as *const T;
            Some(&*ptr)
        }
    }

    /// То же самое, но возвращает изменяемую ссылку
    pub fn read_struct_mut<T>(&mut self) -> Option<&mut T>
    where
        T: Sized,
    {
        let start = self.raw.offset;
        let end = start.checked_add(std::mem::size_of::<T>())?;

        if end > self.raw.capacity {
            return None;
        }

        unsafe {
            let ptr = self.raw.data.add(start) as *mut T;
            Some(&mut *ptr)
        }
    }

    /// Считывает структуру `T` и сдвигает смещение (consume)
    pub fn take_struct<T: Copy>(&mut self) -> Option<T> {
        let size = size_of::<T>();
        let align = align_of::<T>();

        if size == 0 {
            return Some(unsafe { std::mem::zeroed() });
        }

        let offset = self.raw.offset;
        let aligned_offset = (offset + align - 1) & !(align - 1);

        if aligned_offset + size > self.raw.offset + self.raw.data_len {
            return None;
        }

        unsafe {
            let src = self.raw.data.add(aligned_offset) as *const T;
            let result = *src;
            let advance_by = aligned_offset + size - self.raw.offset;
            self.advance_offset(advance_by);
            Some(result)
        }
    }
}


// Реализуем Drop для корректного освобождения памяти
impl Drop for Buffer {
    fn drop(&mut self) {
        // Освобождаем память, если она была выделена
        if let (Some(ptr), Some(layout)) = (self.ptr, self.layout) {
            unsafe {
                alloc::dealloc(ptr.as_ptr(), layout);
            }
        }
    }
}

impl Deref for Buffer {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        self.get_data_slice()
    }
}

impl DerefMut for Buffer {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.get_mut_data_slice()
    }
}

// Реализуем конвертацию из Buffer в BufferRaw для C-функций
impl AsRef<BufferRaw> for Buffer {
    fn as_ref(&self) -> &BufferRaw {
        &self.raw
    }
}

// Реализуем конвертацию из &mut Buffer в &mut BufferRaw для C-функций
impl AsMut<BufferRaw> for Buffer {
    fn as_mut(&mut self) -> &mut BufferRaw {
        &mut self.raw
    }
}
