use std::sync::Arc;
use libloading::Library;

use abstractions::{warn, info, PluginManager, PluginStatus, PluginType};
use common::UnixContext;
use common::plugin::PluginLoader;

pub struct App {
    // Теперь храним Arc<UnixContext> вместо UnixContext
    context: Arc<UnixContext>,
    plugin_manager: PluginManager<UnixContext, Library>,
}

impl App {
    pub fn new(context: UnixContext) -> Self {
        // Оборачиваем контекст в Arc при создании App
        let context_arc = Arc::new(context);
        
        Self {
            // Передаем клон Arc в plugin_manager
            plugin_manager: PluginManager::new(context_arc.clone()),
            context: context_arc,
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
        // Передаем Arc<UnixContext> вместо &mut UnixContext
        let res = PluginLoader::apply_config_changes(
            &mut self.plugin_manager,
            self.context.clone(),
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
                PluginStatus::Enable(plugin_type) => {
                    match plugin_type {
                        PluginType::Rust {plugin, .. } => {
                            // Теперь handle принимает &self вместо &mut self
                            plugin.handle()
                        },
                        PluginType::C { plugin, .. } => {
                            unsafe {
                                // Для C-плагинов используем внутренний контекст плагина
                                (plugin.handle)(plugin.get_raw(), std::ptr::null_mut())
                            }
                        }
                    }
                },
                PluginStatus::Unloaded => 1,          // Плагин выгружен
                PluginStatus::Disable(_) => 0,        // Плагин отключен
                PluginStatus::LoadingFailed{..} => 0,  // Плагин не загрузился
            };
            
            // Если плагин вернул 1, это значит, что он готов к выгрузке
            if result == 1 {
                // Больше не нужно явно вызывать free, это будет сделано в Drop
                // Просто отмечаем плагин как выгруженный
                plugin.status = PluginStatus::Unloaded;
                continue;
            }
        }

        // Проверяем, нужно ли перезагрузить конфигурацию
        if self.context.reload_config.check_and_reset() {
            info!(self.context, "Reloading configuration due to file change");
            self.reload_config();
        }
    }
}
