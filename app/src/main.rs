#![feature(allocator_api)]

use log::info;

mod app;
mod plugin;
use app::App;
use abstractions::UnixContext;
use common::init_log::init_log;


fn main() {
    init_log();

    let mut app = App::new(UnixContext::new(1024));

    let (stop_code, stop_message) = {
        while !app.is_stoped() {
            app.processing();
        }

        (app.exit_code(), app.exit_message())
    };

    info!("exit code {stop_code} message {stop_message:?}");

    std::process::exit(stop_code);
}

