use nix::pty::openpty;
use nix::unistd::Pid;
use std::process::Stdio;
use std::fs::File;
use std::os::fd::OwnedFd;
use std::env;

use tokio;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command as TokioCommand;
use tokio::process::Child;

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

fn create_pty(programm: &str, args: Vec<String>) -> Pty {
    let ends = openpty(None, None).expect("openpty failed");
    let master = ends.master;
    let slave = ends.slave;

    let mut cmd = TokioCommand::new(programm);
    let cmd = cmd.args(args);

    cmd.stdin(Stdio::from(slave.try_clone().unwrap()));
    cmd.stdout(Stdio::from(slave.try_clone().unwrap()));
    cmd.stderr(Stdio::from(slave.try_clone().unwrap()));

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
    let mut ptyin = tokio::fs::File::from(File::from(pty.fd.try_clone().expect("fd clone failed")));

    let mut ptyout_buf: [u8; 1024] = [0; 1024];
    let mut stdin_buf: [u8; 1024] = [0; 1024];

    let mut stdin = tokio::io::stdin();
    let mut stdout = tokio::io::stdout();

    let mut sigint =tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt()).expect("Failed to create SIGINT handler");
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).expect("Failed to create SIGTERM handler");

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
                }
            },
            res = stdin.read(stdin_buf.as_mut()) => match res {
                Ok(0) => {},
                Ok(n) => {
                    if let Err(e) = ptyin.write_all(&stdin_buf[..n]).await {
                        eprintln!("ptyin.write_all: {:#?}", e);
                    }
                    if let Err(e) = ptyin.flush().await {
                        eprintln!("ptyin.write_all: {:#?}", e);
                    }

                    stdin_buf.fill(0);
                }
                Err(e) => {
                    eprintln!("stdin.read: {:#?}", e);
                }
            },
            _ = sigint.recv() => {
                if let Some(id) = pty.process.id() {
                    let pid = Pid::from_raw(id.try_into().unwrap());
                    if let Err(e) = nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGINT) {
                        eprintln!("failed send SIGINT to pid: {}, error: {}", pid, e);
                    }
                }
            },
            _ = sigterm.recv() => {
                if let Some(id) = pty.process.id() {
                    let pid = Pid::from_raw(id.try_into().unwrap());
                    if let Err(e) = nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGTERM) {
                        eprintln!("failed send SIGTERM to pid: {}, error: {}", pid, e);
                    }
                }
            },
            r = pty.process.wait() => match r {
                Ok(s) => match s.code() {
                    None => {
                        println!("Process terminated by signal");
                        break Err(CliError::ExitCodeError(-1));
                    },
                    Some(0) => {
                        println!("Ok()");
                        break Ok(())
                    },
                    Some(s) => {
                        println!("{:#?}", s);
                        break Err(CliError::ExitCodeError(s))
                    },
                }
                Err(e) => {
                    println!("{:#?}", e);
                    break Err(CliError::StdIoError(e));
                }
            },
        }
    };

    status
}