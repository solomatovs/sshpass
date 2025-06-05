pub trait PollErrorHandler<C, E> {
    fn handle(&mut self, app: &mut C, err: E);
}

pub trait PollReventHandler<C> {
    fn handle(&mut self, app: &mut C, number_events: i32);

    fn reg_next(&mut self, handler: Box<dyn FdEventHandler<C>>);
}

pub trait ReadHandler<C> {
    fn read(&mut self, app: &mut C, pollfd_index: usize) -> bool;
}

pub trait PollOutHandler<C> {
    fn write(&mut self, app: &mut C, pollfd_index: usize) -> bool;
}

pub trait PollErrHandler<C> {
    fn handle(&mut self, app: &mut C, pollfd_index: usize);
}

pub trait PollNvalHandler<C> {
    fn handle(&mut self, app: &mut C, pollfd_index: usize);
}

pub trait PollHupHandler<C> {
    fn handle(&mut self, app: &mut C, pollfd_index: usize);
}

pub trait FdEventHandler<C> {
    fn handle(&mut self, app: &mut C, res: i32);

    fn reg_next(&mut self, next: Box<dyn FdEventHandler<C>>);
}

