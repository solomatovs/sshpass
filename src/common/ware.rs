use std::cell::{RefCell, RefMut};

pub type WareArg<'a, T> = RefMut<'a, T>;
pub type WareArgBox<T> = Box<dyn Fn(WareArg<T>)>;
pub type WareArgVec<T> = Vec<WareArgBox<T>>;

/// A middleware chain.
pub struct Ware<T> {
    pub fns: WareArgVec<T>,
}

impl<T> Ware<T> {
    pub fn new() -> Ware<T> {
        let vec = Vec::new();
        Ware { fns: vec }
    }

    pub fn wrap(&mut self, func: WareArgBox<T>) {
        self.fns.push(func);
    }


    pub fn run(&self, arg: T) -> T {
        let ware_arg = RefCell::new(arg);

        self.fns.iter().for_each(|func| {
            let res = func(ware_arg.borrow_mut());
            res
        });

        ware_arg.into_inner()
    }
}

