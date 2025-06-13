use nix::libc;
use std::collections::HashMap;
use std::os::fd::RawFd;

/// C-совместимая структура для работы с poll
#[derive(Debug)]
#[repr(C)]
pub struct UnixPollRaw {
    pub fds_ptr: *mut libc::pollfd,
    pub fds_len: usize,
    pub timeout: i32,
    pub result: i32,
}

/// Rust-обертка для удобной работы с UnixPoll
#[derive(Debug)]
pub struct UnixPoll {
    raw: UnixPollRaw,
    fds: Vec<libc::pollfd>,
    // Добавляем HashMap для быстрого поиска по fd
    fds_map: HashMap<RawFd, usize>,
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
            // fds_buffer_ptr: std::ptr::null_mut(),
        };

        UnixPoll {
            raw,
            fds,
            fds_map: HashMap::new(),
        }
    }

    /// Создает UnixPoll с предварительно выделенной емкостью для fds
    pub fn with_capacity(timeout: i32, capacity: usize) -> Self {
        let mut fds = Vec::with_capacity(capacity);
        let raw = UnixPollRaw {
            fds_ptr: fds.as_mut_ptr(),
            fds_len: 0,
            timeout,
            result: 0,
        };

        UnixPoll {
            raw,
            fds,
            fds_map: HashMap::with_capacity(capacity),
        }
    }

    /// Добавляет новый файловый дескриптор в массив fds с буферами указанного размера
    /// Возвращает true, если fd успешно добавлен, false если fd уже существует или не удалось создать буферы
    pub fn add_fd(
        &mut self,
        fd: i32,
        events: i16,
    ) -> bool {
        // Проверяем, есть ли уже такой fd
        if self.fds_map.contains_key(&fd) {
            return false;
        }

        let pollfd = libc::pollfd {
            fd,
            events,
            revents: 0,
        };

        // Добавляем fd в вектор для poll
        self.fds.push(pollfd);

        // Сохраняем индекс в HashMap
        let index = self.fds.len() - 1;
        self.fds_map.insert(fd, index);

        // Обновляем указатель и длину в raw структуре
        self.update_raw();

        true
    }

    /// Добавляет новый файловый дескриптор с попыткой создать буферы указанного размера
    /// Если не удается выделить память указанного размера, пытается создать буферы меньшего размера
    pub fn add_fd_with_fallback(
        &mut self,
        fd: i32,
        events: i16,
    ) -> bool {
        // Проверяем, есть ли уже такой fd
        if self.fds_map.contains_key(&fd) {
            return false;
        }

        let pollfd = libc::pollfd {
            fd,
            events,
            revents: 0,
        };

        // Добавляем fd в вектор для poll
        self.fds.push(pollfd);

        // Сохраняем индекс в HashMap
        let index = self.fds.len() - 1;
        self.fds_map.insert(fd, index);

        // Обновляем указатель и длину в raw структуре
        self.update_raw();

        true
    }

    /// Удаляет файловый дескриптор из массива fds
    pub fn remove_fd(&mut self, fd: i32) -> bool {
        if let Some(index) = self.fds_map.remove(&fd) {
            // Удаляем из вектора fds
            self.fds.swap_remove(index);

            // Если мы удалили не последний элемент, нужно обновить индекс
            if index < self.fds.len() {
                let moved_fd = self.fds[index].fd;
                self.fds_map.insert(moved_fd, index);
            }

            // Обновляем указатель и длину в raw структуре
            self.update_raw();

            true
        } else {
            false
        }
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

    /// Получает изменяемую ссылку на pollfd для указанного fd
    pub fn get_fd_mut(&mut self, fd: RawFd) -> Option<&mut libc::pollfd> {
        self.fds_map
            .get(&fd)
            .copied()
            .map(move |index| &mut self.fds[index])
    }

    /// Очищает массив fds
    pub fn clear_fds(&mut self) {
        self.fds.clear();
        self.fds_map.clear();
        // self.fds_buffer.clear();

        // Обновляем указатель и длину в raw структуре
        self.update_raw();
    }

    /// Обновляет указатель и длину в raw структуре
    fn update_raw(&mut self) {
        self.raw.fds_ptr = if self.fds.is_empty() {
            std::ptr::null_mut()
        } else {
            self.fds.as_mut_ptr()
        };

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

    /// Получает результат poll
    pub fn get_result(&self) -> i32 {
        self.raw.result
    }

    /// Устанавливает результат poll
    pub fn set_result(&mut self, result: i32) {
        self.raw.result = result;
    }

    /// Получает timeout
    pub fn get_timeout(&self) -> i32 {
        self.raw.timeout
    }

    /// Устанавливает timeout
    pub fn set_timeout(&mut self, timeout: i32) {
        self.raw.timeout = timeout;
    }

    /// Проверяет наличие файлового дескриптора
    pub fn has_fd(&self, fd: RawFd) -> bool {
        self.fds_map.contains_key(&fd)
    }

    /// Получает индекс файлового дескриптора в массиве fds
    pub fn get_fd_index(&self, fd: RawFd) -> Option<usize> {
        self.fds_map.get(&fd).copied()
    }

    /// Обновляет события для указанного fd
    pub fn upd_events(&mut self, fd: RawFd, events: i16) -> bool {
        if let Some(&index) = self.fds_map.get(&fd) {
            self.fds[index].events = events;
            true
        } else {
            false
        }
    }

    /// Получает события для указанного fd
    pub fn get_events(&self, fd: RawFd) -> Option<i16> {
        self.fds_map.get(&fd).map(|&index| self.fds[index].events)
    }

    /// Получает возвращенные события для указанного fd
    pub fn get_revents(&self, fd: RawFd) -> Option<i16> {
        self.fds_map.get(&fd).map(|&index| self.fds[index].revents)
    }

    /// сбрасывает возвращенные события для указанного fd
    pub fn reset_revents(&mut self, fd: RawFd) -> bool {
        if let Some(index) = self.fds_map.get(&fd) {
            self.fds[*index].revents = 0;
            return true;
        }

        false
    }

    /// Проверяет, установлен ли указанный флаг в revents для fd
    pub fn has_reevent(&self, fd: RawFd, event_flag: i16) -> bool {
        self.get_revents(fd)
            .is_some_and(|revents| (revents & event_flag) != 0)
    }

    /// Итератор по всем fd с установленными revents
    pub fn iter_ready_fds(&self) -> impl Iterator<Item = (RawFd, i16)> + '_ {
        self.fds
            .iter()
            .filter(|pollfd| pollfd.revents != 0)
            .map(|pollfd| (pollfd.fd, pollfd.revents))
    }

    /// Получает C-совместимый массив файловых дескрипторов
    /// Полезно для передачи в C-код
    pub fn get_fds_array(&self) -> Vec<i32> {
        self.fds.iter().map(|pollfd| pollfd.fd).collect()
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
