// use crate::unix::middleware::Ware;
// use crate::unix::unix_event::{UnixEvent, UnixEventResponse};

// use log::trace;


// pub struct LoggingMiddleware;

// impl Ware for LoggingMiddleware {
//     fn handle(
//         &self,
//         event: UnixEvent,
//         next: &dyn Fn(UnixEvent) -> UnixEventResponse,
//     ) -> UnixEventResponse {
//         trace!("Logging event: {:?}", event);
//         next(event) // Передаём событие дальше
//     }
// }