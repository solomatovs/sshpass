mod app;
use app::App;
use common::UnixContext;


fn main() {
    let mut app = App::new(UnixContext::new(1024));
    
    app.reload_config();

    let (stop_code, _stop_message) = {
        while !app.is_stoped() {
            app.processing();
        }

        (app.exit_code(), app.exit_message())
    };

    std::process::exit(stop_code);
}
