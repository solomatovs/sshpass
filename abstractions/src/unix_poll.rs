use nix::libc;

/// C-совместимая структура для работы с poll
#[derive(Debug)]
#[repr(C)]
pub struct UnixPollRaw {
    pub timeout: i32,
    pub result: i32,
    pub fds_ptr: *mut libc::pollfd,
    pub fds_len: usize,
    pub revents: i32,
}

/// Rust-обертка для удобной работы с UnixPoll
#[derive(Debug)]
pub struct UnixPoll {
    raw: UnixPollRaw,
    fds: Vec<libc::pollfd>,
}

impl UnixPoll {
    /// Создает новый экземпляр UnixPoll
    pub fn new(timeout: i32) -> Self {
        let mut fds = Vec::new();
        let raw = UnixPollRaw {
            timeout,
            result: 0,
            fds_ptr: fds.as_mut_ptr(),
            fds_len: 0,
            revents: 0,
        };
        
        UnixPoll { raw, fds }
    }
    
    /// Создает UnixPoll с предварительно выделенной емкостью для fds
    pub fn with_capacity(timeout: i32, capacity: usize) -> Self {
        let mut fds = Vec::with_capacity(capacity);
        let raw = UnixPollRaw {
            timeout,
            result: 0,
            fds_ptr: fds.as_mut_ptr(),
            fds_len: 0,
            revents: 0,
        };
        
        UnixPoll { raw, fds }
    }
    
    /// Добавляет новый файловый дескриптор в массив fds
    pub fn add_fd(&mut self, fd: i32, events: i16) {
        self.fds.push(libc::pollfd {
            fd,
            events,
            revents: 0,
        });
        
        // Обновляем указатель и длину в raw структуре
        self.update_raw();
    }
    
    /// Удаляет файловый дескриптор из массива fds
    pub fn remove_fd(&mut self, fd: i32) -> bool {
        let initial_len = self.fds.len();
        self.fds.retain(|pollfd| pollfd.fd != fd);
        let removed = self.fds.len() < initial_len;
        
        // Обновляем указатель и длину в raw структуре
        self.update_raw();
        
        removed
    }
    
    /// Получает срез fds для чтения
    pub fn fds(&self) -> &[libc::pollfd] {
        &self.fds
    }

    pub fn len(&self) -> usize {
        self.fds.len()
    }

    pub fn is_empty(&self) -> bool {
        self.fds.is_empty()
    }
    
    /// Получает изменяемый срез fds
    pub fn fds_mut(&mut self) -> &mut [libc::pollfd] {
        &mut self.fds
    }
    
    /// Очищает массив fds
    pub fn clear_fds(&mut self) {
        self.fds.clear();
        
        // Обновляем указатель и длину в raw структуре
        self.update_raw();
    }
    
    /// Обновляет указатель и длину в raw структуре
    fn update_raw(&mut self) {
        self.raw.fds_ptr = self.fds.as_mut_ptr();
        self.raw.fds_len = self.fds.len();
    }
    
    /// Получает ссылку на C-совместимую структуру
    pub fn as_raw(&self) -> &UnixPollRaw {
        &self.raw
    }
    
    /// Получает изменяемую ссылку на C-совместимую структуру
    pub fn as_raw_mut(&mut self) -> &mut UnixPollRaw {
        &mut self.raw
    }
    
    /// Устанавливает результат poll
    pub fn set_result(&mut self, result: i32) {
        self.raw.result = result;
    }
    
    /// Устанавливает revents
    pub fn set_revents(&mut self, revents: i32) {
        self.raw.revents = revents;
    }
    
    /// Получает timeout
    pub fn timeout(&self) -> i32 {
        self.raw.timeout
    }
    
    /// Устанавливает timeout
    pub fn set_timeout(&mut self, timeout: i32) {
        self.raw.timeout = timeout;
    }
    
    /// Получает результат poll
    pub fn result(&self) -> i32 {
        self.raw.result
    }
    
    /// Получает revents
    pub fn revents(&self) -> i32 {
        self.raw.revents
    }
}

// Реализуем конвертацию из UnixPoll в UnixPollRaw для C-функций
impl AsRef<UnixPollRaw> for UnixPoll {
    fn as_ref(&self) -> &UnixPollRaw {
        &self.raw
    }
}

// Реализуем конвертацию из &mut UnixPoll в &mut UnixPollRaw для C-функций
impl AsMut<UnixPollRaw> for UnixPoll {
    fn as_mut(&mut self) -> &mut UnixPollRaw {
        &mut self.raw
    }
}
