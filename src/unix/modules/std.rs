use std::cell::RefCell;
use std::rc::Rc;

use crate::common::Handler;
use crate::unix::{UnixEvent, UnixEventResponse};
use log::trace;


pub struct StdMiddleware<UnixEvent, UnixEventResponse> {
    next: Option<Rc<RefCell<dyn Handler<UnixEvent, UnixEventResponse>>>>,
}

impl<'a> Handler<UnixEvent<'a>, UnixEventResponse<'a>>  for StdMiddleware<UnixEvent<'a>, UnixEventResponse<'a>>  {
    fn handle(&mut self, value: UnixEvent<'a>) -> UnixEventResponse<'a> {
        trace!("std middleware");

        let mut res = UnixEventResponse::Unhandled;
        
        if let Some(ref next) = self.next {
            res = Rc::clone(next).borrow_mut().handle(value);
        }
        
        res
    }
}
