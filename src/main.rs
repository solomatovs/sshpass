use std::env;
use std::fs::File;
use std::io::Read;
use std::os::unix::io::FromRawFd;
use std::process::Stdio;

use clap::{Arg, Command};
use nix::unistd::dup;
use termion::input::TermRead;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command as TokioCommand;
use tokio::signal;

#[tokio::main]
async fn main() {
    // Инициализация логирования
    env_logger::init();

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
        eprintln!(
            "Error: Arguments conflict. You can't use -d option with SSHPASS environment variable."
        );
        std::process::exit(1);
    }

    // Получаем SSH команду
    let mut ssh_command: Vec<&str> = matches
        .get_many::<String>("ssh_command")
        .unwrap()
        .map(|s| s.as_str())
        .collect();

    // Создаем асинхронный процесс SSH
    let mut child = TokioCommand::new(ssh_command.remove(0))
        .args(&ssh_command)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
        ;


    // Получаем потоки ввода/вывода дочернего процесса
    let mut child_stdin = child.stdin.take().expect("Failed to open stdin");
    let child_stdout = child.stdout.take().expect("Failed to open stdout");
    let child_stderr = child.stderr.take().expect("Failed to open stderr");

    // Создаем асинхронные буферизованные читатели для stdout и stderr
    let mut stdout_reader = BufReader::new(child_stdout).lines();
    let mut stderr_reader = BufReader::new(child_stderr).lines();
    let mut sigint = signal::unix::signal(signal::unix::SignalKind::interrupt()).expect("Failed to create SIGINT handler");
    let mut sigterm = signal::unix::signal(signal::unix::SignalKind::terminate()).expect("Failed to create SIGTERM handler");

    // создаём псевдотерминал
    // let mut terminal = child.spawn_terminal()?;
    // Ожидаем строку "Enter password:" из stdout или "--2fpa--" из stderr
    let status = loop {
        tokio::select! {
            // Чтение данных из stdout
            line = stdout_reader.next_line() => match line {
                Ok(Some(line)) => {
                    if line.contains("password:") {
                        // Передаем пароль в stdin процесса
                        let password = get_password(&matches);
                        child_stdin.write_all(password.as_bytes()).await.expect("Failed to write to stdin");
                        child_stdin.flush().await.expect("Failed to flush stdin");
                    } else {
                        println!("{}", line);
                    }
                }
                Ok(None) => {

                }
                Err(e) => {
                    eprintln!("sshpass: {:?}", e);
                }
            },
            // Чтение данных из stderr
            line = stderr_reader.next_line() => match line {
                Ok(Some(line)) => {
                    if line.contains("= Challenge =") {
                        // Передаем пароль в stdin процесса
                        let password = get_totp(&matches);
                        child_stdin.write_all(password.as_bytes()).await.expect("Failed to write to stdin");
                        child_stdin.flush().await.expect("Failed to flush stdin");
                    } else {
                        eprintln!("{}", line);
                    }
                } 
                Ok(None) => {

                }
                Err(e) => {
                    eprintln!("sshpass: {:?}", e);
                }
            },
            _ = sigint.recv() => {
                eprintln!("Received SIGINT, terminating child process...");
                break 0;
            },
            _ = sigterm.recv() => {
                eprintln!("Received SIGTERM, terminating child process...");
                break 0;
            },
            status = child.wait() => {
                if let Ok(s) = status {
                    match s.code() {
                        Some(s) => break s,
                        None => break 0,
                    }
                } else if let Err(_) = status {
                    break -1;
                }
            },
        }

        std::thread::sleep(std::time::Duration::from_millis(100));
    };

    // Завершаем дочерний процесс
    let _ = child.kill().await;

    // Возвращаем код завершения процесса
    std::process::exit(status);
}

fn strip_nl(s: &mut String) -> String {
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
fn get_password(matches: &clap::ArgMatches) -> String {
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
        let pass = strip_nl(&mut pass);
        pass
        // rpassword::read_password().expect("Failed to read password from tty")
    }
}

fn get_totp(_matches: &clap::ArgMatches) -> String {
    return "test".into();
    // todo!();
}