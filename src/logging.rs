//! The log: every line also goes to `logs/openmapper.log` in openOMSI's data folder (a
//! program without a console, as on Windows, would otherwise lose it), the last run's log is
//! kept as `openmapper.previous.log`, and a crash writes its message, place and backtrace
//! there before the program ends.

use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

static FILE: OnceLock<Mutex<std::fs::File>> = OnceLock::new();
/// The program has a window (not `--mcp`, `--check` and the like): a crash is told in a dialog.
static WINDOWED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The window is about to open: a crash from now on is told to the person at the screen.
pub fn windowed() {
    WINDOWED.store(true, std::sync::atomic::Ordering::Relaxed);
}
static PATH: OnceLock<PathBuf> = OnceLock::new();

/// Where the log is.
pub fn path() -> Option<&'static PathBuf> {
    PATH.get()
}

/// A writer for env_logger: the log file and the console.
struct Tee;

impl Write for Tee {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let _ = std::io::stderr().write_all(buf);
        if let Some(f) = FILE.get() {
            let mut f = f.lock().unwrap_or_else(|e| e.into_inner());
            let _ = f.write_all(buf);
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        if let Some(f) = FILE.get() {
            let _ = f.lock().unwrap_or_else(|e| e.into_inner()).flush();
        }
        Ok(())
    }
}

fn open() {
    let dir = crate::install::openomsi_data_dir().join("logs");
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let path = dir.join("openmapper.log");
    let _ = std::fs::rename(&path, dir.join("openmapper.previous.log"));
    if let Ok(f) = std::fs::File::create(&path) {
        let _ = FILE.set(Mutex::new(f));
        let _ = PATH.set(path);
    }
}

/// Start the log and the crash report. `filter` is the default level (RUST_LOG overrides).
pub fn init(filter: &str) {
    open();
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(filter)).target(env_logger::Target::Pipe(Box::new(Tee))).init();
    log::info!("openMapper {} on {} {}", env!("CARGO_PKG_VERSION"), std::env::consts::OS, std::env::consts::ARCH);
    std::panic::set_hook(Box::new(|info| {
        let thread = std::thread::current();
        let place = info.location().map(|l| format!("{}:{}", l.file(), l.line())).unwrap_or_default();
        let message = info.payload().downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| info.payload().downcast_ref::<String>().cloned()).unwrap_or_else(|| "?".into());
        let backtrace = std::backtrace::Backtrace::force_capture();
        log::error!("panic in thread '{}' at {place}: {message}\n{backtrace}", thread.name().unwrap_or("?"));
        // (a thread of the program that is caught carries on; only the window's own thread
        // ending is the end of the program. With no window - the MCP server, a check - there is
        // no one to click the dialog away, and the server hung on it instead of ending)
        if thread.name() == Some("main") && WINDOWED.load(std::sync::atomic::Ordering::Relaxed) {
            if let Some(p) = path() {
                rfd::MessageDialog::new()
                    .set_level(rfd::MessageLevel::Error)
                    .set_title("openMapper")
                    .set_description(format!("{} {message}\n\n{} {}", crate::i18n::tr("openMapper has stopped:", "openMapper остановился:"), crate::i18n::tr("The log with the details:", "Журнал с подробностями:"), p.display()))
                    .show();
            }
        }
    }));
}

static STAGE: Mutex<String> = Mutex::new(String::new());
static BEAT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn now_ms() -> u64 {
    static START: OnceLock<std::time::Instant> = OnceLock::new();
    START.get_or_init(std::time::Instant::now).elapsed().as_millis() as u64
}

/// Note what the program is about to do (the watchdog names it when the window stops
/// answering) and log it.
pub fn stage(what: &str) {
    log::info!("start: {what}");
    *STAGE.lock().unwrap_or_else(|e| e.into_inner()) = what.to_string();
    beat();
}

/// The window's thread is alive (called once a frame).
pub fn beat() {
    BEAT.store(now_ms(), std::sync::atomic::Ordering::Relaxed);
}

/// What the window's thread is taken up with for a while (opening a map, saving it, a grab
/// of many pieces): the watchdog names it when the window stops answering, so a log of a long
/// wait says what it was waiting for. `busy("")` when that is done.
pub fn busy(what: &str) {
    *STAGE.lock().unwrap_or_else(|e| e.into_inner()) = what.to_string();
}

/// A thread that writes to the log when the window's thread has not drawn a frame for ten
/// seconds, with the stage it was last at: a program that hangs at its start then leaves a
/// log saying where.
pub fn watchdog() {
    beat();
    let _ = std::thread::Builder::new().name("watchdog".into()).spawn(|| {
        let mut told = 0;
        loop {
            std::thread::sleep(std::time::Duration::from_secs(5));
            let quiet = now_ms().saturating_sub(BEAT.load(std::sync::atomic::Ordering::Relaxed)) / 1000;
            if quiet >= 10 && quiet / 30 >= told {
                told = quiet / 30 + 1;
                let stage = STAGE.lock().unwrap_or_else(|e| e.into_inner()).clone();
                log::warn!("the window has not answered for {quiet} s; last stage: {}", if stage.is_empty() { "(before the window)" } else { &stage });
                if let Some(f) = FILE.get() {
                    let _ = f.lock().unwrap_or_else(|e| e.into_inner()).sync_all();
                }
            } else if quiet < 10 {
                told = 0;
            }
        }
    });
}
