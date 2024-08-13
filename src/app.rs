pub trait SpecifiedApp<E: std::error::Error> {
    fn process_event(&mut self) -> Result<bool, E>;
    // fn write_stdout(&self, buf: &[u8]);
    // fn write_pty(&self, buf: &[u8]);
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