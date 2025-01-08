use crate::common::{AppContext, Handler};
use crate::unix::{UnixEvent, UnixEventResponse};

pub type EventMiddlewareType<'a> =
    dyn Handler<&'a mut AppContext, UnixEvent<'a>, UnixEventResponse<'a>>;
pub type EventMiddlewareArgs<'a> = (&'a mut AppContext, UnixEvent<'a>, UnixEventResponse<'a>);
