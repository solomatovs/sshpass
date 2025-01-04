
use std::str::FromStr;
use nix::sys::signal::Signal;
use clap::{Arg, ArgGroup, Command};
use log::{trace, error};

mod app;

#[cfg(target_os = "linux")]
mod unix;
use unix::{UnixApp, UnixEvent, UnixError};

fn cli() -> Command {
    Command::new("sshpass")
        .version("1.0")
        .about("Non-interactive ssh password provider")
        .arg(
            Arg::new("password")
                .short('p')
                .long("password")
                .value_name("PASSWORD")
                .help("Provide password as argument (security unwise)"),
        )
        .arg(
            Arg::new("filename")
                .short('f')
                .long("file")
                .value_name("FILENAME")
                .help("Take password to use from file"),
        )
        .arg(
            Arg::new("fd")
                .short('d')
                .long("fd")
                .value_name("FD")
                .help("Use number as file descriptor for getting password"),
        )
        .arg(
            Arg::new("env")
                .short('e')
                .long("env")
                .value_name("ENV")
                .help("Password is passed as env-var 'SSHPASS'"),
        )
        .arg(
            Arg::new("prompt")
                .short('P')
                .long("prompt")
                .value_name("PROMPT")
                .help("Which string should sshpass search for to detect a password prompt"),
        )
        .arg(
            Arg::new("verbose")
                .short('v')
                .long("verbose")
                .value_name("VERBOSE")
                .help("Be verbose about what you're doing"),
        )
        .arg(
            Arg::new("otp-secret")
                .long("otp-secret")
                .help("One time secret in argument"),
        )
        .arg(
            Arg::new("otp-secret-file")
                .long("otp-secret-file")
                .help("One time secret in file"),
        )
        .arg(
            Arg::new("otp-secret-env")
                .env("SSHPASS_OTP_SECRET")
                .long("otp-secret-env")
                .help("One time secret is passed as env"),
        )
        .arg(
            Arg::new("otp-secret-fd")
                .long("otp-secret-fd")
                .help("Use number as file descriptor for getting otp secret"),
        )
        .arg(
            Arg::new("otp-code")
                .long("otp-code")
                .help("One time code in argument"),
        )
        .arg(
            Arg::new("otp-code-file")
                .long("otp-code-file")
                .help("One time code in file"),
        )
        .arg(
            Arg::new("otp-code-env")
                .env("SSHPASS_OTP_CODE")
                .long("otp-code-env")
                .help("One time code is passed as env"),
        )
        .arg(
            Arg::new("otp-code-fd")
                .long("otp-code-fd")
                .help("Use number as file descriptor for getting otp code"),
        )
        .arg(
            Arg::new("otp-prompt")
                .short('O')
                .long("otp-prompt")
                .help("Which string should sshpass search for the one time password prompt"),
        )
        .group(
            ArgGroup::new("password-conflict")
                .args(["password"])
                .conflicts_with_all(["filename", "fd", "env"]),
        )
        .group(
            ArgGroup::new("otp-conflict")
                .args(["otp-secret"])
                .conflicts_with_all([
                    "otp-secret-file",
                    "otp-secret-fd",
                    "otp-secret-env",
                    "otp-code",
                    "otp-code-file",
                    "otp-code-fd",
                    "otp-code-env",
                ]),
        )
        .arg(
            Arg::new("program")
                .help("Program to execute")
                .required(true)
                .num_args(1),
        )
        .arg(
            Arg::new("program_args")
                .help("arguments that will be passed to the program being run")
                .required(false)
                .num_args(1..)
                .allow_hyphen_values(true)
                .trailing_var_arg(true),
        )
}

fn main() {
    if let Ok(level) = std::env::var("SSHPASS_LOG") {
        let level = log::LevelFilter::from_str(&level).unwrap();

        let config = simplelog::ConfigBuilder::new()
            .set_time_format_rfc3339()
            .set_time_offset_to_local()
            .unwrap()
            .set_max_level(level)
            .build();

        simplelog::CombinedLogger::init(vec![simplelog::WriteLogger::new(
            level,
            config,
            std::fs::File::create("sshpass.log").unwrap(),
        )])
        .unwrap();
    }

    let args = cli().get_matches();
    trace!("mach arguments {:#?}", args);

    #[cfg(target_os = "linux")]
    let status = {
        trace!("app ok, create unix app");
        let mut app = UnixApp::new(args).unwrap();
        loop {
            {
                match app.system_event() {
                    Ok(res) => match res {
                        UnixEvent::PollTimeout => {
                            trace!("poll timeout");
                        }
                        // UnixEvent::ChildExited(_pid, status) => {
                        //     trace!("child {} exit: {}", _pid, status);
                        // }
                        // UnixEvent::ChildSignaled(_pid, _signal, _dumped) => {
                        //     trace!("child {} signal: {} dumped {}", _pid, _signal, _dumped);
                        // }
                        UnixEvent::PtyMaster(buf) => {
                            trace!("pty utf8: {}", String::from_utf8_lossy(buf.trim_ascii()));
                            // app.send_to(0, buf);
                        }
                        UnixEvent::Stdin(buf) => {
                            trace!("stdin utf8: {}", String::from_utf8_lossy(buf.trim_ascii()));
                        }
                        UnixEvent::Signal(sig, _sigino) => {
                            trace!("signal {:#?}", sig);
                            // trace!("signal info {:#?}", sigino);
                            if matches!(sig, Signal::SIGINT | Signal::SIGTERM) {
                                break 0;
                            }
                        }
                        UnixEvent::ReadZeroBytes => {
                            trace!("read zero bytes");
                        }
                    }
                    Err(UnixError::StdIoError(ref e)) => {
                        error!("IO Error: {}", e);
                        break 1;
                    }
                    Err(UnixError::NixErrorno(ref e)) => {
                        error!("Nix Error: {}", e);
                        break 2;
                    }
                    Err(UnixError::PollEventNotHandle) => {
                        error!("the poll reported the event");
                        error!("However, the event was not read through the handlers");
                        break 3;
                    }
                }    
            }
        }
    };

    std::process::exit(status);
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

// // Функция для чтения пароля в зависимости от аргументов командной строки
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
