use libloading::{Library};


use abstractions::{warn, info, PluginManager};
use common::UnixContext;
use common::plugin::{PluginLoader};


pub struct App {
    context: UnixContext,
    plugin_manager: PluginManager<UnixContext, Library>,
}

impl App {
    pub fn new(context: UnixContext) -> Self {
        Self {
            context,
            plugin_manager: PluginManager::new(),
        }
    }

    pub fn reload_config(&mut self) {
        let plugin_configs = match PluginLoader::load_ordered_plugin_config("config.toml") {
            Ok(x) => x,
            Err(e) => {
                warn!(self.context, "{}", e.to_string());
                return;
            }
        };
    
        // Анализируем изменения в конфигурации
        let changes = PluginLoader::analyze_config_changes(
            self.plugin_manager.get_plugins(),
            &plugin_configs
        );
        
        // Применяем только необходимые изменения
        let res = PluginLoader::apply_config_changes(
            &mut self.plugin_manager,
            &mut self.context,
            changes,
        );
    
        if let Err(e) = res {
            warn!(self.context, "{}", e.to_string());
        }
    }    

    pub fn exit_code(&self) -> i32 {
        self.context.shutdown.get_code()
    }

    pub fn is_stoped(&self) -> bool {
        self.context.shutdown.is_stoped()
    }

    pub fn exit_message(&self) -> Option<String> {
        self.context.shutdown.get_message()
    }

    pub fn processing(&mut self) {
        // Получаем список плагинов, которые нужно обработать

        for plugin in self.plugin_manager.get_plugins() {
            // Вызываем handle для плагина
            let result = match &mut plugin.status {
                abstractions::PluginStatus::Enable(plugin_type) => {
                    match plugin_type {
                        abstractions::PluginType::Rust { plugin, .. } => {
                            plugin.handle(&mut self.context)
                        },
                        abstractions::PluginType::C { plugin, .. } => {
                            unsafe {
                                (plugin.handle)(plugin.get_raw(), &mut self.context)
                            }
                        }
                    }
                },
                abstractions::PluginStatus::Unloaded => 1,          // Плагин выгружен
                abstractions::PluginStatus::Disable(_) => 0,        // Плагин отключен
                abstractions::PluginStatus::LoadingFailed{..} => 0,  // Плагин не загрузился
            };
            
            // Если плагин вернул 1, это значит, что он готов к выгрузке
            if result == 1 {
                // Вызываем free для плагина
                match &mut plugin.status {
                    abstractions::PluginStatus::Enable(plugin_type) => {
                        match plugin_type {
                            abstractions::PluginType::Rust { plugin, .. } => {
                                plugin.free(&mut self.context);
                            },
                            abstractions::PluginType::C { plugin, .. } => {
                                unsafe {
                                    (plugin.free)(plugin.get_raw(), &mut self.context);
                                }
                            }
                        }
                    },
                    _ => {} // Для отключенных или не загруженных плагинов ничего не делаем
                }
                
                // Отмечаем плагин как выгруженный
                plugin.status = abstractions::PluginStatus::Unloaded;
                
                // Не увеличиваем i, так как следующий элемент теперь имеет тот же индекс
                continue;
            }
        }

        // Проверяем, нужно ли перезагрузить конфигурацию
        if self.context.reload_config {
            info!(self.context, "Reloading configuration due to file change");
            self.reload_config();
            self.context.reload_config = false; // Сбрасываем флаг
        }
    }
}