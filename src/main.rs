mod unix;

use std::io::Write;
use std::os::unix::io::{AsFd, AsRawFd, FromRawFd};
use std::os::unix::process::CommandExt;
use std::process::Stdio;
use std::str::FromStr;

use nix::sys::signal::{kill, SigSet, Signal};
use nix::sys::wait::{waitpid, WaitPidFlag, WaitStatus};

use log::{debug, error, info, trace};
use nix::pty::openpty;
use nix::unistd::{fork, ForkResult};
use nix::{
    poll::{poll, PollFd, PollFlags, PollTimeout},
    unistd::{read, write},
};

use termios::Termios;
use termios::{
    tcsetattr, BRKINT, CS8, CSIZE, ECHO, ECHONL, ICANON, ICRNL, IEXTEN, IGNBRK, IGNCR, INLCR, ISIG,
    ISTRIP, IXON, OPOST, PARENB, PARMRK, TCSANOW, VMIN, VTIME,
};

use clap::{Arg, ArgGroup, ArgMatches, Command};

#[derive(Debug)]
pub enum CliError {
    StdIoError(std::io::Error),

    NixErrorno(nix::errno::Errno),

    ArgumentError(String),

    ExitCodeError(i32),

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

// Флаг          Значение
// ISIG          Разрешить посылку сигналов
// ICANON        Канонический ввод (обработка забоя и стирания строки)
// XCASE         Каноническое представление верхнего/нижнего регистров
// ECHO          Разрешить эхо
// ECHOE         Эхо на символ забоя - BS-SP-BS
// ECHOK         Выдавать NL после символа стирания строки
// ECHONL        Выдавать эхо на NL
// NOFLSH        Запретить сброс буферов после сигналов прерывания и
//               завершения
// TOSTOP        Посылать SIGTTOU фоновым процессам, которые пытаются
//               выводить на терминал
// ECHOCTL       Выдавать эхо на CTRL-символы как .r, ASCII DEL как
//               ?
// ECHOPRT       Эхо на символ забоя как стертый символ
// ECHOKE        При стирании строки, очищать ранее введенную строку
//               символами BS-SP-BS
// FLUSHO        Сбрасывание буфера вывода (состояние)
// PENDIN        Повторять несчитанный ввод при следующем чтении или
//               введенном символе
// IEXTEN        Разрешить расширенные (определенные реализацией)
//               функции
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

fn get_termsize(stdin_fild: i32) -> std::io::Result<Box<nix::libc::winsize>> {
    let mut size = Box::new(nix::libc::winsize {
        ws_row: 25,
        ws_col: 80,
        ws_xpixel: 0,
        ws_ypixel: 0,
    });
    let ret = unsafe { nix::libc::ioctl(stdin_fild, nix::libc::TIOCGWINSZ, &mut *size) };

    match ret {
        0 => Ok(size),
        _ => Err(std::io::Error::last_os_error()),
    }
}

pub fn set_termsize(fd: i32, mut size: Box<nix::libc::winsize>) -> std::io::Result<()> {
    let ret = unsafe {
        nix::libc::ioctl(fd, nix::libc::TIOCSWINSZ, &mut *size)
    };

    match ret {
        0 => Ok(()),
        _ => Err(std::io::Error::last_os_error()),
    }
}

fn cli() -> Command {
    let cmd = Command::new("sshpass")
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
        );

    cmd
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

        let _ = simplelog::CombinedLogger::init(vec![simplelog::WriteLogger::new(
            level,
            config,
            std::fs::File::create("sshpass.log").unwrap(),
        )])
        .unwrap();
    }

    let args = cli().get_matches();
    trace!("mach arguments {:#?}", args);

    trace!("args ok, start program");
    let status = run(args);

    if let Err(e) = status {
        error!("exit with error: {:?}", e);
    }
}

fn run(args: ArgMatches) -> Result<(), CliError> {
    // Создаем псевдотерминал (PTY)
    let pty = openpty(None, None).expect("Failed to open PTY");
    let slave = pty.slave;
    let master = pty.master.try_clone().expect("try_clone pty.master");

    // fork() - разделяет процесс на два
    // parent блок это продолжение текущего запущенного процесса
    // child блок это то, что выполняется в дочернем процессе
    // все окружение дочернего процесса наследуется из родительского
    let status = match unsafe { fork() } {
        Ok(ForkResult::Child) => {
            unsafe { nix::libc::ioctl(master.as_raw_fd(), nix::libc::TIOCNOTTY) };
            unsafe { nix::libc::setsid() };
            unsafe { nix::libc::ioctl(slave.as_raw_fd(), nix::libc::TIOCSCTTY) };
            // эта программа исполняется только в дочернем процессе
            // родительский процесс в это же время выполняется и что то делает

            let program = args.get_one::<String>("program").unwrap();
            let args = args.get_many::<String>("program_args").unwrap();

            // lambda функция для перенаправления stdio
            let new_follower_stdio = || unsafe { Stdio::from_raw_fd(slave.as_raw_fd()) };

            // ДАЛЬНЕЙШИЙ ЗАПУСК БЕЗ FORK ПРОЦЕССА
            // это означает что дочерний процесс не будет еще раз разделятся
            // Command будет выполняться под pid этого дочернего процесса и буквально станет им
            // осуществляется всё это с помощью exec()
            let e = std::process::Command::new(program)
                .args(args)
                .stdin(new_follower_stdio())
                .stdout(new_follower_stdio())
                .stderr(new_follower_stdio())
                .exec();

            error!("child error: {e}");

            Err(e.into())
        }
        Ok(ForkResult::Parent { child }) => {
            let stdin = std::io::stdin();
            let termios = get_termios(stdin.as_raw_fd())?;
            trace!("termios backup: {:#?}", termios);
            let c_lflag = termios.c_lflag;
            let c_iflag = termios.c_iflag;
            let c_oflag = termios.c_oflag;
            let c_cflag = termios.c_cflag;
            let c_cc = termios.c_cc;
            // // эта програма исполняется только в родительском процессе
            let stdin = std::io::stdin().lock();
            let mut termios = get_termios(stdin.as_raw_fd())?;
            set_keypress_mode(&mut termios);
            set_termios(stdin.as_raw_fd(), &termios)?;

            // регистрирую сигналы ОС для обработки в приложении
            let mut mask = SigSet::empty();
            mask.add(nix::sys::signal::SIGINT);
            mask.add(nix::sys::signal::SIGTERM);
            mask.add(nix::sys::signal::SIGCHLD);
            mask.add(nix::sys::signal::SIGSTOP);
            mask.add(nix::sys::signal::SIGWINCH);

            trace!("mask.thread_block()");
            mask.thread_block()
                .expect("pthread_sigmask(SigmaskHow::SIG_BLOCK, Some(self), None) error");

            trace!("nix::sys::signalfd::SignalFd::with_flags(&mask, nix::sys::signalfd::SfdFlags::SFD_NONBLOCK | nix::sys::signalfd::SfdFlags::SFD_CLOEXEC);");
            let signal_fd = nix::sys::signalfd::SignalFd::with_flags(
                &mask,
                nix::sys::signalfd::SfdFlags::SFD_NONBLOCK
                    | nix::sys::signalfd::SfdFlags::SFD_CLOEXEC,
            )
            // let signal_fd = nix::sys::signalfd::SignalFd::new(&mask)
            .expect("SignalFd error");

            // набор файловых указателей, которые будут обработаны poll
            let mut fds = [
                PollFd::new(signal_fd.as_fd(), PollFlags::POLLIN),
                PollFd::new(pty.master.as_fd(), PollFlags::POLLIN),
                PollFd::new(stdin.as_fd(), PollFlags::POLLIN),
            ];

            let mut stdout = std::io::stdout().lock();
            // Асинхронный обработчик
            let mut buf = [0; 1024];

            let status = loop {
                let mut is_timeout = false;
                trace!("poll(&mut fds, PollTimeout::MAX)");
                match poll(&mut fds, PollTimeout::MAX) {
                    Err(e) => {
                        // error poll calling
                        error!("poll calling error: {}", e);
                        break Err(e.into());
                    }
                    Ok(0) => {
                        // timeout
                        trace!("poll timeout: Ok(0)");
                        is_timeout = true;
                    }
                    Ok(n) => {
                        // match n events
                        trace!("poll match {} events", n);
                    }
                };

                trace!("fds: {:#?}", fds);
                trace!("check child process {} is running...", child);
                match waitpid(child, Some(WaitPidFlag::WNOHANG)) {
                    Err(nix::errno::Errno::ECHILD) => {
                        error!(
                            "the process {} is not a child of the process: {:?}",
                            child,
                            std::thread::current().id()
                        );
                        break Ok(());
                    }
                    Err(nix::errno::Errno::EINTR) => {
                        error!("waitpid error: {}", nix::errno::Errno::EINTR);
                        break Err(CliError::NixErrorno(nix::errno::Errno::EINTR));
                    }
                    Err(e) => {
                        error!("waitpid error: {}", e);
                        break Err(e.into());
                    }
                    Ok(WaitStatus::Exited(pid, status)) => {
                        info!("WaitStatus::Exited(pid: {:?}, status: {:?}", pid, status);
                        if status != 0 {
                            break Err(CliError::ExitCodeError(status));
                        } else {
                            break Ok(());
                        }
                    }
                    Ok(WaitStatus::Signaled(pid, sig, _dumped)) => {
                        info!(
                            "WaitStatus::Signaled(pid: {:?}, sig: {:?}, dumped: {:?})",
                            pid, sig, _dumped
                        );

                        break Ok(());
                    }
                    Ok(WaitStatus::Stopped(pid, sig)) => {
                        debug!("WaitStatus::Stopped(pid: {:?}, sig: {:?})", pid, sig);
                    }
                    Ok(WaitStatus::StillAlive) => {
                        trace!("WaitStatus::StillAlive");
                    }
                    Ok(WaitStatus::Continued(pid)) => {
                        trace!("WaitStatus::Continued(pid: {:?})", pid);
                    }
                    Ok(WaitStatus::PtraceEvent(pid, sig, c)) => {
                        trace!(
                            "WaitStatus::PtraceEvent(pid: {:?}, sig: {:?}, c: {:?})",
                            pid,
                            sig,
                            c
                        );
                    }
                    Ok(WaitStatus::PtraceSyscall(pid)) => {
                        trace!("WaitStatus::PtraceSyscall(pid: {:?})", pid);
                    }
                }

                if is_timeout {
                    continue;
                }

                trace!("check OS signal event...");
                if let Some(nix::poll::PollFlags::POLLIN) = fds[0].revents() {
                    trace!("match OS signal: {:#?}", fds[0].revents());
                    match signal_fd.read_signal() {
                        Ok(Some(sig)) => {
                            trace!("Some(res) = read_signal()");

                            match Signal::try_from(sig.ssi_signo as i32) {
                                Ok(Signal::SIGINT) => {
                                    info!("recv SIGINT");
                                    trace!("kill({child}, SIGINT");
                                    if let Err(nix::errno::Errno::ESRCH) =
                                        kill(child, Signal::SIGINT)
                                    {
                                        error!("pid {child} doesnt exists or zombie");
                                    }
                                }
                                Ok(Signal::SIGTERM) => {
                                    info!("recv SIGTERM");
                                    trace!("kill({child}, SIGTERM");
                                    if let Err(nix::errno::Errno::ESRCH) =
                                        kill(child, Signal::SIGTERM)
                                    {
                                        error!("pid {child} doesnt exists or zombie");
                                    }
                                }
                                Ok(Signal::SIGCHLD) => {
                                    info!("recv SIGCHLD");
                                }
                                Ok(Signal::SIGWINCH) => {
                                    info!("recv SIGWINCH");
                                    if let Ok(size) = get_termsize(stdin.as_raw_fd()) {
                                        trace!("set termsize: {:#?}", size);
                                        let res = set_termsize(slave.as_raw_fd(), size);
                                        trace!("set_termsize: {:#?}", res);
                                    }
                                }
                                Ok(Signal::SIGSTOP) => {
                                    info!("recv SIGSTOP");
                                }
                                Ok(sig) => {
                                    info!("recv signal {}", sig);
                                }
                                Err(e) => {
                                    error!("recv unknown signal");
                                    error!("{e}");
                                }
                            }
                        }
                        Err(nix::errno::Errno::EAGAIN) => {
                            trace!("Err(nix::errno::Errno::EAGAIN) = read_signal(), SFD_NONBLOCK flag is set");
                        }
                        Ok(None) => {
                            trace!("Ok(None) = read_signal(), SFD_NONBLOCK flag is set possible");
                        }
                        Err(e) => {
                            trace!("Err(e) = read_signal()");
                            error!("{}", e);
                        }
                    }
                    trace!("read OS signal after");
                }

                trace!("check pty events...");
                if let Some(nix::poll::PollFlags::POLLIN) = fds[1].revents() {
                    trace!("match pty event");
                    trace!("read(pty.master.as_raw_fd(), &mut buf[..])");

                    match read(pty.master.as_raw_fd(), &mut buf) {
                        Err(nix::errno::Errno::EAGAIN) => {
                            // SFD_NONBLOCK mode is set
                            trace!("Err(nix::errno::Errno::EAGAIN) = read(pty.master), SFD_NONBLOCK flag is set");
                        }
                        Err(e) => {
                            // error
                            trace!("Err(e) = read(pty.master)");
                            error!("pty.master read error");
                            error!("{}", e);
                        }
                        Ok(0) => {
                            // EOF
                            trace!("Ok(0) = read(pty.master)");
                        }
                        Ok(n) => {
                            // read n bytes
                            trace!("Ok({n}) = read(pty.master)");
                            trace!("utf8: {}", String::from_utf8_lossy(&buf[..n]));
                            if let Err(e) = stdout.write_all(&buf[..n]) {
                                trace!("Err(e) = stdout.write_all(&buf[..n])");
                                error!("stdout write error");
                                error!("{e}");
                            }
                            if let Err(e) = stdout.flush() {
                                trace!("Err(e) = stdout.write_all(&buf[..n])");
                                error!("stdout write error");
                                error!("{e}");
                            }
                        }
                    }
                }

                trace!("check stdin events...");
                if let Some(nix::poll::PollFlags::POLLIN) = fds[2].revents() {
                    trace!("read(stdin)");

                    match read(stdin.as_raw_fd(), &mut buf) {
                        Err(nix::errno::Errno::EAGAIN) => {
                            // SFD_NONBLOCK mode is set
                            trace!("Err(nix::errno::Errno::EAGAIN) = read(stdin), SFD_NONBLOCK flag is set");
                        }
                        Err(e) => {
                            // error
                            trace!("Err(e) = read(stdin)");
                            error!("stdin read error");
                            error!("{}", e);
                        }
                        Ok(0) => {
                            // EOF
                            trace!("Ok(0) = read(stdin)");
                        }
                        Ok(n) => {
                            trace!("Ok({n}) = read(stdin)");
                            trace!("utf8: {}", String::from_utf8_lossy(&buf[..n]));
                            if let Err(e) = write(pty.master.as_fd(), &buf[..n]) {
                                trace!("write(pty.master.as_fd()");
                                error!("error writing to pty");
                                error!("{e}");
                            }
                        }
                    }
                }
            };

            // Восстанавливаем исходные атрибуты терминала
            termios.c_lflag = c_lflag;
            termios.c_iflag = c_iflag;
            termios.c_oflag = c_oflag;
            termios.c_cflag = c_cflag;
            termios.c_cc = c_cc;

            trace!("termios resore: {:#?}", termios);
            set_termios(stdin.as_raw_fd(), &mut termios)?;

            status
        }
        Err(e) => {
            error!(
                "{:?}: {:?}: Fork failed: {}",
                std::thread::current().id(),
                std::time::SystemTime::now(),
                e
            );
            Err(CliError::NixErrorno(e))
        }
    };

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
