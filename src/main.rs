use nix::pty::openpty;
use nix::unistd::Pid;
use tokio::sync::mpsc::error::SendError;
use std::os::unix::process::CommandExt;
use std::process::Stdio;
use std::fs::File;
use std::io::{StdinLock, StdoutLock, StderrLock};
use std::os::fd::{FromRawFd, OwnedFd};
use std::env;
use nix::unistd::{fork, ForkResult, dup2};
use nix::sys::wait::{waitpid, WaitPidFlag, WaitStatus};
use nix::sys::signal::{signal, kill, SIGTERM, SIGINT};

use std::os::fd::AsRawFd;

use tokio;
use tokio::sync::mpsc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command as TokioCommand;
use tokio::process::Child;
// use tokio::signal;

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

    Ok,

    ShutdownSendError,

    ChildTerminatedBySignal,
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
    cmd.stdin(new_follower_stdio());
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

pub fn waitpio(child: Pid) -> Result<bool, CliError> {
    // eprintln!("{:?}: {:?}: waitpid before", std::thread::current().id(), std::time::SystemTime::now());
    match waitpid(child, None) {
        Err(nix::errno::Errno::ECHILD) => {
            // eprintln!("{:?}: {:?}: waitpid error: {:#?}", std::thread::current().id(), std::time::SystemTime::now(), nix::errno::Errno::ECHILD);
            return Ok(true);
        }
        Err(nix::errno::Errno::EINTR) => {
            // eprintln!("{:?}: {:?}: waitpid error: {:#?}", std::thread::current().id(), std::time::SystemTime::now(), nix::errno::Errno::EINTR);
            return Ok(false);
        }
        Err(e) => {
            // eprintln!("{:?}: {:?}: waitpid error: {:#?}", std::thread::current().id(), std::time::SystemTime::now(), e);
            return Err(e.into());
        }
        Ok(WaitStatus::Exited(_pid, status)) => {
            // eprintln!("{:?}: {:?}: WaitStatus::Exited(pid: {:?}, status: {:?}", std::thread::current().id(), std::time::SystemTime::now(), _pid, status);
            if status != 0 {
                return Err(CliError::ExitCodeError(status));
            } else {
                return Ok(true);
            }
        }
        Ok(WaitStatus::Signaled(pid, sig, _dumped)) => {
            // eprintln!("{:?}: {:?}: WaitStatus::Signaled(pid: {:?}, sig: {:?}, dumped: {:?})", std::thread::current().id(), std::time::SystemTime::now(), pid, sig, _dumped);
            return  Ok(false);
        }
        Ok(WaitStatus::Stopped(pid, sig)) => {
            // eprintln!("{:?}: {:?}: WaitStatus::Stopped(pid: {:?}, sig: {:?})", std::thread::current().id(), std::time::SystemTime::now(), pid, sig);
            return Ok(false)
        }
        Ok(WaitStatus::StillAlive) => {
            // eprintln!("{:?}: {:?}: WaitStatus::StillAlive", std::thread::current().id(), std::time::SystemTime::now());
            return Ok(false)
        }
        Ok(WaitStatus::Continued(pid)) => {
            // eprintln!("{:?}: {:?}: WaitStatus::Continued(pid: {:?})", std::thread::current().id(), std::time::SystemTime::now(), pid);
            return Ok(false)
        }
        Ok(WaitStatus::PtraceEvent(pid, sig, c)) => {
            // eprintln!("{:?}: {:?}: WaitStatus::PtraceEvent(pid: {:?}, sig: {:?}, c: {:?})", std::thread::current().id(), std::time::SystemTime::now(), pid, sig, c);
            return Ok(false)
        }
        Ok(WaitStatus::PtraceSyscall(pid)) => {
            // eprintln!("{:?}: {:?}: WaitStatus::PtraceSyscall(pid: {:?})", std::thread::current().id(), std::time::SystemTime::now(), pid);
            return Ok(false)
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

    // Создаем псевдотерминал (PTY)
    let pty = openpty(None, None).expect("Failed to open PTY");
    let slave = pty.slave;

    let status = match unsafe { fork() } {
        Ok(ForkResult::Child) => {
            // эта программа исполняется только в дочернем процессе
            // родительский процесс в это же время выполняется и что то делает

            // Подключаем slave конец псевдотерминала к стандартным потокам ввода/вывода            
            // dup2(slave.as_raw_fd(), libc::STDIN_FILENO.as_raw_fd()).expect("Failed to dup2 stdin");
            // dup2(slave.as_raw_fd(), libc::STDOUT_FILENO.as_raw_fd()).expect("Failed to dup2 stdout");
            // dup2(slave.as_raw_fd(), libc::STDERR_FILENO.as_raw_fd()).expect("Failed to dup2 stderr");

            cmd.remove(0);
            let program = cmd.remove(0);
            let args = cmd;
        
            // lambda функция для перенаправления stdio
            let new_follower_stdio = || unsafe {
                Stdio::from_raw_fd(slave.as_raw_fd())
            };

            // ДАЛЬНЕЙШИЙ ЗАПУСК БЕЗ FORK ПРОЦЕССА
            // это означает что дочерний процесс не будет еще раз разделятся
            // Command будет выполняться под pid этого дочернего процесса и буквально станет им
            // осуществляется всё это с помощью exec()
            let err = std::process::Command::new(program)
                .args(args)
                // .stdin(new_follower_stdio())
                // .stdout(new_follower_stdio())
                // .stderr(new_follower_stdio())
                .exec()
            ;

            Err(err.into())
        }
        Ok(ForkResult::Parent { child }) => {
            // эта програма исполняется только в родительском процессе
            // дочерний процесс в это же время работает и что то делает

            // создаем дескриптов для чтения/записи из/в дочерний процесс
            // let mut ptyout = unsafe { tokio::fs::File::from_raw_fd(pty.master.as_raw_fd()) };
            let mut ptyinout = unsafe { tokio::fs::File::from_raw_fd(pty.master.as_raw_fd()) };
            
            // let mut ptyout = unsafe { tokio::fs::File::from_raw_fd(pty.master.as_raw_fd()) };

            // канал в котором появится сигнал о завершении работы
            // let (shutdown_tx, mut shutdown_rx) = mpsc::channel(1);

            // обработка сигналов заверешния раоты через CTRL+C
            let mut sigint = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt()).expect("Failed to create SIGINT handler");
            let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).expect("Failed to create SIGTERM handler");

            let mut stdin = tokio::io::stdin();
            let stdin_fd = stdin.as_raw_fd();
            // let mut stdin = stdin.take(1024);
            // let mut termios = get_termios(stdin_fd)?;
            // let c_lflag = termios.c_lflag;
            // let c_iflag = termios.c_iflag;
            // let c_oflag = termios.c_oflag;
            // let c_cflag = termios.c_cflag;
            // let c_cc = termios.c_cc;
            // set_keypress_mode(&mut termios);
            // set_termios(stdin_fd, &termios)?;

            // let mut stdin_fd = stdin.take(1024);

            // Создаем буферы для чтения и записи в родительском процессе
            let mut stdout = tokio::io::stdout();
            // let mut stderr = tokio::io::stderr();
        

            // Асинхронный обработчик
            let mut stdin_buf = [0; 1024];
            let mut pty_buf = [0; 1024];
            let status = loop {
                // eprintln!("{:?}: {:?}: tick", std::thread::current().id(), std::time::SystemTime::now());
                tokio::select! {
                    // Чтение из стандартного ввода и запись в PTY
                    r = stdin.read(&mut stdin_buf) => match r {
                        Ok(0) => {
                            // eprintln!("stdin.read: Ok(0)");
                        }, // EOF
                        Ok(n) => {
                            eprintln!("stdin.read: Ok({})", n);
                            if let Err(e) = ptyinout.write_all(&stdin_buf[..n]).await {
                                // eprintln!("pty_master.write_all: {:#?}", e);
                            }
                        }
                        Err(e) => {
                            eprintln!("stdin.read: {:#?}", e);
                        }
                    },
                    // Чтение из PTY и запись в стандартный вывод
                    r = ptyinout.read(&mut pty_buf) => match r {
                        Ok(0) => {
                            eprintln!("pty_master.read: Ok(0)");
                        }, // EOF
                        Ok(n) => {
                            eprintln!("pty_master.read: Ok({})", String::from_utf8_lossy(&pty_buf[..n]));
                            if let Err(e) = stdout.write_all(&pty_buf[..n]).await {
                                // eprintln!("stdout.write_all: {:#?}", e);
                            }

                            if let Err(e) = stdout.flush().await {
                                eprintln!("stdout.flush(): {:#?}", e);
                            }
                        }
                        Err(e) => {
                            // eprintln!("stdin.read: {:#?}", e);
                        }
                    },
                    // Обработка сигналов завершения
                    _ = sigint.recv() => {
                        // eprintln!("{:?}: {:?}: Received SIGINT, terminating child process...", std::thread::current().id(), std::time::SystemTime::now());
                        
                        let res = kill(child, SIGINT);
                        match res {
                            Ok(_) | Err(nix::errno::Errno::ESRCH) => {},
                            Err(e) => {
                                // eprintln!("{:?}: {:?}: kill(child, SIGINT): {:#?}", std::thread::current().id(), std::time::SystemTime::now(), e);
                            },
                        }
                    },
                    _ = sigterm.recv() => {
                        eprintln!("{:?}: {:?}: Received SIGTERM, terminating child process...", std::thread::current().id(), std::time::SystemTime::now());
                        let res = kill(child, SIGTERM);
                        match res {
                            Ok(_) | Err(nix::errno::Errno::ESRCH) => {},
                            Err(e) => {
                                // eprintln!("{:?}: {:?}: kill(child, SIGTERM): {:#?}", std::thread::current().id(), std::time::SystemTime::now(), e);
                            },
                        }
                    },
                    res = tokio::task::spawn_blocking(move || waitpio(child)) => match res {
                        Err(e) => {
                            // if let Err(_) = shutdown_tx.send(Err(e.into())).await {
                            eprintln!("{:?}: {:?}: spawn_blocking(move || waitpio: {:#?}", std::thread::current().id(), std::time::SystemTime::now(), e);
                            break Err(CliError::ShutdownSendError);
                            // }
                        }
                        Ok(Err(e)) => {
                            // if let Err(_) = shutdown_tx.send(Err(e.into())).await {
                            // eprintln!("{:?}: {:?}: spawn_blocking(move || waitpio: {:#?}", std::thread::current().id(), std::time::SystemTime::now(), e);
                            break Err(CliError::ShutdownSendError);
                            // }
                        }
                        Ok(Ok(true)) => {
                            // eprintln!("{:?}: {:?}: waitpio(child) = Ok(true)", std::thread::current().id(), std::time::SystemTime::now());
                            break Ok(());
                        }
                        Ok(Ok(false)) => {
                            // eprintln!("{:?}: {:?}: waitpio(child) = Ok(false)", std::thread::current().id(), std::time::SystemTime::now());
                        }
                    },
                    // _ = tokio::time::sleep(std::time::Duration::from_secs(5)) => {
                    //     eprintln!("timer...");
                    //     std::thread::sleep(std::time::Duration::from_secs(10));
                    // }
                }
            };

            // Восстанавливаем исходные атрибуты терминала
            // termios.c_lflag = c_lflag;
            // termios.c_iflag = c_iflag;
            // termios.c_oflag = c_oflag;
            // termios.c_cflag = c_cflag;
            // termios.c_cc = c_cc;

            // set_termios(stdin_fd, &mut termios)?;

            status
        }
        Err(e) => {
            eprintln!("{:?}: {:?}: Fork failed: {}", std::thread::current().id(), std::time::SystemTime::now(), e);
            Err(CliError::NixErrorno(e))
        }
    };

    // Ok(())
    // let mut pty = create_pty(&program, cmd);
    // println!("{:#?}", pty);

    // let mut ptyout = tokio::fs::File::from(File::from(pty.fd.try_clone().expect("fd clone failed")));
    // // let mut ptyin = tokio::fs::File::from(File::from(pty.fd.try_clone().expect("fd clone failed")));

    // let mut ptyout_buf: [u8; 1024] = [0; 1024];
    // // let mut stdin_buf: [u8; 1024] = [0; 1024];

    // let mut stdout = tokio::io::stdout();
    // // let mut stdin = tokio::io::stdin();
    // // let mut termios = get_termios(stdin.as_raw_fd())?;
    // // let c_lflag = termios.c_lflag;
    // // let c_iflag = termios.c_iflag;
    // // let c_oflag = termios.c_oflag;
    // // let c_cflag = termios.c_cflag;
    // // let c_cc = termios.c_cc;

    // // set_keypress_mode(&mut termios);
    // // set_termios(stdin.as_raw_fd(), &termios)?;

    // // let mut sigint =tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt()).expect("Failed to create SIGINT handler");
    // // let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).expect("Failed to create SIGTERM handler");

    // let (shutdown_tx, mut shutdown_rx) = mpsc::channel(1);
    // let status = loop {
    //     tokio::select! {
    //         res = ptyout.read(ptyout_buf.as_mut()) => match res {
    //             Ok(0) => {},
    //             Ok(n) => {
    //                 if let Err(e) = stdout.write_all(&ptyout_buf[..n]).await {
    //                     eprintln!("stdout.write_all: {:#?}", e);
    //                 }

    //                 ptyout_buf.fill(0);
    //             }
    //             Err(e) => {
    //                 eprintln!("ptyout.read: {:#?}", e);

    //                 shutdown_tx.send(true).await.expect("shutdown send failed");
    //             }
    //         },
    //         res = shutdown_rx.recv() => match res {
    //             Some(_) => {
    //                 break;
    //             }
    //             None => {

    //             }
    //         }
    //         // res = stdin.read(stdin_buf.as_mut()) => match res {
    //         //     Ok(0) => {},
    //         //     Ok(n) => {
    //         //         if let Err(e) = ptyin.write_all(&stdin_buf[..n]).await {
    //         //             eprintln!("ptyin.write_all: {:#?}", e);
    //         //         }
    //         //         if let Err(e) = ptyin.flush().await {
    //         //             eprintln!("ptyin.write_all: {:#?}", e);
    //         //         }

    //         //         stdin_buf.fill(0);
    //         //     }
    //         //     Err(e) => {
    //         //         eprintln!("stdin.read: {:#?}", e);
    //         //     }
    //         // },
    //         // _ = sigint.recv() => {
    //         //     if let Some(id) = pty.process.id() {
    //         //         let pid = Pid::from_raw(id.try_into().unwrap());
    //         //         if let Err(e) = nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGINT) {
    //         //             eprintln!("failed send SIGINT to pid: {}, error: {}", pid, e);
    //         //         }
    //         //     }
    //         // },
    //         // _ = sigterm.recv() => {
    //         //     if let Some(id) = pty.process.id() {
    //         //         let pid = Pid::from_raw(id.try_into().unwrap());
    //         //         if let Err(e) = nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGTERM) {
    //         //             eprintln!("failed send SIGTERM to pid: {}, error: {}", pid, e);
    //         //         }
    //         //     }
    //         // },
    //         // r = pty.process.wait() => match r {
    //         //     Ok(s) => match s.code() {
    //         //         None => {
    //         //             println!("Process terminated by signal");
    //         //             break Err(CliError::ExitCodeError(-1));
    //         //         },
    //         //         Some(0) => {
    //         //             println!("Ok()");
    //         //             break Ok(())
    //         //         },
    //         //         Some(s) => {
    //         //             println!("{:#?}", s);
    //         //             break Err(CliError::ExitCodeError(s))
    //         //         },
    //         //     }
    //         //     Err(e) => {
    //         //         println!("{:#?}", e);
    //         //         break Err(CliError::StdIoError(e));
    //         //     }
    //         // },
    //     }
    // };

    // termios.c_lflag = c_lflag;
    // termios.c_iflag = c_iflag;
    // termios.c_oflag = c_oflag;
    // termios.c_cflag = c_cflag;
    // termios.c_cc = c_cc;

    // set_termios(stdin.as_raw_fd(), &mut termios)?;
    // let termios = get_termios(stdin.as_raw_fd())?;
    println!("buy\n");

    status
}