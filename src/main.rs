use std::env;
use std::fs::File;
use std::io::{Read, Write};
use std::os::unix::io::FromRawFd;
use std::os::fd::AsRawFd;
use std::os::unix::process::CommandExt;

use nix::pty::{openpty, Winsize};
use nix::sys;
use nix::unistd::{Pid, dup, dup2, ForkResult, fork, setsid};
use termion::input::TermRead;
use termios::Termios;
use termios::{tcsetattr, ECHO, ICANON, TCSANOW, IGNBRK, PARMRK, ISTRIP, INLCR, IGNCR, ICRNL, IXON, OPOST, ECHONL, ISIG, IEXTEN, CSIZE, PARENB, CS8, VMIN, VTIME, BRKINT};
use tokio::io::AsyncReadExt;
use tokio::io::{self, AsyncWriteExt};
use tokio::process::Command as TokioCommand;
// use tokio::signal;
use totp_rs::{Algorithm, Secret, TOTP};
use tokio::signal::unix;

use thiserror::Error;


#[tokio::main]
async fn main() -> Result<(), SshPassError> {
    // Получаем аргументы командной строки
    let mut ssh_command: Vec<String> = env::args().collect();
    if ssh_command.len() < 2 {
        eprintln!("Usage: {} <command> [args...]", ssh_command[0]);
        std::process::exit(1);
    }

    // Создаем псевдотерминал
    let pty_master = openpty(None, None).expect("Failed to open PTY");
    // Настраиваем терминальные параметры
    let mut termios = get_termios(pty_master.master.as_raw_fd()).expect("Failed to get termios");
    termios::cfmakeraw(&mut termios);
    set_termios(pty_master.master.as_raw_fd(), &termios).expect("Failed to set termios");

    match unsafe { fork() } {
        Ok(ForkResult::Parent { .. }) => {
            // Родительский процесс

            // Закрываем slave конец псевдотерминала
            let _ = unsafe { libc::close(pty_master.slave.as_raw_fd()) };

            // Получаем потоки ввода/вывода дочернего процесса
            let mut child_stdout = unsafe { std::fs::File::from_raw_fd(pty_master.master.as_raw_fd()) };
            let mut child_stderr = child_stdout.try_clone().expect("Failed to clone PTY master");

            let mut stdin_buf: [u8; 64] = [0; 64];
            let mut stdout_buf: [u8; 64] = [0; 64];
            let mut stderr_buf: [u8; 64] = [0; 64];

            let mut stdin = io::stdin();
            let mut stdout = io::stdout();
            let mut stderr = io::stderr();

            loop {

                let mut child_stdout2 = child_stdout.try_clone().expect("Failed to clone PTY master");
                let mut child_stderr2 = child_stderr.try_clone().expect("Failed to clone PTY master");

                tokio::select! {
                    r = stdin.read(&mut stdin_buf) => match r {
                        Ok(n) if n > 0 => {
                            let _ = child_stdout.write_all(&stdin_buf[..n]);
                            let _ = child_stdout.flush();
                        }
                        Ok(_) => {}
                        Err(e) => {
                            eprintln!("sshpass: read stdin {:?}", e);
                        }
                    },
                    r = tokio::task::spawn_blocking(move || child_stdout2.read(&mut stdout_buf)) => match r.unwrap() {
                        Ok(n) if n > 0 => {
                            let _ = stdout.write_all(&stdout_buf[..n]).await.map_err(|e| SshPassError::StdIoError(e));
                            stdout.flush().await.unwrap();
                        }
                        Ok(_) => {}
                        Err(e) => {
                            eprintln!("sshpass: read stdout: {:?}", e);
                        }
                    },
                    r = tokio::task::spawn_blocking(move || child_stderr2.read(&mut stderr_buf)) => match r.unwrap() {
                        Ok(n) if n > 0 => {
                            let _ = stderr.write_all(&stderr_buf[..n]).await.map_err(|e| SshPassError::StdIoError(e));
                            stderr.flush().await.unwrap();
                        }
                        Ok(_) => {}
                        Err(e) => {
                            eprintln!("sshpass: read stderr: {:?}", e);
                        }
                    },
                }
            }
        }
        Ok(ForkResult::Child) => {
            // Дочерний процесс

            // Создаем новую сессию
            setsid().expect("Failed to create new session");

            // Подключаем slave конец псевдотерминала к стандартным потокам ввода/вывода
            dup2(pty_master.slave.as_raw_fd(), libc::STDIN_FILENO.as_raw_fd()).expect("Failed to dup2 stdin");
            dup2(pty_master.slave.as_raw_fd(), libc::STDOUT_FILENO.as_raw_fd()).expect("Failed to dup2 stdout");
            dup2(pty_master.slave.as_raw_fd(), libc::STDERR_FILENO.as_raw_fd()).expect("Failed to dup2 stderr");

            // Закрываем master конец псевдотерминала
            let _ = unsafe { libc::close(pty_master.master.as_raw_fd()) };

            // Исполняем команду
            ssh_command.remove(0);
            let cmd = std::process::Command::new(ssh_command.remove(0)).args(&ssh_command);
            let error = cmd.exec();

            eprintln!("sshpass: failed to exec ssh: {:?}", error);
        }
        Err(_) => {
            eprintln!("fork failed");
        }
    }
    // // Создаем асинхронный процесс SSH
    // ssh_command.remove(0);
    // let mut child = TokioCommand::new(ssh_command.remove(0))
    //     .args(&ssh_command)
    //     .stdin(std::process::Stdio::piped())
    //     .stdout(std::process::Stdio::piped())
    //     .stderr(std::process::Stdio::piped())
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
    Ok(())
}

fn set_keypress_mode(termios: &mut Termios) {
    termios.c_iflag &= !(IGNBRK | BRKINT | PARMRK | ISTRIP | INLCR | IGNCR | ICRNL |IXON);
    termios.c_oflag &= !OPOST;
    termios.c_lflag &= !(ECHO | ECHONL | ICANON | ISIG | IEXTEN);
    termios.c_cflag &= !(CSIZE | PARENB);
    termios.c_cflag |= CS8;
    termios.c_cc[VMIN] = 0;
    termios.c_cc[VTIME] = 0;
}

fn set_termios(stdin_fild: i32, termios: &Termios) -> Result<(), std::io::Error> {
    Ok(tcsetattr(stdin_fild, TCSANOW, &termios)?)
}

fn get_termios(stdin_fild: i32) -> io::Result<Termios> {
    Termios::from_fd(stdin_fild)
}

#[derive(Debug, Error)]
pub enum SshPassError {
    #[error("std io: {0}")]
    StdIoError(#[from] std::io::Error),

    #[error("Argument error")]
    ArgumentError(String),
    
    #[error("Failed to open {0}")]
    StdTakeError(String),
    
    #[error("exit code: {0}")]
    ExitCodeError(i32),
    
    #[error("exit status: {0}")]
    ExitStatusError(std::process::ExitStatus),

    #[error("the data for key `{0}` is not available")]
    Redaction(String),
    #[error("invalid header (expected {expected:?}, found {found:?})")]
    InvalidHeader {
        expected: String,
        found: String,
    },
    #[error("unknown data store error")]
    Unknown,
}



fn _strip_nl(s: &mut String) -> String {
    if s.ends_with('\n') {
        s.pop();
        if s.ends_with('\r') {
            s.pop();
        }
    }
    let out: String = s.to_string();
    out
}

// Функция для чтения пароля в зависимости от аргументов командной строки
fn _get_password(matches: &clap::ArgMatches) -> String {
    if let Some(&fd) = matches.get_one::<i32>("fd") {
        // Дублируем файловый дескриптор и читаем пароль
        let fd_dup = dup(fd).expect("Failed to duplicate file descriptor");
        let mut fd_file = unsafe { File::from_raw_fd(fd_dup) };
        let mut password = String::new();
        fd_file
            .read_to_string(&mut password)
            .expect("Failed to read password from file descriptor");
        drop(fd_file); // Закрываем файл, так как он нам больше не нужен
        password
    } else if let Some(password) = env::var("SSHPASS").ok() {
        // Использование переменной окружения SSHPASS
        password
    } else {
        // Ввод пароля с клавиатуры
        println!("Enter Password:");
        let mut pass = TermRead::read_passwd(&mut std::io::stdin(), &mut std::io::stdout())
            .expect("Failed to read password from tty")
            .expect("Failed to read password from tty");
        let pass = _strip_nl(&mut pass);
        pass
        // rpassword::read_password().expect("Failed to read password from tty")
    }
}

fn _get_totp(_matches: &clap::ArgMatches) -> String {
    let secret = _matches
        .get_one::<String>("totp_secret")
        .expect("totp secret is required");
    _generate_totp(secret)
    // "get_totp".into()
}

fn _generate_totp(secret: &str) -> String {
    let totp = TOTP::new(
        Algorithm::SHA1,
        6,
        1,
        30,
        Secret::Raw(secret.as_bytes().to_vec()).to_bytes().unwrap(),
    )
    .unwrap();
    let token = totp.generate_current().unwrap();
    token
}
