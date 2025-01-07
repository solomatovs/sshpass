// use crate::unix::middleware::Middleware;
// use crate::unix::unix_event::{UnixEvent, UnixEventResponse};

// use nix::sys::signal::Signal;

// use log::trace;

// pub struct SignalFilterMiddleware;

// impl Middleware for SignalFilterMiddleware {
//     fn handle(
//         &self,
//         event: UnixEvent,
//         next: &dyn Fn(UnixEvent) -> UnixEventResponse,
//     ) -> UnixEventResponse {
//         if let UnixEvent::Signal(_, sig, _) = event {
//             if sig == Signal::SIGINT {
//                 trace!("SIGINT received, stopping application.");
//                 return UnixEventResponse::Shutdown; // Завершаем обработку цепочки
//             }
//         }
//         next(event) // Передаём событие дальше
//     }
// }