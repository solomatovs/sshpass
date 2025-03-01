// use std::fs::File;
// use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::io::FromRawFd;
// use std::os::unix::process::CommandExt;
use std::env;

use nix::libc::{self};
use nix::pty::openpty;
use nix::unistd::{fork, ForkResult, dup2};
// use std::sync::mpsc::channel;
// use std::sync::mpsc::sync_channel;
// use crossbeam_channel::{bounded, tick, Receiver, select};
// use std::{thread};
use tokio;

// use nix::unistd::{fork, ForkResult};
use std::process::Stdio;
// use termion::input::TermRead;
use termios::Termios;
use termios::{
    tcsetattr, BRKINT, CS8, CSIZE, ECHO, ECHONL, ICANON, ICRNL, IEXTEN, IGNBRK, IGNCR, INLCR, ISIG,
    ISTRIP, IXON, OPOST, PARENB, PARMRK, TCSANOW, VMIN, VTIME,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command as TokioCommand;
// use tokio::signal;
// use totp_rs::{Algorithm, Secret, TOTP};
use tokio::signal::unix;

// use thiserror::Error;
// use anyhow::{anyhow, bail, Result};

#[tokio::main]
async fn main() -> Result<(), CliError> {
    // Получаем аргументы командной строки
    let mut cmd: Vec<String> = env::args().collect();
    if cmd.len() < 2 {
        eprintln!("Usage: {} <command> [args...]", cmd[0]);
        std::process::exit(1);
    }

    // Создаем псевдотерминал
    let ends = openpty(None, None).expect("Failed to open PTY");
    let master = ends.master;
    let slave = ends.slave;

    let child_wait = match unsafe { fork() } {
        Ok(ForkResult::Parent { child: pid, .. }) => {
            println!("this is a parent fork, child process: {:?}", pid);
            pid
        }
        Ok(ForkResult::Child) => {
            println!("child fork");
            // let res = unsafe { libc::ioctl(master.as_raw_fd(), libc::TIOCNOTTY) };
            // if res == -1 {
            //     eprintln!("Failed libc::ioctl(master)");
            // }

            // Создаем новую сессию
            let res = unsafe { libc::setsid() };
            if res == -1 {
                eprintln!("Failed to create new session");
            }

            // let res = unsafe { libc::ioctl(slave.as_raw_fd(), libc::TIOCSCTTY) };
            // if res == -1 {
            //     eprintln!("Failed libc::ioctl(slave)");
            // }

            // Подключаем slave конец псевдотерминала к стандартным потокам ввода/вывода            
            // dup2(slave.as_raw_fd(), libc::STDIN_FILENO.as_raw_fd()).expect("Failed to dup2 stdin");
            // dup2(slave.as_raw_fd(), libc::STDOUT_FILENO.as_raw_fd()).expect("Failed to dup2 stdout");
            // dup2(slave.as_raw_fd(), libc::STDERR_FILENO.as_raw_fd()).expect("Failed to dup2 stderr");

            // // Закрываем master конец псевдотерминала
            // let _res = unsafe { libc::close(master.as_raw_fd()) };

            println!("run cmd");
            cmd.remove(0);
            let command = cmd.remove(0);
            let args = cmd;

            let res = TokioCommand::new(command)
                .args(args)
                .stdin(unsafe { Stdio::from_raw_fd(slave.as_raw_fd()) })
                .stdout(unsafe { Stdio::from_raw_fd(slave.as_raw_fd()) })
                .stderr(unsafe { Stdio::from_raw_fd(slave.as_raw_fd()) })
                .kill_on_drop(true)
                .status()
                .await;

            println!("cmd end: {:#?}", res);

            if let Err(e) = res {
                return Err(e.into());
            }

            return Ok(());
        }
        Err(e) => {
            return Err(e.into());
        }
    };

    let mut ptyin = unsafe { tokio::fs::File::from_raw_fd(master.as_raw_fd()) };
    let mut ptyout = unsafe { tokio::fs::File::from_raw_fd(master.as_raw_fd()) };

    let mut ptyout_buf: [u8; 1024] = [0; 1024];
    let mut stdin_buf: [u8; 1024] = [0; 1024];

    let mut stdin = tokio::io::stdin();
    // let mut termios = get_termios(stdin.as_raw_fd()).expect("get termios error");
    // let c_lflag = termios.c_lflag;
    // let c_iflag = termios.c_iflag;
    // let c_oflag = termios.c_oflag;
    // let c_cflag = termios.c_cflag;
    // let c_cc = termios.c_cc;

    // set_keypress_mode(&mut termios);
    // set_termios(stdin.as_raw_fd(), &termios).expect("set termios error");

    let mut stdout = tokio::io::stdout();

    let mut sigint =
        unix::signal(unix::SignalKind::interrupt()).expect("Failed to create SIGINT handler");
    let mut sigterm =
        unix::signal(unix::SignalKind::terminate()).expect("Failed to create SIGTERM handler");

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

                    stdin_buf.fill(0);
                }
                Err(e) => {
                    eprintln!("stdin.read: {:#?}", e);
                }
            },
            _ = sigint.recv() => {
                nix::sys::signal::kill(child_wait, nix::sys::signal::Signal::SIGINT).unwrap();
            },
            _ = sigterm.recv() => {
                nix::sys::signal::kill(child_wait, nix::sys::signal::Signal::SIGTERM).unwrap();
            },
            r = tokio::task::spawn_blocking(move || unsafe {
                let mut status = 0;
                eprintln!("wait pid: {}", child_wait);
                let _ = libc::waitpid(i32::from(child_wait), &mut status, 0);
                status
            }) => match r {
                    Ok(0) => {
                        break Ok(());
                    }
                    Ok(status) => {
                        break Err(CliError::ExitCodeError(status))
                    }
                    Err(e) => {
                        break Err(e.into());
                    }
            },
        }
    };

    // termios.c_lflag = c_lflag;
    // termios.c_iflag = c_iflag;
    // termios.c_oflag = c_oflag;
    // termios.c_cflag = c_cflag;
    // termios.c_cc = c_cc;
    // // println!("set termios\n{:#?}", termios);
    // set_termios(stdin.as_raw_fd(), &mut termios).expect("set termios error");

    status
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
// fn _strip_nl(s: &mut String) -> String {
//     if s.ends_with('\n') {
//         s.pop();
//         if s.ends_with('\r') {
//             s.pop();
//         }
//     }
//     let out: String = s.to_string();
//     out
// }

// Функция для чтения пароля в зависимости от аргументов командной строки
// fn _get_password(matches: &clap::ArgMatches) -> String {
//     if let Some(&fd) = matches.get_one::<i32>("fd") {
//         // Дублируем файловый дескриптор и читаем пароль
//         let fd_dup = dup(fd).expect("Failed to duplicate file descriptor");
//         let mut fd_file = unsafe { File::from_raw_fd(fd_dup) };
//         let mut password = String::new();
//         fd_file
//             .read_to_string(&mut password)
//             .expect("Failed to read password from file descriptor");
//         drop(fd_file); // Закрываем файл, так как он нам больше не нужен
//         password
//     } else if let Some(password) = env::var("SSHPASS").ok() {
//         // Использование переменной окружения SSHPASS
//         password
//     } else {
//         // Ввод пароля с клавиатуры
//         println!("Enter Password:");
//         let mut pass = TermRead::read_passwd(&mut std::io::stdin(), &mut std::io::stdout())
//             .expect("Failed to read password from tty")
//             .expect("Failed to read password from tty");
//         let pass = _strip_nl(&mut pass);
//         pass
//         // rpassword::read_password().expect("Failed to read password from tty")
//     }
// }

// fn _get_totp(_matches: &clap::ArgMatches) -> String {
//     let secret = _matches
//         .get_one::<String>("totp_secret")
//         .expect("totp secret is required");
//     _generate_totp(secret)
//     // "get_totp".into()
// }

// fn _generate_totp(secret: &str) -> String {
//     let totp = TOTP::new(
//         Algorithm::SHA1,
//         6,
//         1,
//         30,
//         Secret::Raw(secret.as_bytes().to_vec()).to_bytes().unwrap(),
//     )
//     .unwrap();
//     let token = totp.generate_current().unwrap();
//     token
// }

// let ptyout_reader = thread::spawn(move || {
//     let mut buf: [u8; 1024] = [0; 1024];
//     loop {
//         let result = ptyout.read(buf.as_mut());
//         let size = result.unwrap();

//         if size == 0 {
//             break;
//         }

//         // println!("ptyout: {}", String::from_utf8_lossy(&buf[..size]));
//         match stdout_tx.send(buf[..size].to_vec()) {
//             Ok(()) => (),
//             Err(_) => {
//                 break;
//             }
//         }

//         buf.fill(0);
//     }
// });

// let stdin_reader = thread::spawn(move || {
//     println!("ptyin reader");
//     let mut buf: [u8; 1024] = [0; 1024];
//     let mut stdin = std::io::stdin();

//     let mut termios = get_termios(stdin.as_raw_fd()).expect("get termios error");
//     // println!("current termios\n{:#?}", termios);
//     let c_lflag = termios.c_lflag;
//     let c_iflag = termios.c_iflag;
//     let c_oflag = termios.c_oflag;
//     let c_cflag = termios.c_cflag;
//     let c_cc = termios.c_cc;

//     set_keypress_mode(&mut termios);
//     set_termios(stdin.as_raw_fd(), &termios).expect("set termios error");

//     loop {
//         match stdin.read(buf.as_mut()) {
//             Ok(0) => {
//                 thread::sleep(std::time::Duration::from_millis(100));
//             },
//             Ok(n) => {
//                 match stdin_tx.send(buf[..n].to_vec()) {
//                     Ok(()) => (),
//                     Err(_) => {
//                         break;
//                     }
//                 }

//                 buf.fill(0);
//             }
//             Err(e) => {
//                 eprintln!("stdin.read: {:?}", e);
//                 break;
//             }
//         }
//     }

//     termios.c_lflag = c_lflag;
//     termios.c_iflag = c_iflag;
//     termios.c_oflag = c_oflag;
//     termios.c_cflag = c_cflag;
//     termios.c_cc = c_cc;
//     // println!("set termios\n{:#?}", termios);
//     set_termios(stdin.as_raw_fd(), &mut termios).expect("set termios error");
//     // let termios = get_termios(stdin.as_raw_fd()).expect("get termios error");
//     // println!("current termios\n{:#?}", termios);
// });

// let ptyin_writer = thread::spawn(move || loop {
//     let message = match stdin_rx.recv() {
//         Ok(m) => m,
//         Err(e) => {
//             eprintln!("stdin_rx.recv(): {:?}", e);
//             break;
//         }
//     };
//     // println!("stdin_rx.recv(): {}", String::from_utf8_lossy(&message));

//     if let Err(e) = ptyin.write_all(&message) {
//         eprintln!("ptyin.write_all: {:?}", e);
//         break;
//     }
// });

// let stdout_writer = thread::spawn(move || {
//     let mut stdout = std::io::stdout();
//     loop {
//         let message = match stdout_rx.recv() {
//             Ok(m) => m,
//             Err(e) => {
//                 eprintln!("stdout_rx.recv(): {:?}", e);
//                 break;
//             }
//         };

//         // println!("stdout_rx.recv(): {}", String::from_utf8_lossy(&message));

//         if let Err(e) = stdout.write_all(&message) {
//             eprintln!("stdout.write_all: {:?}", e);
//             break;
//         }
//     }
// });

// let _ = child_wait.join();
// let _ = stdout_writer.join();
// let _ = stdin_reader.join();
// let _ = ptyin_writer.join();
// let _ = ptyout_reader.join();

// match unsafe { fork() } {
//     Ok(ForkResult::Parent { .. }) => {
//         // Родительский процесс

//         // Закрываем slave конец псевдотерминала
//         let _ = unsafe { libc::close(pty_master.slave.as_raw_fd()) };

//         // Получаем потоки ввода/вывода дочернего процесса
//         let mut child_stdout = unsafe { std::fs::File::from_raw_fd(pty_master.master.as_raw_fd()) };
//         let mut child_stderr = child_stdout.try_clone().expect("Failed to clone PTY master");

//         let mut stdin_buf: [u8; 64] = [0; 64];
//         let mut stdout_buf: [u8; 64] = [0; 64];
//         let mut stderr_buf: [u8; 64] = [0; 64];

//         let mut stdin = io::stdin();
//         let mut stdout = io::stdout();
//         let mut stderr = io::stderr();

//         loop {
//             let mut child_stdout2 = child_stdout.try_clone().expect("Failed to clone PTY master");
//             let mut child_stderr2 = child_stderr.try_clone().expect("Failed to clone PTY master");

//             tokio::select! {
//                 r = stdin.read(&mut stdin_buf) => match r {
//                     Ok(n) if n > 0 => {
//                         let _ = child_stdout.write_all(&stdin_buf[..n]);
//                         let _ = child_stdout.flush();
//                     }
//                     Ok(_) => {}
//                     Err(e) => {
//                         eprintln!("sshpass: read stdin {:?}", e);
//                     }
//                 },
//                 r = tokio::task::spawn_blocking(move || child_stdout2.read(&mut stdout_buf)) => match r.unwrap() {
//                     Ok(n) if n > 0 => {
//                         let _ = stdout.write_all(&stdout_buf[..n]).await.map_err(|e| SshPassError::StdIoError(e));
//                         stdout.flush().await.unwrap();
//                     }
//                     Ok(_) => {}
//                     Err(e) => {
//                         eprintln!("sshpass: read stdout: {:?}", e);
//                     }
//                 },
//                 r = tokio::task::spawn_blocking(move || child_stderr2.read(&mut stderr_buf)) => match r.unwrap() {
//                     Ok(n) if n > 0 => {
//                         let _ = stderr.write_all(&stderr_buf[..n]).await.map_err(|e| SshPassError::StdIoError(e));
//                         stderr.flush().await.unwrap();
//                     }
//                     Ok(_) => {}
//                     Err(e) => {
//                         eprintln!("sshpass: read stderr: {:?}", e);
//                     }
//                 },
//             }

//             thread::sleep(std::time::Duration::from_secs(1));
//         }
//     }
//     Ok(ForkResult::Child) => {
//         // Дочерний процесс

//         // Создаем новую сессию
//         setsid().expect("Failed to create new session");

//         // Подключаем slave конец псевдотерминала к стандартным потокам ввода/вывода
//         dup2(pty_master.slave.as_raw_fd(), libc::STDIN_FILENO.as_raw_fd()).expect("Failed to dup2 stdin");
//         dup2(pty_master.slave.as_raw_fd(), libc::STDOUT_FILENO.as_raw_fd()).expect("Failed to dup2 stdout");
//         dup2(pty_master.slave.as_raw_fd(), libc::STDERR_FILENO.as_raw_fd()).expect("Failed to dup2 stderr");

//         // Закрываем master конец псевдотерминала
//         let _ = unsafe { libc::close(pty_master.master.as_raw_fd()) };

//         // Исполняем команду
//         ssh_command.remove(0);
//         let mut child = TokioCommand::new(ssh_command.remove(0))
//             .args(&ssh_command)
//             .stdin(unsafe { std::process::Stdio::from_raw_fd(pty_master.slave) })
//             .stdout(unsafe { std::process::Stdio::from_raw_fd(pty_master.slave) })
//             .stderr(unsafe { std::process::Stdio::from_raw_fd(pty_master.slave) })
//             // .kill_on_drop(true)
//             .spawn()
//             .unwrap();

//         let error = child.wait().await;

//         eprintln!("sshpass: failed to exec ssh: {:?}", error);
//     }
//     Err(_) => {
//         eprintln!("fork failed");
//     }
// }
// Создаем асинхронный процесс SSH
// cmd.remove(0);
// let mut child = TokioCommand::new(cmd.remove(0))
//     .args(&cmd)
//     // .stdin(std::process::Stdio::piped())
//     // .stdout(std::process::Stdio::piped())
//     // .stderr(std::process::Stdio::piped())
//     .stdin(unsafe { std::process::Stdio::from_raw_fd(pty_master.slave.as_raw_fd()) })
//     .stdout(unsafe { std::process::Stdio::from_raw_fd(pty_master.slave.as_raw_fd()) })
//     .stderr(unsafe { std::process::Stdio::from_raw_fd(pty_master.slave.as_raw_fd()) })
//     // .kill_on_drop(true)
//     .spawn()
//     .unwrap();

// // Получаем потоки ввода/вывода дочернего процесса
// let mut child_stdin = child.stdin.take().ok_or(SshPassError::StdTakeError("child_stdin".into()))?;
// let mut child_stdout = child.stdout.take().ok_or(SshPassError::StdTakeError("child_stdout".into()))?;
// let mut child_stderr = child.stderr.take().ok_or(SshPassError::StdTakeError("child_stderr".into()))?;

// let mut sigint = unix::signal(unix::SignalKind::interrupt()).map_err(|_| SshPassError::StdTakeError("Failed to create SIGINT handler".into()))?;
// let mut sigterm = unix::signal(unix::SignalKind::terminate()).map_err(|_| SshPassError::StdTakeError("Failed to create SIGTERM handler".into()))?;

// // Local buffers
// let mut stdin_buf: [u8; 64] = [0; 64];
// let mut stdout_buf: [u8; 64] = [0; 64];
// let mut stderr_buf: [u8; 64] = [0; 64];

// let mut stdin = io::stdin();

// let mut termios = get_termios(stdin.as_raw_fd())?;
// println!("current termios\n{:#?}", termios);
// let c_lflag = termios.c_lflag;
// let c_iflag = termios.c_iflag;
// let c_oflag = termios.c_oflag;
// let c_cflag = termios.c_cflag;
// let c_cc = termios.c_cc;

// set_keypress_mode(&mut termios);
// set_termios(stdin.as_raw_fd(), &termios)?;

// let mut stdout = io::stdout();
// let mut stderr = io::stderr();

// let status = loop {
//     tokio::select! {
//         r = stdin.read(&mut stdin_buf) => match r {
//             Ok(n) if n > 0 => {
//                 // let _ = child_stdin.write_all(b"stdin").await.map_err(|e| SshPassError::StdIoError(e));
//                 let _ = child_stdin.write_all(&stdin_buf[..n]).await.map_err(|e| SshPassError::StdIoError(e));
//                 child_stdin.flush().await.unwrap();
//             }
//             Ok(_) => {
//             }
//             Err(e) => {
//                 eprintln!("sshpass: read stdin {:?}", e);
//             }
//         },
//         r = child_stdout.read(&mut stdout_buf) => match r  {
//             Ok(n) if n > 0 => {
//                 // let _ = stdout.write_all(b"stdout").await.map_err(|e| SshPassError::StdIoError(e));
//                 let _ = stdout.write_all(&stdout_buf[..n]).await.map_err(|e| SshPassError::StdIoError(e));
//                 stdout.flush().await.unwrap();
//             }
//             Ok(_) => {
//             }
//             Err(e) => {
//                 eprintln!("sshpass: read stdout: {:?}", e);
//             }
//         },
//         r = child_stderr.read(&mut stderr_buf) => match r  {
//             Ok(n) if n > 0 => {
//                 // let _ = stderr.write_all(b"stderr").await.map_err(|e| SshPassError::StdIoError(e));
//                 let _ = stderr.write_all(&stderr_buf[..n]).await.map_err(|e| SshPassError::StdIoError(e));
//                 stderr.flush().await.unwrap();
//             }
//             Ok(_) => {
//             }
//             Err(e) => {
//                 eprintln!("sshpass: read stderr: {:?}", e);
//             }
//         },
//         _ = sigint.recv() => {
//             let id = child.id().unwrap();
//             sys::signal::kill(Pid::from_raw(id.try_into().unwrap()), sys::signal::Signal::SIGINT).unwrap();
//         },
//         _ = sigterm.recv() => {
//             let id = child.id().unwrap();
//             sys::signal::kill(Pid::from_raw(id.try_into().unwrap()), sys::signal::Signal::SIGTERM).unwrap();
//         },
//         // _ = time::sleep(time::Duration::from_millis(50)) => {
//         // }
//         status = child.wait() => {
//             match status {
//                 Ok(s) => match s.code() {
//                     None => {
//                         println!("Process terminated by signal");
//                         break Err(SshPassError::ExitCodeError(-1));
//                     },
//                     Some(0) => {
//                         println!("Ok()");
//                         break Ok(())
//                     },
//                     Some(s) => {
//                         println!("{:#?}", status);
//                         break Err(SshPassError::ExitCodeError(s))
//                     },
//                 }
//                 Err(e) => {
//                     println!("{:#?}", e);
//                     break Err(SshPassError::StdIoError(e));
//                 }
//             }
//         },
//     }
// };

// drop(stdout);
// drop(stderr);

// termios.c_lflag = c_lflag;
// termios.c_iflag = c_iflag;
// termios.c_oflag = c_oflag;
// termios.c_cflag = c_cflag;
// termios.c_cc = c_cc;
// println!("set termios\n{:#?}", termios);
// set_termios(stdin.as_raw_fd(), &mut termios)?;
// let termios = get_termios(stdin.as_raw_fd())?;
// println!("current termios\n{:#?}", termios);
// drop(stdin);

// status
// Ok(())
