use std::cell::RefCell;
use std::rc::Rc;

use crate::common::{Handler, AppContext};
use crate::unix::{UnixEvent, UnixEventResponse};
use super::EventMiddlewareNext;
use log::trace;


pub struct StdMiddleware<'a> {
    next: EventMiddlewareNext<'a> ,
}

impl StdMiddleware<'_>  {
    pub fn new() -> Self {
        Self {
            next: None,
        }
    }
}

impl<'a> Handler<&'a mut AppContext, UnixEvent<'a>, UnixEventResponse<'a>> for StdMiddleware<'a>  {
    fn handle(&mut self, context: &'a mut AppContext, value: UnixEvent<'a>) -> UnixEventResponse<'a> {
        trace!("std middleware");

        if let Some(ref next) = self.next {
            return Rc::clone(next).borrow_mut().handle(context, value);
        }
        
        UnixEventResponse::Unhandled
    }
}
