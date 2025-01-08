use std::cell::RefCell;

#[derive(Debug)]
pub struct MiddlewareBase<V, R> {
    inner: fn(V) -> R,
}

impl<V, R> MiddlewareBase<V, R> {
    pub fn handle(&self, value: V) -> R {
        (self.inner)(value)
    }
}

use std::rc::Rc;

type Middleware<V, R> = Box<dyn FnMut(V, Next<V, R>) -> R>;
type MiddlewareRefCell<V, R> = RefCell<Middleware<V, R>>;
type ListOfMiddlewares<V, R> = Vec<MiddlewareRefCell<V, R>>;
type SharableListOfMiddlewares<V, R> = Rc<RefCell<ListOfMiddlewares<V, R>>>;

pub struct Manager<V, R> {
    list: SharableListOfMiddlewares<V, R>,
}

impl<V: 'static, R: 'static> Manager<V, R> {
    /// Create new instance
    pub fn new() -> Self {
        Self {
            list: Rc::default(),
        }
    }

    pub fn last<M>(last: M) -> Self
    where
        M: FnMut(V, Next<V, R>) -> R + 'static,
    {
        let s = Self::new();
        s.next(last);

        s
    }

    /// Start processing the value
    pub fn send(&self, value: V) -> R {
        let total = self.list.borrow().len();

        let qq = Rc::clone(&self.list);
        let next = Next {
            list: Rc::clone(&qq),
            next: total - 1,
        };

        let lock = self.list.borrow();
        let mut callback = lock.last().unwrap().borrow_mut();
        (callback)(value, next)
    }

    pub fn next<M>(&self, m: M) -> &Self
    where
        M: FnMut(V, Next<V, R>) -> R + 'static,
    {
        let list = Rc::clone(&self.list);
        let mut lock = list.borrow_mut();
        lock.push(RefCell::new(Box::new(m)));

        self
    }
}

impl<V: 'static, R: 'static> Default for Manager<V, R> {
    fn default() -> Self {
        Self::new()
    }
}

pub struct Next<V, R> {
    list: SharableListOfMiddlewares<V, R>,
    next: usize,
}

impl<V, R> Next<V, R> {
    pub fn call(mut self, value: V) -> R {
        let list = Rc::clone(&self.list);
        self.next -= 1;
        if let Some(next) = list.borrow().get(self.next) {
            let mut callback = next.borrow_mut();
            return callback(value, self);
        }
        panic!("There must be a default")
    }
}
