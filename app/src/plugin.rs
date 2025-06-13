// use libloading::{Library, Symbol};
// use std::fs;
// use std::ops::{Deref, DerefMut};
// use toml::Value;

// use abstractions::{PluginC, RegisterCPluginFn, UnixContext};

// /// Управляемый плагин, который владеет указателем на PluginInterface и соответствующей библиотекой.
// /// Автоматически вызывает инициализацию при создании и освобождение ресурсов при уничтожении.
// pub struct ManagedPlugin {
//     plugin: *mut PluginC<UnixContext>, // Храним указатель на плагин
//     _library: Library,                 // Храним библиотеку, чтобы она не выгрузилась
// }

// impl ManagedPlugin {
//     /// Создает новый экземпляр ManagedPlugin, инициализируя плагин.
//     ///
//     /// # Arguments
//     /// * `plugin_name` - Имя плагина для сообщений об ошибках
//     /// * `ctx` - Контекст приложения
//     ///
//     /// # Returns
//     /// * `Result<Self, String>` - Успешно созданный ManagedPlugin или сообщение об ошибке
//     pub fn new(plugin_name: &str, ctx: &mut UnixContext) -> Result<Self, String> {
//         let library = unsafe {
//             Library::new(plugin_name)
//                 .map_err(|e| format!("Не удалось загрузить библиотеку {}: {}", plugin_name, e))?
//         };

//         // Загружаем функцию new из библиотеки
//         let new: Symbol<RegisterCPluginFn<UnixContext>> = unsafe {
//             library
//                 .get(b"new")
//                 .map_err(|e| format!("Не удалось загрузить символ из {}: {}", plugin_name, e))?
//         };

//         let plugin = new(ctx as *mut UnixContext);
//         if plugin.is_null() {
//             return Err(format!(
//                 "Не удалось создать экземпляр плагина для {}",
//                 plugin_name
//             ));
//         }

//         let managed = ManagedPlugin {
//             plugin,
//             _library: library,
//         };

//         Ok(managed)
//     }

//     /// Обрабатывает событие с помощью плагина
//     ///
//     /// # Arguments
//     /// * `ctx` - Контекст для обработки
//     ///
//     /// # Returns
//     /// * `i32` - Результат обработки
//     pub fn handle(&mut self, ctx: &mut UnixContext) -> i32 {
//         unsafe {
//             // Вызываем handle для перехвата события
//             ((*self.plugin).handle)(self.plugin, ctx as *mut UnixContext)
//         }
//     }

//     /// Освобождает ресурсы плагина.
//     ///
//     /// # Arguments
//     /// * `ctx` - Контекст для обработки
//     ///
//     /// # Returns
//     /// * `i32` - Результат обработки
//     pub fn drop(&mut self, ctx: &mut UnixContext) -> i32 {
//         unsafe {
//             // Вызываем метод free для освобождения ресурсов плагина
//             ((*self.plugin).drop)(self.plugin, ctx as *mut UnixContext)
//         }
//     }
// }

// // Реализуем Deref для доступа к методам PluginInterface
// impl Deref for ManagedPlugin {
//     type Target = PluginC<UnixContext>;

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

// pub struct PluginLoader {}

// impl PluginLoader {
//     /// Загружает плагины из конфигурационного файла
//     ///
//     /// # Arguments
//     /// * `config_path` - Путь к конфигурационному файлу
//     ///
//     /// # Returns
//     /// * `Result<Vec<ManagedPlugin>, String>` - Список загруженных плагинов или сообщение об ошибке
//     pub fn reload_plugins(
//         config_path: &str,
//         ctx: &mut UnixContext,
//     ) -> Result<Vec<ManagedPlugin>, String> {
//         // Читаем конфиг
//         let config_content = fs::read_to_string(config_path)
//             .map_err(|e| format!("Не удалось прочитать config.toml: {}", e))?;

//         let config: Value = config_content
//             .parse::<Value>()
//             .map_err(|e| format!("Ошибка парсинга config.toml: {}", e))?;

//         let plugin_section = config.get("plugins").ok_or_else(|| {
//             "Некорректный формат config.toml: отсутствует секция plugins".to_string()
//         })?;

//         let plugin_order = plugin_section
//             .get("order")
//             .and_then(|o| o.as_array())
//             .ok_or_else(|| {
//                 "Некорректный формат config.toml: отсутствует массив plugins.order".to_string()
//             })?;

//         if plugin_order.is_empty() {
//             return Err("В конфиге не указаны плагины".to_string());
//         }

//         let mut plugins = Vec::new();
//         let mut load_errors = Vec::new();

//         for plugin_value in plugin_order {
//             // Поддержка как простых строк, так и объектов с настройками
//             let (plugin_name, required) = match plugin_value {
//                 Value::String(name) => (name.as_str(), true), // По умолчанию обязательный
//                 Value::Table(table) => {
//                     let name = table.get("name").and_then(|n| n.as_str()).ok_or_else(|| {
//                         "Имя плагина должно быть указано в поле 'name'".to_string()
//                     })?;

//                     let required = table
//                         .get("required")
//                         .and_then(|r| r.as_bool())
//                         .unwrap_or(true); // По умолчанию обязательный

//                     (name, required)
//                 }
//                 _ => {
//                     return Err(
//                         "Элемент массива plugins.order должен быть строкой или таблицей"
//                             .to_string(),
//                     )
//                 }
//             };

//             match ManagedPlugin::new(plugin_name, ctx) {
//                 Ok(managed_plugin) => {
//                     plugins.push(managed_plugin);
//                 }
//                 Err(e) => {
//                     if required {
//                         return Err(format!(
//                             "Не удалось загрузить обязательный плагин {}: {}",
//                             plugin_name, e
//                         ));
//                     } else {
//                         load_errors.push(format!(
//                             "Пропуск необязательного плагина {}: {}",
//                             plugin_name, e
//                         ));
//                     }
//                 }
//             }
//         }

//         // Можно логировать ошибки загрузки необязательных плагинов
//         for error in &load_errors {
//             log::warn!("{}", error);
//         }

//         Ok(plugins)
//     }
// }
