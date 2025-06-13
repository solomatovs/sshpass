#[cfg(feature = "log_queue_max_len_128")]
pub const LOG_QUEUE_MAX_LEN: usize = 128;
#[cfg(feature = "log_queue_max_len_256")]
pub const LOG_QUEUE_MAX_LEN: usize = 256;
#[cfg(feature = "log_queue_max_len_512")]
pub const LOG_QUEUE_MAX_LEN: usize = 512;
#[cfg(feature = "log_queue_max_len_1024")]
pub const LOG_QUEUE_MAX_LEN: usize = 1024;
#[cfg(feature = "log_queue_max_len_2048")]
pub const LOG_QUEUE_MAX_LEN: usize = 2048;
#[cfg(feature = "log_queue_max_len_4096")]
pub const LOG_QUEUE_MAX_LEN: usize = 4096;
#[cfg(feature = "log_queue_max_len_8192")]
pub const LOG_QUEUE_MAX_LEN: usize = 8192;

#[cfg(not(any(
    feature = "log_queue_max_len_128",
    feature = "log_queue_max_len_256",
    feature = "log_queue_max_len_512",
    feature = "log_queue_max_len_1024",
    feature = "log_queue_max_len_2048",
    feature = "log_queue_max_len_4096",
    feature = "log_queue_max_len_8192"
)))]
compile_error!("You must enable one of the `log_queue_max_len_*` features");

#[cfg(feature = "log_message_max_len_1")]
pub const LOG_MESSAGE_MAX_LEN: usize = 1;
#[cfg(feature = "log_message_max_len_2")]
pub const LOG_MESSAGE_MAX_LEN: usize = 2;
#[cfg(feature = "log_message_max_len_4")]
pub const LOG_MESSAGE_MAX_LEN: usize = 4;
#[cfg(feature = "log_message_max_len_8")]
pub const LOG_MESSAGE_MAX_LEN: usize = 8;
#[cfg(feature = "log_message_max_len_16")]
pub const LOG_MESSAGE_MAX_LEN: usize = 16;
#[cfg(feature = "log_message_max_len_32")]
pub const LOG_MESSAGE_MAX_LEN: usize = 32;
#[cfg(feature = "log_message_max_len_64")]
pub const LOG_MESSAGE_MAX_LEN: usize = 64;
#[cfg(feature = "log_message_max_len_128")]
pub const LOG_MESSAGE_MAX_LEN: usize = 128;
#[cfg(feature = "log_message_max_len_256")]
pub const LOG_MESSAGE_MAX_LEN: usize = 256;
#[cfg(feature = "log_message_max_len_512")]
pub const LOG_MESSAGE_MAX_LEN: usize = 512;
#[cfg(feature = "log_message_max_len_1024")]
pub const LOG_MESSAGE_MAX_LEN: usize = 1024;

#[cfg(not(any(
    feature = "log_message_max_len_1",
    feature = "log_message_max_len_2",
    feature = "log_message_max_len_4",
    feature = "log_message_max_len_8",
    feature = "log_message_max_len_16",
    feature = "log_message_max_len_32",
    feature = "log_message_max_len_64",
    feature = "log_message_max_len_128",
    feature = "log_message_max_len_256",
    feature = "log_message_max_len_512",
    feature = "log_message_max_len_1024",
)))]
compile_error!("You must enable one of the `log_message_max_len_*` features");