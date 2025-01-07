use nix::sys::signal::Signal;
use nix::sys::signalfd::siginfo;
use std::cell::Ref;


#[derive(Debug)]
pub enum UnixEvent<'a> {
    // PtyMaster(&'a [u8]),
    // Stdin(&'a [u8]),
    // Signal(Signal, &'a siginfo),
    Stdin(usize, Ref<'a, [u8]>),
    PtyMaster(usize, Ref<'a, [u8]>),
    PtySlave(usize, Ref<'a, [u8]>),
    Signal(usize, Signal, Ref<'a, siginfo>),
        // struct signalfd_siginfo {
        //     uint32_t ssi_signo;    /* Signal number */
        //     int32_t  ssi_errno;    /* Error number (unused) */
        //     int32_t  ssi_code;     /* Signal code */
        //     uint32_t ssi_pid;      /* PID of sender */
        //     uint32_t ssi_uid;      /* Real UID of sender */
        //     int32_t  ssi_fd;       /* File descriptor (SIGIO) */
        //     uint32_t ssi_tid;      /* Kernel timer ID (POSIX timers)
        //     uint32_t ssi_band;     /* Band event (SIGIO) */
        //     uint32_t ssi_overrun;  /* POSIX timer overrun count */
        //     uint32_t ssi_trapno;   /* Trap number that caused signal */
        //     int32_t  ssi_status;   /* Exit status or signal (SIGCHLD) */
        //     int32_t  ssi_int;      /* Integer sent by sigqueue(3) */
        //     uint64_t ssi_ptr;      /* Pointer sent by sigqueue(3) */
        //     uint64_t ssi_utime;    /* User CPU time consumed (SIGCHLD) */
        //     uint64_t ssi_stime;    /* System CPU time consumed
        //                               (SIGCHLD) */
        //     uint64_t ssi_addr;     /* Address that generated signal
        //                               (for hardware-generated signals) */
        //     uint16_t ssi_addr_lsb; /* Least significant bit of address
        //                               (SIGBUS; since Linux 2.6.37) */
        //     uint8_t  pad[X];       /* Pad size to 128 bytes (allow for
        //                               additional fields in the future) */
        // };
    ReadZeroBytes,
    PollTimeout,
}

impl std::fmt::Display for UnixEvent<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "UnixEvent")
    }
}

// impl From<std::io::Error> for UnixEvent<'_> {
//     fn from(e: std::io::Error) -> Self {
//         UnixEvent::StdIoError(e)
//     }
// }

// impl From<nix::errno::Errno> for UnixEvent<'_> {
//     fn from(e: nix::errno::Errno) -> Self {
//         UnixEvent::NixErrorno(e)
//     }
// }

// impl<'a> From<WaitStatus> for UnixEvent<'a> {
//     fn from(e: WaitStatus) -> Self {
//         UnixEvent::WaitStatus(e)
//     }
// }

#[derive(Debug)]
pub enum UnixEventResponse<'a> {
    Unhandled,
    SendTo(usize, Ref<'a, [u8]>),
    WriteToStdOut(Ref<'a, [u8]>),
    WriteToStdIn(Ref<'a, [u8]>),
    WriteToPtyMaster(Ref<'a, [u8]>),
    WriteToPtySlave(Ref<'a, [u8]>),
}

impl std::fmt::Display for UnixEventResponse<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "UnixEventResponse")
    }
}