use bytes::Buf;

#[derive(Debug)]
pub enum AppError {}

pub enum AppMessage {
    Stdin(Box<dyn Buf>),
    Stdout(Box<dyn Buf>),
    Pty(Box<dyn Buf>),
}

pub trait AppTransport {
    fn write_stdout(&self, buf: &[u8]);
    fn write_pty(&self, buf: &[u8]);
}

pub trait AppHandler<'t> {

    fn init<A: AppTransport>(&mut self, trans: &'t A);

    fn handle_stdin(&self, buf: &[u8]);

    fn handle_ptyin(&self, buf: &[u8]);
}
pub struct App<'t>
{
    trans: Option<&'t dyn AppTransport>,
}

impl<'t> App<'t>
{
    pub fn new() -> Result<Self, AppError> {
        Ok(Self {
            trans: None,
        })
    }
}

impl<'t> AppHandler<'t> for App<'t> {
    fn init<A: AppTransport>(self: &mut Self, trans: &'t A) {
        self.trans = Some(trans);
    }

    fn handle_stdin(&self, buf: &[u8]) {
        if let Some(trans) = self.trans {
            trans.write_pty(buf);
        }
    }

    fn handle_ptyin(&self, buf: &[u8]) {
        if let Some(trans) = self.trans {
            trans.write_stdout(buf);
        }
    }
}
