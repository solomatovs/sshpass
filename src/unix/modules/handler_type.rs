use crate::common::{AppContext, Handler};
use crate::unix::{UnixEvent, UnixEventResponse};

use std::cell::RefCell;
use std::rc::Rc;

pub type EventMiddlewareType<'a> =
    dyn Handler<&'a mut AppContext, UnixEvent<'a>, UnixEventResponse<'a>>;
pub type EventMiddlewareNext<'a> = Option<Rc<RefCell<EventMiddlewareType<'a>>>>;
