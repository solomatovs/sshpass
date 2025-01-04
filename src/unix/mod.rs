mod fds;
mod unix_app;
mod unix_error;
mod unix_event;

pub use unix_app::UnixApp;
pub use unix_error::UnixError;
pub use unix_event::UnixEvent;

// pub unsafe fn get_mut_from_immutble<T>(reference: &T) -> &mut T {
//     let const_ptr = reference as *const T;
//     let mut_ptr = const_ptr as *mut T;
//     &mut *mut_ptr
// }