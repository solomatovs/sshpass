use std::cell::RefCell;
use std::rc::Rc;

use crate::common::{AppContext, Handler};
use crate::unix::{UnixEvent, UnixEventResponse};
use log::trace;


pub struct ZeroBytesMiddleware<UnixEvent, UnixEventResponse> {
    next: Option<Rc<RefCell<dyn Handler<UnixEvent, UnixEventResponse>>>>,
    context: AppContext,
}

impl<'a> Handler<UnixEvent<'a>, UnixEventResponse<'a>>  for ZeroBytesMiddleware<UnixEvent<'a>, UnixEventResponse<'a>>  {
    fn handle(&mut self, value: UnixEvent<'a>) -> UnixEventResponse<'a> {
        trace!("zero bytes middleware");

        if let UnixEvent::ReadZeroBytes = value {
            self.context.shutdown.shutdown_starting(5, Some("reading zero bytes from system event".to_owned()));
        }
        
        if let Some(ref next) = self.next {
            return Rc::clone(next).borrow_mut().handle(value);
        }
        
        UnixEventResponse::Unhandled
    }
}
