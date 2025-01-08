use std::cell::RefCell;
use std::rc::Rc;

use crate::common::Handler;
use crate::unix::{UnixEvent, UnixEventResponse};
use log::trace;


pub struct LoggingMiddleware<UnixEvent, UnixEventResponse> {
    next: Option<Rc<RefCell<dyn Handler<UnixEvent, UnixEventResponse>>>>,
}

impl <'a> LoggingMiddleware<UnixEvent<'a>, UnixEventResponse<'a>>  {
    pub fn new() -> Self {
        Self {
            next: None,
        }
    }
}

impl<'a> Handler<UnixEvent<'a>, UnixEventResponse<'a>>  for LoggingMiddleware<UnixEvent<'a>, UnixEventResponse<'a>>  {
    fn handle(&mut self, value: UnixEvent<'a>) -> UnixEventResponse<'a> {
        trace!("logger middleware: {:?}", value);

        let mut res = UnixEventResponse::Unhandled;
        
        if let Some(ref next) = self.next {
            res = Rc::clone(next).borrow_mut().handle(value);
        }
        
        res
    }
}
