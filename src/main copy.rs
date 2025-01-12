use nix::pty::openpty;
use nix::unistd::Pid;
use std::process::Stdio;
use std::fs::File;
use std::os::fd::{FromRawFd, OwnedFd};
use std::env;

use std::os::fd::AsRawFd;

use tokio;
use tokio::sync::mpsc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command as TokioCommand;
use tokio::process::Child;

use termios::Termios;
use termios::{
    tcsetattr, BRKINT, CS8, CSIZE, ECHO, ECHONL, ICANON, ICRNL, IEXTEN, IGNBRK, IGNCR, INLCR, ISIG,
    ISTRIP, IXON, OPOST, PARENB, PARMRK, TCSANOW, VMIN, VTIME,
};

#[derive(Debug)]
pub struct Pty {
    pub process: Child,
    fd: OwnedFd,
}

#[derive(Debug)]
pub enum CliError {
    StdIoError(std::io::Error),

    NixErrorno(nix::errno::Errno),

    ArgumentError(String),

    ExitCodeError(i32),

    JoinError(tokio::task::JoinError),
}


impl From<std::io::Error> for CliError {
    fn from(error: std::io::Error) -> Self {
        CliError::StdIoError(error)
    }
}

impl From<nix::errno::Errno> for CliError {
    fn from(error: nix::errno::Errno) -> Self {
        CliError::NixErrorno(error)
    }
}

impl From<i32> for CliError {
    fn from(error: i32) -> Self {
        CliError::ExitCodeError(error)
    }
}
impl From<tokio::task::JoinError> for CliError {
    fn from(error: tokio::task::JoinError) -> Self {
        CliError::JoinError(error)
    }
}

fn set_keypress_mode(termios: &mut Termios) {
    termios.c_iflag &= !(IGNBRK | BRKINT | PARMRK | ISTRIP | INLCR | IGNCR | ICRNL | IXON);
    termios.c_oflag &= !OPOST;
    termios.c_lflag &= !(ECHO | ECHONL | ICANON | ISIG | IEXTEN);
    termios.c_cflag &= !(CSIZE | PARENB);
    termios.c_cflag |= CS8;
    termios.c_cc[VMIN] = 0;
    termios.c_cc[VTIME] = 0;
}

fn set_termios(stdin_fild: i32, termios: &Termios) -> std::io::Result<()> {
    Ok(tcsetattr(stdin_fild, TCSANOW, &termios)?)
}

fn get_termios(stdin_fild: i32) -> std::io::Result<Termios> {
    Termios::from_fd(stdin_fild)
}

fn create_pty(programm: &str, args: Vec<String>) -> Pty {
    let ends = openpty(None, None).expect("openpty failed");
    let master = ends.master;
    let slave = ends.slave;

    let mut cmd = TokioCommand::new(programm);
    let cmd = cmd.args(args);

    let new_follower_stdio = || unsafe { Stdio::from_raw_fd(slave.as_raw_fd()) };
    // cmd.stdin(new_follower_stdio());
    cmd.stdout(new_follower_stdio());
    cmd.stderr(new_follower_stdio());

    match cmd.spawn() {
        Ok(process) => {
            let pty = Pty {
                process,
                fd: master,
            };

            pty
        }
        Err(e) => {
            panic!("Failed to create pty: {}", e);
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), CliError> {
    let mut cmd: Vec<String> = env::args().collect();
    if cmd.len() < 2 {
        eprintln!("Usage: {} <command> [args...]", cmd[0]);
        std::process::exit(1);
    }

    cmd.remove(0);
    let program = cmd.remove(0);
    let mut pty = create_pty(&program, cmd);
    println!("{:#?}", pty);

    let mut ptyout = tokio::fs::File::from(File::from(pty.fd.try_clone().expect("fd clone failed")));
    // let mut ptyin = tokio::fs::File::from(File::from(pty.fd.try_clone().expect("fd clone failed")));

    let mut ptyout_buf: [u8; 1024] = [0; 1024];
    // let mut stdin_buf: [u8; 1024] = [0; 1024];

    let mut stdout = tokio::io::stdout();
    // let mut stdin = tokio::io::stdin();
    // let mut termios = get_termios(stdin.as_raw_fd())?;
    // let c_lflag = termios.c_lflag;
    // let c_iflag = termios.c_iflag;
    // let c_oflag = termios.c_oflag;
    // let c_cflag = termios.c_cflag;
    // let c_cc = termios.c_cc;

    // set_keypress_mode(&mut termios);
    // set_termios(stdin.as_raw_fd(), &termios)?;

    // let mut sigint =tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt()).expect("Failed to create SIGINT handler");
    // let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).expect("Failed to create SIGTERM handler");

    let (shutdown_tx, mut shutdown_rx) = mpsc::channel(1);
    let status = loop {
        tokio::select! {
            res = ptyout.read(ptyout_buf.as_mut()) => match res {
                Ok(0) => {},
                Ok(n) => {
                    if let Err(e) = stdout.write_all(&ptyout_buf[..n]).await {
                        eprintln!("stdout.write_all: {:#?}", e);
                    }

                    ptyout_buf.fill(0);
                }
                Err(e) => {
                    eprintln!("ptyout.read: {:#?}", e);

                    shutdown_tx.send(true).await.expect("shutdown send failed");
                }
            },
            res = shutdown_rx.recv() => match res {
                Some(_) => {
                    break;
                }
                None => {

                }
            }
            // res = stdin.read(stdin_buf.as_mut()) => match res {
            //     Ok(0) => {},
            //     Ok(n) => {
            //         if let Err(e) = ptyin.write_all(&stdin_buf[..n]).await {
            //             eprintln!("ptyin.write_all: {:#?}", e);
            //         }
            //         if let Err(e) = ptyin.flush().await {
            //             eprintln!("ptyin.write_all: {:#?}", e);
            //         }

            //         stdin_buf.fill(0);
            //     }
            //     Err(e) => {
            //         eprintln!("stdin.read: {:#?}", e);
            //     }
            // },
            // _ = sigint.recv() => {
            //     if let Some(id) = pty.process.id() {
            //         let pid = Pid::from_raw(id.try_into().unwrap());
            //         if let Err(e) = nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGINT) {
            //             eprintln!("failed send SIGINT to pid: {}, error: {}", pid, e);
            //         }
            //     }
            // },
            // _ = sigterm.recv() => {
            //     if let Some(id) = pty.process.id() {
            //         let pid = Pid::from_raw(id.try_into().unwrap());
            //         if let Err(e) = nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGTERM) {
            //             eprintln!("failed send SIGTERM to pid: {}, error: {}", pid, e);
            //         }
            //     }
            // },
            // r = pty.process.wait() => match r {
            //     Ok(s) => match s.code() {
            //         None => {
            //             println!("Process terminated by signal");
            //             break Err(CliError::ExitCodeError(-1));
            //         },
            //         Some(0) => {
            //             println!("Ok()");
            //             break Ok(())
            //         },
            //         Some(s) => {
            //             println!("{:#?}", s);
            //             break Err(CliError::ExitCodeError(s))
            //         },
            //     }
            //     Err(e) => {
            //         println!("{:#?}", e);
            //         break Err(CliError::StdIoError(e));
            //     }
            // },
        }
    };

    // termios.c_lflag = c_lflag;
    // termios.c_iflag = c_iflag;
    // termios.c_oflag = c_oflag;
    // termios.c_cflag = c_cflag;
    // termios.c_cc = c_cc;

    // set_termios(stdin.as_raw_fd(), &mut termios)?;
    // let termios = get_termios(stdin.as_raw_fd())?;
    println!("buy\n");

    Ok(status)
}