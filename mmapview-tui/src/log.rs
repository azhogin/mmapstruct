use std::fmt;
use std::fs::File;
use std::fs::OpenOptions;
use std::io::Write;
use std::sync::Mutex;
use std::sync::OnceLock;

static LOG: OnceLock<Mutex<File>> = OnceLock::new();

fn log_file() -> &'static Mutex<File> {
    LOG.get_or_init(|| {
        Mutex::new(
            OpenOptions::new()
                .create(true)
                .append(true)
                .open("/tmp/mmapview-tui.log")
                .expect("failed to open /tmp/mmapview-tui.log"),
        )
    })
}

pub fn debug(msg: impl fmt::Display) {
    let mut f = log_file().lock().unwrap();
    writeln!(f, "{}", msg).unwrap();
}

/// Log a formatted message to `/tmp/mmapview-tui.log`.
#[macro_export]
macro_rules! debug_log {
    ($($arg:tt)*) => {
        $crate::log::debug(format!($($arg)*));
    };
}
