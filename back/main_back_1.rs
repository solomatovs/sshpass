use std::env;
use std::fs::File;
use std::io::Read;
use std::os::fd::{AsRawFd, IntoRawFd};
use std::os::unix::io::FromRawFd;
use std::process::Stdio;
use std::thread;

use clap::{Arg, Command};
use libc;
use nix::pty::openpty;
use nix::unistd::{dup, fork, ForkResult};
use termion::input::TermRead;
use termios::Termios;
use termios::{tcsetattr, ECHO, ICANON, TCSANOW, IGNBRK, PARMRK, ISTRIP, INLCR, IGNCR, ICRNL, IXON, OPOST, ECHONL, ISIG, IEXTEN, CSIZE, PARENB, CS8, VMIN, VTIME, BRKINT };
use tokio::io::AsyncReadExt;
use tokio::io::{self, AsyncWriteExt};
use tokio::process::Command as TokioCommand;
use tokio::signal;
use totp_rs::{Algorithm, Secret, TOTP};

use thiserror::Error;

macro_rules! break_if_err {
    ($res:expr) => {
        match $res {
            Ok(val) => val,
            Err(e) => {
                break Err(e)
            }
        }
    };
}

#[tokio::main]
async fn main() -> Result<(), SshPassError> {
    // Инициализация логирования
    // env_logger::init();

    // Используем clap для обработки аргументов командной строки
    let matches = Command::new("sshpass_rust")
        .version("1.0")
        .about("SSH utility with password input")
        .arg(
            Arg::new("fd")
                .short('d')
                .long("fd")
                .value_name("FD")
                .help("File descriptor to read password from")
                .value_parser(clap::value_parser!(i32)),
        )
        .arg(
            Arg::new("ssh_command")
                .required(true)
                .num_args(1..)
                .help("SSH command to execute"),
        )
        .get_matches();

    // Проверка на конфликт аргументов
    let fd_arg = matches.get_one::<i32>("fd");
    let env_pass = env::var("SSHPASS").ok();

    if fd_arg.is_some() && env_pass.is_some() {
        return  Err(
            SshPassError::ArgumentError("Error: Arguments conflict. You can't use -d option with SSHPASS environment variable.".into())
        );
    }

    // Получаем SSH команду
    let mut ssh_command: Vec<&str> = matches
        .get_many::<String>("ssh_command")
        .unwrap()
        .map(|s| s.as_str())
        .collect();


    let ends = openpty(None, None).expect("openpty failed");
    let master = ends.master;
    let slave = ends.slave.into_raw_fd();

    // Создаем асинхронный процесс SSH
    let mut child = TokioCommand::new(ssh_command.remove(0))
        .args(&ssh_command)
        .stdin(unsafe {Stdio::from_raw_fd(slave)})
        .stdout(unsafe {Stdio::from_raw_fd(slave)})
        .stderr(unsafe {Stdio::from_raw_fd(slave)})
        .spawn()
        .unwrap();

	// match unsafe { fork() } {
	// 	Ok(ForkResult::Parent { child: pid, .. }) => {
	// 		thread::spawn(move || {
	// 			let mut status = 0;
	// 			unsafe { libc::waitpid(i32::from(pid), &mut status ,0) };
	// 			println!("child process exit!");
	// 			std::process::exit(0);
	// 		});

	// 	}
	// 	Ok(ForkResult::Child) => {
	// 		unsafe { ioctl_rs::ioctl(master, ioctl_rs::TIOCNOTTY) };
	// 		unsafe { libc::setsid() };
	// 		unsafe { ioctl_rs::ioctl(slave, ioctl_rs::TIOCSCTTY) };

	// 		builder
	// 		.stdin(unsafe { Stdio::from_raw_fd(slave) })
	// 		.stdout(unsafe { Stdio::from_raw_fd(slave) })
	// 		.stderr(unsafe { Stdio::from_raw_fd(slave) })
	// 		.exec();
	// 		return;
	// 	},
	// 	Err(_) => println!("Fork failed"),
	// }

    // Получаем потоки ввода/вывода дочернего процесса
    let mut child_stdin = child.stdin.take().ok_or(SshPassError::StdTakeError("child_stdin".into()))?;
    let mut child_stdout = child.stdout.take().ok_or(SshPassError::StdTakeError("child_stdout".into()))?;
    let mut child_stderr = child.stderr.take().ok_or(SshPassError::StdTakeError("child_stderr".into()))?;

    let mut sigint = signal::unix::signal(signal::unix::SignalKind::interrupt()).map_err(|_| SshPassError::StdTakeError("Failed to create SIGINT handler".into()))?;
    let mut sigterm = signal::unix::signal(signal::unix::SignalKind::terminate()).map_err(|_| SshPassError::StdTakeError("Failed to create SIGTERM handler".into()))?;

    // Local buffers
    let mut stdin_buf: [u8; 64] = [0; 64];
    let mut stdout_buf: [u8; 64] = [0; 64];
    let mut stderr_buf: [u8; 64] = [0; 64];

    let mut stdin = io::stdin();
    let mut stdout = io::stdout();
    let mut stderr = io::stderr();

    // set no canonical mode
    let termios = set_keypress(stdin.as_raw_fd())?;

    let status = loop {
        tokio::select! {
            r = stdin.read(&mut stdin_buf) => match r {
                Ok(r) if r > 0 => {
                    println!("stdin_buf: {:?}", stdin_buf);
                    break_if_err!(child_stdin.write_all(&stdin_buf[..r]).await.map_err(|e| SshPassError::StdIoError(e)));//.expect("Failed to write to stdin");
                    break_if_err!(stdout.write_all(&stdin_buf[..r]).await.map_err(|e| SshPassError::StdIoError(e)));//.expect("Failed to write to stdout");
                    break_if_err!(child_stdin.flush().await.map_err(|e| SshPassError::StdIoError(e)));//.expect("Failed to flush stdout");
                    break_if_err!(stdout.flush().await.map_err(|e| SshPassError::StdIoError(e)));//.expect("Failed to flush stdout");
                }
                Ok(_) => {

                }
                Err(e) => {
                      eprintln!("sshpass: read stdin {:?}", e);
                }
            },
            r = child_stdout.read(&mut stdout_buf) => match r  {
                Ok(r) if r > 0 => {
                    println!("stdin_buf: {:?}", stdout_buf);
                    break_if_err!(stdout.write_all(&stdin_buf[..r]).await.map_err(|e| SshPassError::StdIoError(e)));//.expect("Failed to write to stdout");
                    break_if_err!(stdout.flush().await.map_err(|e| SshPassError::StdIoError(e)));//.expect("Failed to flush stdout");
                }
                Ok(_) => {

                }
                Err(e) => {
                    eprintln!("sshpass: read stdout: {:?}", e);
                }
            },
            r = child_stderr.read(&mut stderr_buf) => match r  {
                Ok(r) if r > 0 => {
                    println!("stdin_buf: {:?}", stderr_buf);
                    break_if_err!(stderr.write_all(&stdin_buf[..r]).await.map_err(|e| SshPassError::StdIoError(e)));//.expect("Failed to write to stderr");
                    break_if_err!(stderr.flush().await.map_err(|e| SshPassError::StdIoError(e)));//.expect("Failed to flush stderr");
                }
                Ok(_) => {

                }
                Err(e) => {
                    eprintln!("sshpass: read stderr: {:?}", e);
                }
            },
            _ = sigint.recv() => {
                eprintln!("Received SIGINT, terminating child process...");
                break Ok(());
            },
            _ = sigterm.recv() => {
                eprintln!("Received SIGTERM, terminating child process...");
                break Ok(());
            },
            status = child.wait() => {
                match status {
                    Ok(s) => match s.code() {
                        Some(s) => break Err(SshPassError::ExitCodeError(s)),
                        None => break Ok(()),
                    }
                    Err(e) => {
                        break Err(SshPassError::StdIoError(e));
                    }
                }
            },
        }

        std::thread::sleep(std::time::Duration::from_millis(100));
    };

    set_termios(stdin.as_raw_fd(), &termios)?;

    // Завершаем дочерний процесс
    match child.try_wait() {
        Ok(Some(status)) => match status.code() {
            Some(s) => return Err(SshPassError::ExitCodeError(s)),
            None => return Ok(()),
        }
        Ok(None) => {
            println!("child.start_kill()? before");
            child.start_kill()?;
            child.wait().await?;
            println!("child.start_kill()? after");
        },
        Err(e) => {
            return Err(SshPassError::StdIoError(e));
        }
    }

    // Возвращаем код завершения процесса
    // std::process::exit(status);
    status
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

fn set_keypress(stdin_fild: i32) -> Result<Termios, std::io::Error> {
    let mut termios = Termios::from_fd(stdin_fild)?;//.context(format!("Termios::from_fd: {}", stdin_fild))?;
    let termios_origin = termios.clone();
    // termios.c_lflag &= !(ECHO | ICANON);

    termios.c_iflag &= !(IGNBRK | BRKINT | PARMRK | ISTRIP | INLCR | IGNCR | ICRNL |IXON);
    termios.c_oflag &= !OPOST;
    termios.c_lflag &= !(ECHO | ECHONL | ICANON | ISIG | IEXTEN);
    termios.c_cflag &= !(CSIZE | PARENB);
    termios.c_cflag |= CS8;
    termios.c_cc[VMIN] = 0;
    termios.c_cc[VTIME] = 0;

    tcsetattr(stdin_fild, TCSANOW, &mut termios)?;//.context(format!("tcsetattr to stdin_fild: {}", stdin_fild))?;

    Ok(termios_origin)
}

fn set_termios(stdin_fild: i32, termios: &Termios) -> Result<(), std::io::Error> {
    Ok(tcsetattr(stdin_fild, TCSANOW, &termios)?)
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


