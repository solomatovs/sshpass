use std::cell::RefCell;
use std::rc::Rc;

use crate::common::{Handler, AppContext};
use crate::unix::{UnixEvent, UnixEventResponse};
use super::EventMiddlewareType;
use log::trace;


pub struct PtyMiddleware<'a> {
    next: Option<Rc<RefCell<EventMiddlewareType<'a>>>>,
}

impl<'a> PtyMiddleware<'a>  {
    pub fn new() -> Self {
        Self {
            next: None,
        }
    }
}

impl<'a> Handler<&'a mut AppContext, UnixEvent<'a>, UnixEventResponse<'a>> for PtyMiddleware<'a>  {
    fn handle(&mut self, context: &'a mut AppContext, value: UnixEvent<'a>) -> UnixEventResponse<'a> {
        trace!("pty middleware");

        if let UnixEvent::Stdin(_index, buf) = value {
            trace!("stdin utf8: {}", String::from_utf8_lossy(&buf));
            return UnixEventResponse::WriteToPtyMaster(buf);
        }
        
        if let Some(ref next) = self.next {
            return Rc::clone(next).borrow_mut().handle(context, value);
        }
        
        UnixEventResponse::Unhandled
    }
}
