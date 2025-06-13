use libloading::{Library, Symbol};
use std::fs;
use std::sync::Arc;
use toml::Value;
use thiserror::Error;

use abstractions::{
    PluginRegistrator, PluginType, RegisterCPluginFn,
    RegisterRustPluginFn, UnixContext,
    warn,
};


#[derive(Debug, Error)]
pub enum PluginLoadError {
    #[error("Ошибка загрузки символа `{symbol_name}`: {error}")]
    SymbolLoadError {
        symbol_name: String,
        error: libloading::Error,
    },

    #[error("Метод `{symbol_name}` не найден")]
    SymbolNotFound {
        symbol_name: String,
    },

    #[error("Вызов метода: `{symbol_name}` завершился ошибкой `{error}`.")]
    PluginInitFailed {
        symbol_name: String,
        error: String,
    },
}

#[derive(Debug, Error)]
pub enum LibraryLoadError {
    #[error("Не удалось загрузить библиотеку {}: {}", library_name, error)]
    LibraryLoadError {
        library_name: String,
        error: String,
    },

    #[error("Не удалось найти символы {symbols_name:?} в библиотеке {library_name}")]
    SymbolsNotFound {
        library_name: String,
        symbols_name: String,
    },

    #[error("символ {symbol_name} не определен в библиотеке {library_name}")]
    SymbolUndefined {
        library_name: String,
        symbol_name: String,
    },

    #[error("Ошибка загрузки символа `{symbol_name}` в библиотеке `{library_name}`: {error}")]
    SymbolLoadError {
        library_name: String,
        symbol_name: String,
        error: libloading::Error,
    },

    #[error("Вызов метода: `{symbol_name}` завершился ошибкой `{error}`. Инициализация плагина `{library_name}` невозможна")]
    PluginInitFailed {
        library_name: String,
        symbol_name: String,
        error: String,
    },
}


/// Управляемый плагин, который владеет указателем на PluginInterface и соответствующей библиотекой.
/// Автоматически вызывает инициализацию при создании и освобождение ресурсов при уничтожении.
pub struct ManagedPlugin {
    plugin: PluginType<UnixContext>,    // Храним указатель на плагин
    _library: Arc<Library>,             // Храним библиотеку, чтобы она не выгрузилась
}

impl ManagedPlugin{
    pub fn try_load_library(plugin_name: &str) -> Result<Arc<Library>, LibraryLoadError> {
        let library = unsafe {
            Library::new(plugin_name)
                .map_err(|e| LibraryLoadError::LibraryLoadError {
                    library_name: plugin_name.to_string(),
                    error: e.to_string(),
                })?
        };

        Ok(Arc::new(library))
    }

    pub fn try_load_plugin(library_name: &str, ctx: &mut UnixContext) -> Result<Vec<Self>, LibraryLoadError> {
        let library = Self::try_load_library(library_name)?;
        let match_symbols = [
            "register_rust_plugin",
            "register_c_plugin",
        ];

        let mut res = vec![];

        for symbol_name in match_symbols {
            let plugins = match symbol_name {
                "register_rust_plugin" => {
                    Self::try_load_rust_plugin(library.clone(), symbol_name, ctx)
                }
                "register_c_plugin" => {
                    Self::try_load_c_plugin(library.clone(), symbol_name, ctx)
                }
                _ => {
                    return Err(LibraryLoadError::SymbolUndefined {
                        library_name: library_name.to_string(),
                        symbol_name: symbol_name.to_string(),
                    });
                }
            };

            match plugins {
                Err(PluginLoadError::SymbolNotFound { symbol_name: _ }) => {
                    continue;
                }
                Err(PluginLoadError::PluginInitFailed { symbol_name, error }) => {
                    return Err(LibraryLoadError::PluginInitFailed {
                        library_name: library_name.to_string(),
                        symbol_name,
                        error,
                    });
                }
                Err(PluginLoadError::SymbolLoadError { symbol_name, error }) => {
                    return Err(LibraryLoadError::SymbolLoadError {
                        library_name: library_name.to_string(),
                        symbol_name,
                        error,
                    });
                }
                Ok(plugins) => res.extend(plugins),
            }
        }

        if res.is_empty() {
            return Err(LibraryLoadError::SymbolsNotFound {
                library_name: library_name.to_string(),
                symbols_name: match_symbols.join(", "),
            })
        }

        Ok(res)
    }

    /// Создает новый экземпляр ManagedPlugin, инициализируя плагин.
    ///
    /// # Arguments
    /// * `plugin_name` - Имя плагина для сообщений об ошибках
    /// * `ctx` - Контекст приложения
    ///
    /// # Returns
    /// * `Result<Self, String>` - Успешно созданный ManagedPlugin или сообщение об ошибке
    pub fn try_load_c_plugin(
        library: Arc<Library>,
        symbol_name: &str,
        ctx: &mut UnixContext,
    ) -> Result<Vec<Self>, PluginLoadError> {
        let new: Symbol<RegisterCPluginFn<UnixContext>> = unsafe {
            library
                .get(symbol_name.as_bytes())
                .map_err(|e| match e {
                    libloading::Error::DlSymUnknown => PluginLoadError::SymbolNotFound {
                        symbol_name: symbol_name.to_string(),
                    },
                    libloading::Error::DlSym { desc: _ } => PluginLoadError::SymbolNotFound {
                        symbol_name: symbol_name.to_string(),
                    },
                    e => PluginLoadError::SymbolLoadError {
                        symbol_name: symbol_name.to_string(),
                        error: e,
                    },
                })?
        };

        let mut registrator = PluginRegistrator::new(ctx);

        if !new(registrator.as_c_interface()) {
            return Err(PluginLoadError::PluginInitFailed {
                symbol_name: symbol_name.to_string(),
                error: "".to_string(),
            });
        }

        let res = registrator.get_plugins().into_iter().map(|r| ManagedPlugin {
            plugin: r,
            _library: library.clone(),
        });

        Ok(res.collect())
    }

    /// Создает новый экземпляр ManagedPlugin, инициализируя плагин.
    ///
    /// # Arguments
    /// * `plugin_name` - Имя плагина для сообщений об ошибках
    /// * `ctx` - Контекст приложения
    ///
    /// # Returns
    /// * `Result<Self, String>` - Успешно созданный ManagedPlugin или сообщение об ошибке
    pub fn try_load_rust_plugin(
        library: Arc<Library>,
        symbol_name: &str,
        ctx: &mut UnixContext,
    ) -> Result<Vec<Self>, PluginLoadError> {
        let new: Symbol<RegisterRustPluginFn<UnixContext>> = unsafe {
            library
                .get(b"register_rust_plugin")
                .map_err(|e| match e {
                    libloading::Error::DlSymUnknown => PluginLoadError::SymbolNotFound {
                        symbol_name: symbol_name.to_string(),
                    },
                    libloading::Error::DlSym { desc: _ } => PluginLoadError::SymbolNotFound {
                        symbol_name: symbol_name.to_string(),
                    },
                    e => PluginLoadError::SymbolLoadError {
                        symbol_name: symbol_name.to_string(),
                        error: e,
                    },
                })?
        };

        let mut registrator = PluginRegistrator::new(ctx);

        if let Err(e) = new(&mut registrator) {
            return Err(PluginLoadError::PluginInitFailed {
                symbol_name: symbol_name.to_string(),
                error: e.to_string(),
            });
        }

        let res = registrator.get_plugins().into_iter().map(|r| ManagedPlugin {
            plugin: r,
            _library: library.clone(),
        });

        Ok(res.collect())
    }

    /// Обрабатывает событие с помощью плагина
    ///
    /// # Arguments
    /// * `ctx` - Контекст для обработки
    ///
    /// # Returns
    /// * `i32` - Результат обработки
    pub fn handle(&mut self, ctx: &mut UnixContext) -> i32 {
        match &mut self.plugin {
            PluginType::Rust(rust_plugin) => {
                rust_plugin.handle(ctx)
            }
            PluginType::C(c_plugin) => {
                unsafe {
                    ((*(*c_plugin)).handle)(*c_plugin, ctx as *mut UnixContext)
                }
            }
        }
    }

    /// Освобождает ресурсы плагина.
    ///
    /// # Arguments
    /// * `ctx` - Контекст для обработки
    ///
    /// # Returns
    /// * `i32` - Результат обработки
    pub fn free(&mut self, ctx: &mut UnixContext) -> i32 {
        match &mut self.plugin {
            PluginType::Rust(rust_plugin) => {
                rust_plugin.free(ctx)
            }
            PluginType::C(c_plugin) => {
                unsafe {
                    ((*(*c_plugin)).free)(*c_plugin, ctx as *mut UnixContext)
                }
            }
        }
    }
}

// // Реализуем Deref для доступа к методам PluginInterface
// impl Deref for ManagedPlugin {
//     type Target = PluginType<UnixContext>;

//     fn deref(&self) -> &Self::Target {
//         unsafe { &*self.plugin }
//     }
// }

// // Реализуем DerefMut для изменяемого доступа к методам PluginInterface
// impl DerefMut for ManagedPlugin {
//     fn deref_mut(&mut self) -> &mut Self::Target {
//         unsafe { &mut *self.plugin }
//     }
// }

pub struct PluginLoader {}

impl PluginLoader {
    /// Загружает плагины из конфигурационного файла
    ///
    /// # Arguments
    /// * `config_path` - Путь к конфигурационному файлу
    ///
    /// # Returns
    /// * `Result<Vec<ManagedPlugin>, String>` - Список загруженных плагинов или сообщение об ошибке
    pub fn reload_plugins(
        config_path: &str,
        ctx: &mut UnixContext,
    ) -> Result<Vec<ManagedPlugin>, String> {
        // Читаем конфиг
        let config_content = fs::read_to_string(config_path)
            .map_err(|e| format!("Не удалось прочитать config.toml: {}", e))?;

        let config: Value = config_content
            .parse::<Value>()
            .map_err(|e| format!("Ошибка парсинга config.toml: {}", e))?;

        let plugin_section = config.get("plugins").ok_or_else(|| {
            "Некорректный формат config.toml: отсутствует секция plugins".to_string()
        })?;

        let plugin_order = plugin_section
            .get("order")
            .and_then(|o| o.as_array())
            .ok_or_else(|| {
                "Некорректный формат config.toml: отсутствует массив plugins.order".to_string()
            })?;

        if plugin_order.is_empty() {
            return Err("В конфиге не указаны плагины".to_string());
        }

        let mut plugins = Vec::new();
        let mut load_errors = Vec::new();

        for plugin_value in plugin_order {
            // Поддержка как простых строк, так и объектов с настройками
            let (plugin_name, required) = match plugin_value {
                Value::String(name) => (name.as_str(), true), // По умолчанию обязательный
                Value::Table(table) => {
                    let name = table.get("name").and_then(|n| n.as_str()).ok_or_else(|| {
                        "Имя плагина должно быть указано в поле 'name'".to_string()
                    })?;

                    let required = table
                        .get("required")
                        .and_then(|r| r.as_bool())
                        .unwrap_or(true); // По умолчанию обязательный

                    (name, required)
                }
                _ => {
                    return Err(
                        "Элемент массива plugins.order должен быть строкой или таблицей"
                            .to_string(),
                    )
                }
            };

            match ManagedPlugin::try_load_plugin(plugin_name, ctx) {
                Ok(managed_plugin) => {
                    plugins.extend(managed_plugin);
                }
                Err(e) => {
                    if required {
                        return Err(format!(
                            "Не удалось загрузить обязательный плагин {}: {}",
                            plugin_name, e
                        ));
                    } else {
                        load_errors.push(format!(
                            "Пропуск необязательного плагина {}: {}",
                            plugin_name, e
                        ));
                    }
                }
            }
        }

        // Можно логировать ошибки загрузки необязательных плагинов
        for error in &load_errors {
            warn!(ctx, "{}", error);
        }

        Ok(plugins)
    }
}
