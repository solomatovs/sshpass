use std::cell::RefCell;
use std::rc::Rc;

use crate::common::Handler;
use crate::unix::{UnixEvent, UnixEventResponse};
use log::trace;


pub struct PtyMiddleware<UnixEvent, UnixEventResponse> {
    next: Option<Rc<RefCell<dyn Handler<UnixEvent, UnixEventResponse>>>>,
}

impl<'a> Handler<UnixEvent<'a>, UnixEventResponse<'a>>  for PtyMiddleware<UnixEvent<'a>, UnixEventResponse<'a>>  {
    fn handle(&mut self, value: UnixEvent<'a>) -> UnixEventResponse<'a> {
        trace!("pty middleware");

        if let UnixEvent::Stdin(_index, buf) = value {
            trace!("stdin utf8: {}", String::from_utf8_lossy(&buf));
            return UnixEventResponse::WriteToPtyMaster(buf);
        }
        
        if let Some(ref next) = self.next {
            return Rc::clone(next).borrow_mut().handle(value);
        }
        
        UnixEventResponse::Unhandled
    }
}
