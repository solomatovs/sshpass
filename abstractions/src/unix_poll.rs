use nix::libc;
use std::collections::HashMap;
use std::os::fd::RawFd;
use std::sync::{Arc, Mutex, RwLock};
use std::sync::atomic::{AtomicI32, Ordering};

/// C-совместимая структура для работы с poll
#[derive(Debug)]
#[repr(C)]
pub struct UnixPollRaw {
    pub fds_ptr: *mut libc::pollfd,
    pub fds_len: usize,
    pub timeout: i32,
    pub result: i32,
}

// Структура для хранения состояния файловых дескрипторов
// Эта структура будет защищена Mutex
#[derive(Debug)]
struct FdsState {
    fds: Vec<libc::pollfd>,
    fds_map: HashMap<RawFd, usize>,
}

/// Rust-обертка для удобной работы с UnixPoll, адаптированная для многопоточности
#[derive(Debug, Clone)]
pub struct UnixPoll {
    // Состояние файловых дескрипторов защищено Mutex
    state: Arc<Mutex<FdsState>>,
    // Таймаут может быть изменен отдельно, используем RwLock для оптимизации чтения
    timeout: Arc<RwLock<i32>>,
    // Результат poll может быть изменен отдельно, используем AtomicI32
    result: Arc<AtomicI32>,
}

impl UnixPoll {
    /// Создает новый экземпляр UnixPoll
    pub fn new(timeout: i32) -> Self {
        Self {
            state: Arc::new(Mutex::new(FdsState {
                fds: Vec::new(),
                fds_map: HashMap::new(),
            })),
            timeout: Arc::new(RwLock::new(timeout)),
            result: Arc::new(AtomicI32::new(0)),
        }
    }

    /// Создает UnixPoll с предварительно выделенной емкостью для fds
    pub fn with_capacity(timeout: i32, capacity: usize) -> Self {
        Self {
            state: Arc::new(Mutex::new(FdsState {
                fds: Vec::with_capacity(capacity),
                fds_map: HashMap::with_capacity(capacity),
            })),
            timeout: Arc::new(RwLock::new(timeout)),
            result: Arc::new(AtomicI32::new(0)),
        }
    }

    /// Добавляет новый файловый дескриптор в массив fds
    /// Возвращает true, если fd успешно добавлен, false если fd уже существует
    pub fn add_fd(&self, fd: i32, events: i16) -> bool {
        let mut state = self.state.lock().unwrap();
        
        // Проверяем, есть ли уже такой fd
        if state.fds_map.contains_key(&fd) {
            return false;
        }

        let pollfd = libc::pollfd {
            fd,
            events,
            revents: 0,
        };

        // Добавляем fd в вектор для poll
        state.fds.push(pollfd);

        // Сохраняем индекс в HashMap
        let index = state.fds.len() - 1;
        state.fds_map.insert(fd, index);

        true
    }

    /// Добавляет новый файловый дескриптор с попыткой создать буферы указанного размера
    pub fn add_fd_with_fallback(&self, fd: i32, events: i16) -> bool {
        self.add_fd(fd, events)
    }

    /// Удаляет файловый дескриптор из массива fds
    pub fn remove_fd(&self, fd: i32) -> bool {
        let mut state = self.state.lock().unwrap();
        
        if let Some(index) = state.fds_map.remove(&fd) {
            // Удаляем из вектора fds
            state.fds.swap_remove(index);

            // Если мы удалили не последний элемент, нужно обновить индекс
            if index < state.fds.len() {
                let moved_fd = state.fds[index].fd;
                state.fds_map.insert(moved_fd, index);
            }

            true
        } else {
            false
        }
    }

    /// Получает копию текущего состояния fds
    /// Это безопасно для многопоточности, так как возвращается копия
    pub fn fds(&self) -> Vec<libc::pollfd> {
        let state = self.state.lock().unwrap();
        state.fds.clone()
    }

    /// Получает количество файловых дескрипторов
    pub fn len(&self) -> usize {
        let state = self.state.lock().unwrap();
        state.fds.len()
    }

    /// Проверяет, пуст ли список файловых дескрипторов
    pub fn is_empty(&self) -> bool {
        let state = self.state.lock().unwrap();
        state.fds.is_empty()
    }

    /// Обновляет события для указанного fd
    pub fn upd_events(&self, fd: RawFd, events: i16) -> bool {
        let mut state = self.state.lock().unwrap();
        
        if let Some(&index) = state.fds_map.get(&fd) {
            state.fds[index].events = events;
            true
        } else {
            false
        }
    }

    /// Получает события для указанного fd
    pub fn get_events(&self, fd: RawFd) -> Option<i16> {
        let state = self.state.lock().unwrap();
        state.fds_map.get(&fd).map(|&index| state.fds[index].events)
    }

    /// Получает возвращенные события для указанного fd
    pub fn get_revents(&self, fd: RawFd) -> Option<i16> {
        let state = self.state.lock().unwrap();
        state.fds_map.get(&fd).map(|&index| state.fds[index].revents)
    }

    /// Сбрасывает возвращенные события для указанного fd
    pub fn reset_revents(&self, fd: RawFd) -> bool {
        let mut state = self.state.lock().unwrap();
        
        // Сначала получаем копию индекса, а не ссылку
        if let Some(&index) = state.fds_map.get(&fd) {
            state.fds[index].revents = 0;
            return true;
        }

        false
    }

    /// Проверяет, установлен ли указанный флаг в revents для fd
    pub fn has_reevent(&self, fd: RawFd, event_flag: i16) -> bool {
        let state = self.state.lock().unwrap();
        
        state.fds_map.get(&fd)
            .map(|&index| (state.fds[index].revents & event_flag) != 0)
            .unwrap_or(false)
    }

    /// Итератор по всем fd с установленными revents
    /// Возвращает копию данных для безопасности в многопоточной среде
    pub fn iter_ready_fds(&self) -> Vec<(RawFd, i16)> {
        let state = self.state.lock().unwrap();
        
        state.fds.iter()
            .filter(|pollfd| pollfd.revents != 0)
            .map(|pollfd| (pollfd.fd, pollfd.revents))
            .collect()
    }

    /// Получает C-совместимый массив файловых дескрипторов
    pub fn get_fds_array(&self) -> Vec<i32> {
        let state = self.state.lock().unwrap();
        state.fds.iter().map(|pollfd| pollfd.fd).collect()
    }

    /// Очищает массив fds
    pub fn clear_fds(&self) {
        let mut state = self.state.lock().unwrap();
        state.fds.clear();
        state.fds_map.clear();
    }

    /// Получает результат poll
    pub fn get_result(&self) -> i32 {
        self.result.load(Ordering::SeqCst)
    }

    /// Устанавливает результат poll
    pub fn set_result(&self, result: i32) {
        self.result.store(result, Ordering::SeqCst);
    }

    /// Получает timeout
    pub fn get_timeout(&self) -> i32 {
        let timeout = self.timeout.read().unwrap();
        *timeout
    }

    /// Устанавливает timeout
    pub fn set_timeout(&self, timeout: i32) {
        let mut timeout_guard = self.timeout.write().unwrap();
        *timeout_guard = timeout;
    }

    /// Проверяет наличие файлового дескриптора
    pub fn has_fd(&self, fd: RawFd) -> bool {
        let state = self.state.lock().unwrap();
        state.fds_map.contains_key(&fd)
    }

    /// Получает индекс файлового дескриптора в массиве fds
    pub fn get_fd_index(&self, fd: RawFd) -> Option<usize> {
        let state = self.state.lock().unwrap();
        state.fds_map.get(&fd).copied()
    }

    /// Создает C-совместимую структуру для использования в функциях poll
    /// Важно: эта структура действительна только до следующего изменения UnixPoll
    pub fn as_raw(&self) -> UnixPollRaw {
        let state = self.state.lock().unwrap();
        let timeout = self.get_timeout();
        let result = self.get_result();
        
        let fds_ptr = if state.fds.is_empty() {
            std::ptr::null_mut()
        } else {
            // ВНИМАНИЕ: Это небезопасно для многопоточности!
            // Указатель действителен только пока существует блокировка state
            // Используйте этот метод только для кратковременных операций
            state.fds.as_ptr() as *mut libc::pollfd
        };
        
        UnixPollRaw {
            fds_ptr,
            fds_len: state.fds.len(),
            timeout,
            result,
        }
    }

    /// Безопасный метод для выполнения poll
    /// Этот метод блокирует состояние на время выполнения poll
    pub fn do_poll(&self) -> i32 {
        let mut state = self.state.lock().unwrap();
        let timeout = self.get_timeout();
        
        if state.fds.is_empty() {
            return 0;
        }
        
        // Выполняем poll, пока state заблокирован
        let result = unsafe {
            libc::poll(
                state.fds.as_mut_ptr(),
                state.fds.len() as libc::nfds_t,
                timeout,
            )
        };
        
        // Сохраняем результат
        self.set_result(result);
        
        result
    }
}
