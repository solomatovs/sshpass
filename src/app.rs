pub trait SpecifiedApp<'a, R> {
    fn poll(&'a self, timeout: i32) -> R;
    fn write_stdout(&'a self, buf: &'a [u8]);
    fn write_pty(&'a self, buf: &'a [u8]);
}

// pub trait MainApp {
//     fn handle_stdin(&self, buf: &[u8]);
//     fn handle_ptyin(&self, buf: &[u8]);
// }
// pub struct App<E: std::error::Error> {
//     trans: dyn SpecifiedApp<E>,
// }

// impl<E: std::error::Error> App<E> {}

// impl<T, E> MainApp for App<E>
// where 
//     T: Fn(&dyn MainApp, &[u8]),
//     E: std::error::Error
// {
//     fn run(&mut self) {
//         loop {

//         }
//     }

//     fn handle_stdin(&self, buf: &[u8]) {
//         // if let Some(trans) = self.trans {
//             self.trans.write_pty(buf);
//         // }
//     }

//     fn handle_ptyin(&self, buf: &[u8]) {
//         // if let Some(trans) = self.trans {
//             self.trans.write_stdout(buf);
//         // }
//     }
// }
