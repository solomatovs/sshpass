mod app;
use app::App;
use common::UnixContext;


fn main() {
    let mut app = App::new(UnixContext::new(1024));
    
    app.reload_config();

    let (stop_code, stop_message) = {
        while !app.is_stoped() {
            app.processing();
        }

        (app.exit_code(), app.exit_message())
    };

    
    if let Some(message) = stop_message {
        eprintln!("{stop_code}: {message}");
    }

    std::process::exit(stop_code);
}
