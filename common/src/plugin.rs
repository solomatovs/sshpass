use libloading::{Library, Symbol};
use std::{fs, sync::Arc};
use std::collections::HashMap;
use std::time::SystemTime;

use toml::Value;
use thiserror::Error;

use abstractions::{
    warn, CPluginFn, Plugin, PluginC, PluginLoadError, PluginManager, PluginOrderedConfig, PluginTopologicalConfig, PluginType, RustPluginFn
};

use crate::UnixContext;

// Определяем типы ошибок, которые могут возникнуть в плагине
#[derive(Debug, Error)]
pub enum PluginConfigError {
    #[error("config read error: {}", error)]
    ReadFileError {
        error: String,
    },

    #[error("config syntax error: {}", error)]
    ParsingError {
        error: String,
    },

    #[error("plugin {} syntax error: {}", plugin_name, error)]
    PluginParsingError {
        plugin_name: String,
        error: String,
    },

    #[error("missing required plugins: {plugins:?}")]
    PluginMissingError {
        plugins: Vec<String>
    }
}

// Добавим новую структуру для отслеживания изменений в конфигурации
#[derive(Debug)]
pub enum PluginConfigChange {
    Add(PluginOrderedConfig),      // Новый плагин для добавления
    Remove(String),                // Имя плагина для удаления
    Reload(PluginOrderedConfig),   // Плагин для перезагрузки
    Disable(String),               // Плагин для отключения
    Enable(PluginOrderedConfig),   // Плагин для включения
    NoChange(String),              // Плагин без изменений
}

pub struct PluginLoader {}

impl PluginLoader {
    pub fn try_load_library(plugin_name: &str) -> Result<Library, PluginLoadError> {
        let library = unsafe {
            Library::new(plugin_name)
                .map_err(|e| PluginLoadError::LibraryLoadError {
                    library_name: plugin_name.to_string(),
                    error: e.to_string(),
                })?
        };

        Ok(library)
    }

    pub fn try_load_plugin(plugin_name: &str, ctx: Arc<UnixContext>) -> Result<PluginType<UnixContext, Library>, PluginLoadError> {
        let match_symbols = [
            "register_rust_plugin",
            "register_c_plugin",
        ];

        for symbol_name in match_symbols {
            // Пробуем загрузить как Rust плагин
            match Self::try_load_rust_plugin(plugin_name, symbol_name, ctx.clone()) {
                Ok(plugin) => return Ok(plugin),
                Err(PluginLoadError::SymbolNotFound { .. }) => {
                    // Символ не найден, пробуем следующий метод или символ
                },
                Err(e) => return Err(e), // Другие ошибки должны быть переданы выше
            }
            
            // Пробуем загрузить как C плагин
            match Self::try_load_c_plugin(plugin_name, symbol_name, ctx.clone()) {
                Ok(plugin) => return Ok(plugin),
                Err(PluginLoadError::SymbolNotFound { .. }) => {
                    // Символ не найден, пробуем следующий символ
                },
                Err(e) => return Err(e), // Другие ошибки должны быть переданы выше
            }
        }

        return Err(PluginLoadError::SymbolNotFound {
            library_name: plugin_name.to_string(),
            symbols_name: match_symbols.join(", "),
        });
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
        plugin_name: &str,
        symbol_name: &str,
        ctx: Arc<UnixContext>,
    ) -> Result<PluginType<UnixContext, Library>, PluginLoadError> {
        let library = match Self::try_load_library(plugin_name) {
            Ok(lib) => lib,
            Err(e) => return Err(e),
        };
        
        let new: Symbol<CPluginFn<UnixContext>> = unsafe {
            let res = library
                .get(symbol_name.as_bytes())
                .map_err(|e| match e {
                    libloading::Error::DlSymUnknown => PluginLoadError::SymbolNotFound {
                        library_name: plugin_name.to_owned(),
                        symbols_name: symbol_name.to_string(),
                    },
                    libloading::Error::DlSym { desc: _ } => PluginLoadError::SymbolNotFound {
                        library_name: plugin_name.to_owned(),
                        symbols_name: symbol_name.to_string(),
                    },
                    e => PluginLoadError::SymbolLoadError {
                        library_name: plugin_name.to_owned(),
                        symbol_name: symbol_name.to_string(),
                        error: e.to_string(),
                    },
                });
    
            match res {
                Ok(f) => f,
                Err(e) => return Err(PluginLoadError::SymbolLoadError {
                    library_name: plugin_name.to_owned(),
                    symbol_name: symbol_name.to_string(),
                    error: e.to_string(),
                }),
            }
        };
    
        // Для C-плагинов нужно преобразовать Arc<UnixContext> в *mut UnixContext
        let ctx_ptr = Arc::into_raw(ctx.clone()) as *mut _;
        let plugin = new(ctx_ptr);
        
        // Восстанавливаем Arc, чтобы не утекла память
        let _ = unsafe { Arc::from_raw(ctx_ptr) };
    
        if plugin.is_null() {
            return Err(PluginLoadError::PluginInitFailed {
                library_name: plugin_name.to_owned(),
                symbol_name: symbol_name.to_string(),
                error: "Plugin init failed. null ptr received".to_string(),
            });
        }
    
        let plugin = unsafe {
            PluginC::from_raw(plugin, ctx)
        };
    
        let plugin = PluginType::C {
            lib: library,
            plugin: plugin,
        };
    
        Ok(plugin)
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
        plugin_name: &str,
        symbol_name: &str,
        ctx: Arc<UnixContext>,
    ) -> Result<PluginType<UnixContext, Library>, PluginLoadError> {
        let library = match Self::try_load_library(plugin_name) {
            Ok(lib) => lib,
            Err(e) => return Err(e),
        };
    
        let new: Symbol<RustPluginFn<UnixContext>> = unsafe {
            let res = library
                .get(symbol_name.as_bytes())
                .map_err(|e| match e {
                    libloading::Error::DlSymUnknown => PluginLoadError::SymbolNotFound {
                        library_name: plugin_name.to_owned(),
                        symbols_name: symbol_name.to_string(),
                    },
                    libloading::Error::DlSym { desc: _ } => PluginLoadError::SymbolNotFound {
                        library_name: plugin_name.to_owned(),
                        symbols_name: symbol_name.to_string(),
                    },
                    e => PluginLoadError::SymbolLoadError {
                        library_name: plugin_name.to_string(),
                        symbol_name: symbol_name.to_string(),
                        error: e.to_string(),
                    },
                });
    
            match res {
                Ok(f) => f,
                Err(e) => return Err(PluginLoadError::SymbolLoadError {
                    library_name: plugin_name.to_owned(),
                    symbol_name: symbol_name.to_string(),
                    error: e.to_string(),
                }),
            }
        };
    
        match new(ctx) {
            Err(e) => return Err(PluginLoadError::PluginInitFailed {
                library_name: plugin_name.to_owned(),
                symbol_name: symbol_name.to_string(),
                error: e.to_string(),
            }),
            Ok(plugin) => Ok(PluginType::Rust {
                lib: library,
                plugin: plugin,
            }),
        }
    }
    
    pub fn load_topological_plugin_config(path: &str) -> Result<Vec<PluginTopologicalConfig>, Box<dyn std::error::Error>> {
        let content = fs::read_to_string(path)?;
        let value: Value = content.parse()?;
    
        let top = value.as_table().ok_or("Top-level TOML is not a table")?;
    
        let mut plugin_configs = Vec::new();
    
        for (section, entry) in top {
            if section == "plugins" {
                if let Value::Table(plugin_sections) = entry {
                    for (plugin_name, plugin_val) in plugin_sections {
                        if let Value::Table(fields) = plugin_val {
                            let path = fields.get("path")
                                .and_then(|v| v.as_str())
                                .ok_or(format!("Plugin '{}' missing valid 'path'", plugin_name))?
                                .to_string();
    
                            let required = fields.get("required")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(false);
    
                            let depend = fields.get("depend")
                                .and_then(|v| v.as_array())
                                .map(|arr| {
                                    arr.iter()
                                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                                        .collect()
                                })
                                .unwrap_or_else(Vec::new);
    
                            plugin_configs.push(PluginTopologicalConfig {
                                name: plugin_name.clone(),
                                path,
                                required,
                                depend,
                            });
                        }
                    }
                }
            }
        }
    
        Ok(plugin_configs)
    }

    // Функция для получения хеш-суммы файла или времени модификации
    pub fn get_file_signature(path: &str) -> Option<String> {
        // Вариант 1: Использовать время модификации файла (проще и быстрее)
        if let Ok(metadata) = fs::metadata(path) {
            if let Ok(modified) = metadata.modified() {
                if let Ok(duration) = modified.duration_since(SystemTime::UNIX_EPOCH) {
                    return Some(duration.as_secs().to_string());
                }
            }
        }
        
        // Вариант 2: Вычислить SHA-256 хеш файла (более надежно, но медленнее)
        // Раскомментируйте, если нужна более точная проверка изменений
        /*
        use sha2::{Sha256, Digest};
        
        let mut file = match fs::File::open(path) {
            Ok(file) => file,
            Err(_) => return None,
        };
        
        let mut hasher = Sha256::new();
        let mut buffer = [0; 1024];
        
        loop {
            let bytes_read = match file.read(&mut buffer) {
                Ok(0) => break, // EOF
                Ok(n) => n,
                Err(_) => return None,
            };
            
            hasher.update(&buffer[..bytes_read]);
        }
        
        let hash = hasher.finalize();
        Some(format!("{:x}", hash))
        */
        
        None
    }

    // Обновленная функция загрузки конфигурации
    pub fn load_ordered_plugin_config(path: &str) -> Result<Vec<PluginOrderedConfig>, PluginConfigError> {
        let content = fs::read_to_string(path).map_err(|op| PluginConfigError::ReadFileError { error: op.to_string() })?;

        let value = content.parse::<Value>().map_err(|op| PluginConfigError::ParsingError { error: op.to_string() })?;
    
        let top = value.as_table().ok_or(PluginConfigError::ParsingError {
            error: "Top-level TOML is not a table".to_string()
        })?;
    
        let mut plugin_configs = Vec::new();
        let required_plugins = ["poll", "logger"];
    
        let mut i = 0;
        for (section, entry) in top {
            if section == "plugins" {
                if let Value::Table(plugin_sections) = entry {
                    for (plugin_name, plugin_val) in plugin_sections {
                        if let Value::Table(fields) = plugin_val {
                            let enable = fields.get("enable")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(true);

                            let path = fields.get("path")
                                .and_then(|v| v.as_str())
                                .ok_or(PluginConfigError::PluginParsingError {
                                    plugin_name: plugin_name.clone(),
                                    error: format!("missing valid 'path'"),
                                })?
                                .to_string();

                            let order = fields.get("order")
                                .and_then(|v| v.as_integer())
                                .unwrap_or(i);

                            let reload = fields.get("reload")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(false);
                            
                            let system = fields.get("system")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(false);
                            
                            i = order + 1;
                            
                            // Получаем хеш-сумму или время модификации файла
                            let file_hash = Self::get_file_signature(&path);
    
                            plugin_configs.push(PluginOrderedConfig {
                                enable,
                                system,
                                name: plugin_name.clone(),
                                path,
                                order,
                                reload,
                                file_hash,
                            });
                        }
                    }
                }
            }
        }

        // ❗ Сравнение: какие required_plugins отсутствуют среди включённых плагинов
        let missing_plugins: Vec<String> = required_plugins
            .iter()
            .filter(|name| {
                !plugin_configs.iter().any(|p| p.name == **name && p.enable)
            })
            .map(|s| s.to_string())
            .collect();

        if !missing_plugins.is_empty() {
            return Err(PluginConfigError::PluginMissingError {
                plugins: missing_plugins,
            });
        }

        for plugin_config in plugin_configs.iter_mut() {
            if required_plugins.contains(&plugin_config.name.as_str()) {
                plugin_config.enable = true;
                plugin_config.system = true;
            }
        }
    
        Ok(plugin_configs)
    }

    // Вспомогательная функция для проверки, изменился ли файл плагина
    fn file_changed(old_config: &PluginOrderedConfig, new_config: &PluginOrderedConfig) -> bool {
        // Если у нас нет хеш-суммы для старого или нового файла, считаем что файл изменился
        if old_config.file_hash.is_none() || new_config.file_hash.is_none() {
            return true;
        }
        
        // Сравниваем хеш-суммы
        old_config.file_hash != new_config.file_hash
    }

    // Обновленная функция анализа изменений
    pub fn analyze_config_changes(
        current_plugins: &[Plugin<UnixContext, Library>],
        new_configs: &[PluginOrderedConfig]
    ) -> Vec<PluginConfigChange> {
        let mut changes = Vec::new();
        
        // Создаем хэш-мапу текущих плагинов для быстрого поиска
        let mut current_map = HashMap::new();
        for plugin in current_plugins {
            current_map.insert(plugin.config.name.clone(), (plugin.config.clone(), &plugin.status));
        }
        
        // Создаем хэш-сет новых плагинов для быстрой проверки
        let new_names: std::collections::HashSet<String> = new_configs
            .iter()
            .map(|config| config.name.clone())
            .collect();
        
        // Проверяем новые конфиги
        for new_config in new_configs {
            if let Some((current_config, plugin_status)) = current_map.get(&new_config.name) {
                // Плагин уже существует
                if !new_config.enable {
                    // Плагин нужно отключить
                    changes.push(PluginConfigChange::Disable(new_config.name.clone()));
                } else if new_config.reload {
                    // Плагин нужно перезагрузить по требованию конфига
                    changes.push(PluginConfigChange::Reload(new_config.clone()));
                } else if current_config.path != new_config.path || 
                          current_config.order != new_config.order {
                    // Изменились важные параметры - перезагружаем
                    changes.push(PluginConfigChange::Reload(new_config.clone()));
                } else if Self::file_changed(current_config, new_config) {
                    // Файл библиотеки изменился - перезагружаем
                    changes.push(PluginConfigChange::Reload(new_config.clone()));
                } else if !matches!(plugin_status, abstractions::PluginStatus::Enable(_)) && new_config.enable {
                    // Плагин отключен, но должен быть включен
                    changes.push(PluginConfigChange::Enable(new_config.clone()));
                } else {
                    // Нет изменений
                    changes.push(PluginConfigChange::NoChange(new_config.name.clone()));
                }
            } else if new_config.enable {
                // Новый плагин, который нужно добавить
                changes.push(PluginConfigChange::Add(new_config.clone()));
            }
        }
        
        // Проверяем удаленные плагины
        for plugin in current_plugins {
            if !new_names.contains(&plugin.config.name) {
                // Плагин был удален из конфигурации
                changes.push(PluginConfigChange::Remove(plugin.config.name.to_string()));
            }
        }
        
        changes
    }
    
    // Метод для применения изменений конфигурации
    pub fn apply_config_changes(
        plugin_manager: &mut PluginManager<UnixContext, Library>,
        ctx: Arc<UnixContext>,
        changes: Vec<PluginConfigChange>,
    ) -> Result<(), PluginLoadError> {
        for change in changes {
            match change {
                PluginConfigChange::Add(config) => {
                    // Загружаем новый плагин
                    match Self::try_load_plugin(&config.path, ctx.clone()) {
                        Ok(plugin_type) => {
                            plugin_manager.get_plugins().push(abstractions::Plugin {
                                config: config.clone(),
                                status: abstractions::PluginStatus::Enable(plugin_type),
                            });
                        },
                        Err(err) => {
                            plugin_manager.get_plugins().push(abstractions::Plugin {
                                config: config.clone(),
                                status: abstractions::PluginStatus::LoadingFailed {
                                    library_name: config.path.clone(),
                                    error: err.to_string(),
                                },
                            });
                            if config.system {
                                return Err(err);
                            } else {
                                warn!(ctx, "Failed to load plugin {}: {}", config.name, err);
                            }
                        }
                    }
                },
                PluginConfigChange::Reload(config) => {
                    // Находим и удаляем старый плагин
                    if let Some(idx) = plugin_manager.get_plugins().iter().position(|p| p.config.name == config.name) {
                        let _ = plugin_manager.get_plugins().remove(idx);
                    }
                    
                    // Загружаем плагин заново
                    match Self::try_load_plugin(&config.path, ctx.clone()) {
                        Ok(plugin_type) => {
                            plugin_manager.get_plugins().push(abstractions::Plugin {
                                config: config,
                                status: abstractions::PluginStatus::Enable(plugin_type),
                            });
                        },
                        Err(err) => {
                            plugin_manager.get_plugins().push(abstractions::Plugin {
                                config: config.clone(),
                                status: abstractions::PluginStatus::LoadingFailed {
                                    library_name: config.path.clone(),
                                    error: err.to_string(),
                                },
                            });
                            if config.system {
                                return Err(err);
                            } else {
                                warn!(ctx, "Failed to load plugin {}: {}", config.name, err);
                            }
                        }
                    }
                },
                PluginConfigChange::Remove(name) => {
                    // Находим и удаляем плагин
                    if let Some(idx) = plugin_manager.get_plugins().iter().position(|p| p.config.name == name) {
                        let plugin = plugin_manager.get_plugins().remove(idx);
                        // Забываем о плагине, чтобы не вызывать его деструктор
                        std::mem::forget(plugin);
                    }
                },
                PluginConfigChange::Disable(name) => {
                    // Находим и отключаем плагин
                    if let Some(idx) = plugin_manager.get_plugins().iter().position(|p| p.config.name == name) {
                        let plugin = &mut plugin_manager.get_plugins()[idx];
                        if let abstractions::PluginStatus::Enable(plugin_type) = std::mem::replace(&mut plugin.status, abstractions::PluginStatus::Unloaded) {
                            plugin.status = abstractions::PluginStatus::Disable(plugin_type);
                        }
                    }
                },
                PluginConfigChange::Enable(config) => {
                    // Находим и включаем плагин
                    if let Some(idx) = plugin_manager.get_plugins().iter().position(|p| p.config.name == config.name) {
                        let plugin = &mut plugin_manager.get_plugins()[idx];
                        if let abstractions::PluginStatus::Disable(plugin_type) = std::mem::replace(&mut plugin.status, abstractions::PluginStatus::Unloaded) {
                            plugin.status = abstractions::PluginStatus::Enable(plugin_type);
                            // Обновляем конфиг
                            plugin.config = config;
                        }
                    }
                },
                PluginConfigChange::NoChange(_) => {
                    // Ничего не делаем
                }
            }
        }
        
        // Пересортировать плагины по порядку
        plugin_manager.get_plugins().sort_by_key(|p| p.config.order);
        
        Ok(())
    }
}
