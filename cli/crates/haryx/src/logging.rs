//! Logging (upstream `logging_config.setup_logging`): every record at INFO and above (DEBUG with
//! `--debug`) goes to `<store root>/haryx.log`, rotated at 1 MB with three backups; the console gets
//! records only with `--debug`. The log directory is created on the first record actually written, so a
//! no-op run never materialises the store.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

use log::{Level, LevelFilter, Log, Metadata, Record};

const LOG_FILE: &str = "haryx.log";
const MAX_BYTES: u64 = 1024 * 1024;
const BACKUPS: u32 = 3;

struct Logger {
    file: Option<PathBuf>,
    console: bool,
    level: Level,
    write_lock: Mutex<()>,
}

impl Logger {
    fn rotate(path: &PathBuf) {
        let Ok(meta) = fs::metadata(path) else {
            return;
        };
        if meta.len() < MAX_BYTES {
            return;
        }
        for n in (1..BACKUPS).rev() {
            let _ = fs::rename(
                path.with_extension(format!("log.{n}")),
                path.with_extension(format!("log.{}", n + 1)),
            );
        }
        let _ = fs::rename(path, path.with_extension("log.1"));
    }

    fn write_file(&self, record: &Record<'_>) {
        let Some(path) = &self.file else {
            return;
        };
        let _guard = self.write_lock.lock();
        if let Some(dir) = path.parent()
            && fs::create_dir_all(dir).is_err()
        {
            return;
        }
        Self::rotate(path);
        if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
            let now = jiff::Zoned::now().strftime("%Y-%m-%d %H:%M:%S,%3f");
            let _ = writeln!(file, "{now} - {} - {}", record.level(), record.args());
        }
    }
}

impl Log for Logger {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.level() <= self.level && metadata.target().starts_with("haryx")
    }

    fn log(&self, record: &Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }
        self.write_file(record);
        if self.console {
            eprintln!("{}: {}", record.level(), record.args());
        }
    }

    fn flush(&self) {}
}

/// Install the logger. `--debug` adds DEBUG records and the console.
pub fn init(debug: bool) {
    let level = if debug { Level::Debug } else { Level::Info };
    let file = haryx_core::paths::store_root()
        .ok()
        .map(|root| root.join(LOG_FILE));
    let logger = Logger {
        file,
        console: debug,
        level,
        write_lock: Mutex::new(()),
    };
    if log::set_boxed_logger(Box::new(logger)).is_ok() {
        log::set_max_level(if debug {
            LevelFilter::Debug
        } else {
            LevelFilter::Info
        });
    }
}
