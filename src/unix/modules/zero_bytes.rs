use std::cell::RefCell;
use std::rc::Rc;

use crate::common::{AppContext, Handler};
use crate::unix::{UnixEvent, UnixEventResponse};
use super::EventMiddlewareNext;
use log::trace;


pub struct ZeroBytesMiddleware<'a> {
    next: EventMiddlewareNext<'a> ,
}

impl<'a> ZeroBytesMiddleware<'a>  {
    pub fn new() -> Self {
        Self {
            next: None,
        }
    }
}
impl<'a> Handler<&'a mut AppContext, UnixEvent<'a>, UnixEventResponse<'a>> for ZeroBytesMiddleware<'a>  {
    fn handle(&mut self, context: &'a mut AppContext, value: UnixEvent<'a>) -> UnixEventResponse<'a> {
        trace!("zero bytes middleware");

        if let UnixEvent::ReadZeroBytes = value {
            context.shutdown.shutdown_starting(5, Some("reading zero bytes from system event".to_owned()));
        }
        
        if let Some(ref next) = self.next {
            return Rc::clone(next).borrow_mut().handle(context, value);
        }
        
        UnixEventResponse::Unhandled
    }
}
