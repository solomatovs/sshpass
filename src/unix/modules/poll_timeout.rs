use std::cell::RefCell;
use std::rc::Rc;

use crate::common::{AppContext, Handler};
use crate::unix::{UnixEvent, UnixEventResponse};
use super::EventMiddlewareType;
use log::trace;


pub struct PollTimeoutMiddleware<'a> {
    next: Option<Rc<RefCell<EventMiddlewareType<'a>>>>,
}

impl<'a> PollTimeoutMiddleware<'a>  {
    pub fn new() -> Self {
        Self {
            next: None,
        }
    }
}


impl<'a> Handler<&'a mut AppContext, UnixEvent<'a>, UnixEventResponse<'a>> for PollTimeoutMiddleware<'a>  {
    fn handle(&mut self, context: &'a mut AppContext, value: UnixEvent<'a>) -> UnixEventResponse<'a> {
        trace!("poll timeout middleware");

        if let UnixEvent::PollTimeout = value {
            if context.shutdown.is_stoped() {
                // break self.context.shutdown.stop_code();
            }
        }
        
        if let Some(ref next) = self.next {
            return Rc::clone(next).borrow_mut().handle(context, value);
        }
        
        UnixEventResponse::Unhandled
    }
}
