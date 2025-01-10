use std::cell::RefCell;
use std::rc::Rc;

use super::EventMiddlewareNext;
use crate::common::{AppContext, Handler};
use crate::unix::{UnixEvent, UnixEventResponse};
use log::trace;

pub struct LoggingMiddleware<'a> {
    next: EventMiddlewareNext<'a>,
}

impl LoggingMiddleware<'_> {
    pub fn new() -> Self {
        Self { next: None }
    }
}

impl<'a> Handler<&'a mut AppContext, UnixEvent<'a>, UnixEventResponse<'a>>
    for LoggingMiddleware<'a>
{
    fn handle(
        &mut self,
        context: &'a mut AppContext,
        value: UnixEvent<'a>,
    ) -> UnixEventResponse<'a> {
        trace!("logger middleware: {:?}", value);

        if let Some(ref next) = self.next {
            return Rc::clone(next).borrow_mut().handle(context, value);
        }

        UnixEventResponse::Unhandled
    }
}
