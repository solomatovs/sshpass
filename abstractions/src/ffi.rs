use std::{fmt::Debug, ops::{Deref, DerefMut}};
use core::clone::Clone;
use nix::libc::c_int;
use thiserror::Error;
use std::any::Any;

pub trait AppContext: Debug {
}

pub trait PluginRust<C: AppContext>: Debug {
    fn handle(&mut self, ctx: &mut C) -> c_int;
    fn free(&mut self, ctx: &mut C) -> c_int;
}

#[repr(C)]
#[derive(Debug, Clone)]
pub struct PluginCPtr<C: AppContext> {
    pub handle: extern "C" fn(this: *mut PluginCPtr<C>, ctx: *mut C) -> c_int,
    pub free: extern "C" fn(this: *mut PluginCPtr<C>, ctx: *mut C) -> c_int,
}

#[derive(Debug, Clone)]
pub struct PluginC<T: AppContext> {
    ptr: *mut PluginCPtr<T>,
}

impl<T: AppContext> PluginC<T> {
    pub unsafe fn from_raw(ptr: *mut PluginCPtr<T>) -> Self {
        Self { ptr }
    }

    pub fn is_null(&self) -> bool {
        self.ptr.is_null()
    }

    pub unsafe fn get_raw(&self) -> *mut PluginCPtr<T> {
        self.ptr
    }

    /// Явно освобождает ресурс (требуется контекст)
    pub fn free(self, ctx: &mut T) {
        unsafe {
            if !self.ptr.is_null() {
                let plugin_mut = &(*self.ptr);
                (plugin_mut.free)(self.ptr, ctx);
            }
        }
    }
}

impl<T: AppContext> Deref for PluginC<T> {
    type Target = PluginCPtr<T>;

    fn deref(&self) -> &Self::Target {
        assert!(!self.ptr.is_null(), "Dereferencing null plugin pointer");
        unsafe { &*self.ptr }
    }
}

impl<T: AppContext> DerefMut for PluginC<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        assert!(!self.ptr.is_null(), "Mutably dereferencing null plugin pointer");
        unsafe { &mut *self.ptr }
    }
}

#[derive(Debug, Error)]
pub enum PluginLoadError {
    #[error("Не удалось загрузить библиотеку {}: {}", library_name, error)]
    LibraryLoadError {
        library_name: String,
        error: String,
    },

    #[error("Не удалось найти символы {symbols_name:?} в библиотеке {library_name}")]
    SymbolNotFound {
        library_name: String,
        symbols_name: String,
    },

    #[error("Обнаружена циклическая зависимость: {node}")]
    CyrcleDependency {
        node: String,
    },

    #[error("Не удалось загрузить плагин {plugin_name}: отсутствуют зависимости: {depend:?}")]
    MissingDependencies {
        plugin_name: String,
        depend: Vec<String>,
    },

    #[error("Не удалось загрузить плагин {plugin_name}: отсутствует конфигурация")]
    MissingConfig {
        plugin_name: String,
    },


    #[error("Ошибка загрузки символа `{symbol_name}` в библиотеке `{library_name}`: {error}")]
    SymbolLoadError {
        library_name: String,
        symbol_name: String,
        error: String,
    },

    #[error("Вызов метода: `{symbol_name}` завершился ошибкой `{error}`.")]
    PluginInitFailed {
        library_name: String,
        symbol_name: String,
        error: String,
    },
}


#[derive(Debug)]
pub enum PluginType<C: AppContext, L> {
    Rust {
        lib: L,
        plugin: Box<dyn PluginRust<C>>,
    },
    C {
        lib: L,
        plugin: PluginC<C>,
    },
}

#[derive(Debug)]
pub enum PluginStatus<C: AppContext, L> {
    Enable(PluginType<C, L>),
    Disable(PluginType<C, L>),
    Unloaded,
    LoadingFailed{
        library_name: String,
        error: String,
    },
}

pub trait PluginConfig: Debug + Any {
    fn name(&self) -> &str;
    fn path(&self) -> &str;
    fn as_any(&self) -> &dyn Any;
}


#[derive(Debug, Clone)]
pub struct PluginOrderedConfig {
    pub enable: bool,
    pub system: bool,
    pub order: i64,
    pub reload: bool,
    pub name: String,
    pub path: String,
    pub file_hash: Option<String>, // Хеш-сумма файла или время модификации
}

impl PluginConfig for PluginOrderedConfig {
    fn name(&self) -> &str {
        &self.name
    }

    fn path(&self) -> &str {
        &self.path
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}


#[derive(Debug, Clone)]
pub struct PluginTopologicalConfig {
    pub name: String,
    pub path: String,
    pub required: bool,
    pub depend: Vec<String>,
}

impl PluginConfig for PluginTopologicalConfig {
    fn name(&self) -> &str {
        &self.name
    }

    fn path(&self) -> &str {
        &self.path
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl Default for PluginTopologicalConfig {
    fn default() -> Self {
        Self {
            name: String::new(),
            path: String::new(),
            required: true,
            depend: Vec::new(),
        }
    }
}

impl PluginTopologicalConfig {
    pub fn new(name: &str, path: &str, required: bool, depend: &[&str]) -> Self {
        Self {
            name: name.to_string(),
            path: path.to_string(),
            required,
            depend: depend.iter().map(|s| s.to_string()).collect(),
        }
    }
}


#[derive(Debug)]
pub struct Plugin<C: AppContext, L> {
    pub status: PluginStatus<C, L>,
    pub config: PluginOrderedConfig,
}

pub struct PluginManager<C: AppContext, L> {
    plugins: Vec<Plugin<C, L>>,
}

impl<C: AppContext, L> PluginManager<C, L> {
    pub fn new() -> Self {
        Self {
            plugins: Vec::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty()
    }

    pub fn is_loaded(&self, name: &str) -> bool {
        self.plugins.iter().any(|p| p.config.name() == name)
    }
    // Добавляем метод для получения списка плагинов
    pub fn get_plugins(&mut self) -> &mut Vec<Plugin<C, L>> {
        &mut self.plugins
    }

    // Метод для проверки, все ли плагины выгружены
    pub fn all_plugins_unloaded(&self) -> bool {
        self.plugins.iter().all(|p| match &p.status {
            PluginStatus::Disable(_) => true,
            _ => false
        })
    }
    pub fn load_plugin_from_ordered_config<F>(
        &mut self,
        mut configs: Vec<PluginOrderedConfig>,
        ctx: &mut C,
        mut loader: F,
    ) -> Result<(), PluginLoadError>
    where
        F: FnMut(&PluginOrderedConfig, &mut C) -> Result<PluginType<C, L>, PluginLoadError>,
    {
        // Сортируем конфиги по order заранее
        configs.sort_by_key(|a| a.order);
    
        for config in configs {
            let idx = self.plugins.iter().position(|p| p.config.name() == config.name());
    
            match (config.enable, idx) {
                (false, Some(i)) => {
                    // Обновляем статус уже загруженного плагина на Disable
                    self.plugins[i] = Plugin {
                        config: config.clone(),
                        status: PluginStatus::Disable(match std::mem::replace(&mut self.plugins[i].status, PluginStatus::Unloaded) {
                            PluginStatus::Enable(pt) => pt,
                            PluginStatus::Disable(pt) => pt, // уже был выключен
                            PluginStatus::Unloaded | PluginStatus::LoadingFailed{..} => {
                                continue; // ничего не делаем
                            }
                        }),
                    };
                }
                (false, None) => {
                    // Добавляем новый отключённый плагин
                    self.plugins.push(Plugin {
                        config: config.clone(),
                        status: PluginStatus::Unloaded,
                    });
                }
                (true, Some(i)) => {
                    if config.reload {
                        // Удаляем старый плагин перед перезагрузкой
                        self.plugins.remove(i);
                        match loader(&config, ctx) {
                            Ok(plugin_type) => {
                                self.plugins.push(Plugin {
                                    config: config.clone(),
                                    status: PluginStatus::Enable(plugin_type),
                                });
                            }
                            Err(err) => {
                                self.plugins.push(Plugin {
                                    config: config.clone(),
                                    status: PluginStatus::LoadingFailed{
                                        library_name: config.path.clone(),
                                        error: err.to_string(),
                                    },
                                });
                            }
                        }
                    } else {
                        // Плагин уже есть, enable == true, reload == false → не трогаем
                    }
                }
                (true, None) => {
                    // Новый плагин, нужно загрузить
                    match loader(&config, ctx) {
                        Ok(plugin_type) => {
                            self.plugins.push(Plugin {
                                config: config.clone(),
                                status: PluginStatus::Enable(plugin_type),
                            });
                        }
                        Err(err) => {
                            self.plugins.push(Plugin {
                                config: config.clone(),
                                status: PluginStatus::LoadingFailed {
                                    library_name: config.path.clone(),
                                    error: err.to_string(),
                                },
                            });
                        }
                    }
                }
            }
        }
    
        // Пересортировать плагины по порядку
        self.plugins.sort_by_key(|p| {
            if let Some(ordered) = p.config.as_any().downcast_ref::<PluginOrderedConfig>() {
                ordered.order
            } else {
                i64::MAX // если не PluginOrderedConfig, ставим в конец
            }
        });
    
        Ok(())
    }
    
    // pub fn load_plugin_from_topological_config<F>(
    //     &mut self,
    //     configs: Vec<PluginTopologicalConfig>,
    //     ctx: &mut C,
    //     mut loader: F,
    // ) -> Result<(), PluginLoadError>
    // where
    //     F: FnMut(&PluginTopologicalConfig, &mut C) -> Result<PluginType<C, L>, PluginLoadError>,
    // {
    //     let mut name_to_config: HashMap<String, &PluginTopologicalConfig> = HashMap::new();
    //     for config in &configs {
    //         name_to_config.insert(config.name.clone(), config);
    //     }

    //     let sorted = Self::topological_sort(&configs).map_err(|e| PluginLoadError::CyrcleDependency {
    //         node: e.to_string(),
    //     })?;

    //     let mut loaded = HashSet::new();

    //     for name in sorted {
    //         if self.is_loaded(&name) {
    //             loaded.insert(name.clone());
    //             continue;
    //         }

    //         let config = name_to_config.get(&name).ok_or_else(||
    //             PluginLoadError::MissingConfig {
    //                 plugin_name: name.clone(),
    //             }
    //         )?;

    //         // Проверим, что все зависимости уже загружены
    //         let missing: Vec<_> = config
    //             .depend
    //             .iter()
    //             .filter(|dep| !loaded.contains(*dep))
    //             .cloned()
    //             .collect();

    //         if !missing.is_empty() {
    //             if config.required {
    //                 return Err(PluginLoadError::MissingDependencies {
    //                     plugin_name: name,
    //                     depend: missing,
    //                 });
    //             } else {
    //                 eprintln!(
    //                     "Skipping optional plugin '{}': missing dependencies: {:?}",
    //                     name, missing
    //                 );
    //                 continue;
    //             }
    //         }

    //         // Загружаем
    //         match loader(config, ctx) {
    //             Ok(plugin_type) => {
    //                 let plugin = Plugin {
    //                     config: Box::new((*config).clone(),
    //                     plugin: PluginStatus::Enable(plugin_type),
    //                 };
    //                 self.plugins.push(plugin);
    //                 loaded.insert(name.clone());
    //             }
    //             Err(err) => {
    //                 if config.required {
    //                     return Err(err);
    //                 } else {
    //                     let plugin = Plugin {
    //                         config: Box::new((*config).clone()),
    //                         plugin: PluginStatus::LoadingFailed(err.to_string()),
    //                     };
    //                     self.plugins.push(plugin);
    //                     loaded.insert(name.clone());
    //                 }
    //             }
    //         }
    //     }

    //     Ok(())
    // }

    // fn topological_sort(configs: &[PluginTopologicalConfig]) -> Result<Vec<String>, String> {
    //     let mut graph: HashMap<String, Vec<String>> = HashMap::new();
    //     for cfg in configs {
    //         graph
    //             .entry(cfg.name.clone())
    //             .or_default()
    //             .extend(cfg.depend.clone());
    //     }
    
    //     let mut visited = HashSet::new();
    //     let mut temp_mark = HashSet::new();
    //     let mut result = Vec::new();
    
    //     fn visit(
    //         node: &str,
    //         graph: &HashMap<String, Vec<String>>,
    //         visited: &mut HashSet<String>,
    //         temp_mark: &mut HashSet<String>,
    //         result: &mut Vec<String>,
    //     ) -> Result<(), String> {
    //         if visited.contains(node) {
    //             return Ok(());
    //         }
    //         if temp_mark.contains(node) {
    //             return Err(format!("Cycle detected at '{}'", node));
    //         }
    
    //         temp_mark.insert(node.to_string());
    
    //         if let Some(deps) = graph.get(node) {
    //             for dep in deps {
    //                 visit(dep, graph, visited, temp_mark, result)?;
    //             }
    //         }
    
    //         temp_mark.remove(node);
    //         visited.insert(node.to_string());
    //         result.push(node.to_string());
    
    //         Ok(())
    //     }
    
    //     for name in graph.keys() {
    //         visit(name, &graph, &mut visited, &mut temp_mark, &mut result)?;
    //     }
    
    //     Ok(result)
    // }
}

// Определение типа для функции создания плагина
pub type RustPluginFn<C> = extern "Rust" fn(ctx: &mut C) -> Result<Box<dyn PluginRust<C>>, String>;
pub type CPluginFn<C> = extern "C" fn(ctx: *mut C) -> *mut PluginCPtr<C>;
