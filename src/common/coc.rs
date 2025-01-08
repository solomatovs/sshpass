
pub trait Handler<V, R> {
    // fn next(&mut self, next: Box<dyn Handler<V, R>>);
    fn handle(&mut self, value: V) -> R;
}
