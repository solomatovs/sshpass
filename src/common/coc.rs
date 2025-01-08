
pub trait Handler<C, V, R> {
    // fn next(&mut self, next: Box<dyn Handler<V, R>>);
    fn handle(&mut self, context: C, value: V) -> R;
}
