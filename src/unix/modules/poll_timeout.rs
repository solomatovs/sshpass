use std::cell::RefCell;
use std::rc::Rc;

use crate::common::{AppContext, Handler};
use crate::unix::{UnixEvent, UnixEventResponse};
use log::trace;


pub struct PollTimeoutMiddleware<UnixEvent, UnixEventResponse> {
    next: Option<Rc<RefCell<dyn Handler<UnixEvent, UnixEventResponse>>>>,
    context: AppContext,
}

impl<'a> Handler<UnixEvent<'a>, UnixEventResponse<'a>>  for PollTimeoutMiddleware<UnixEvent<'a>, UnixEventResponse<'a>>  {
    fn handle(&mut self, value: UnixEvent<'a>) -> UnixEventResponse<'a> {
        trace!("poll timeout middleware");

        if let UnixEvent::PollTimeout = value {
            if self.context.shutdown.is_stoped() {
                // break self.context.shutdown.stop_code();
            }
        }
        
        if let Some(ref next) = self.next {
            return Rc::clone(next).borrow_mut().handle(value);
        }
        
        UnixEventResponse::Unhandled
    }
}
