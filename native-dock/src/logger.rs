//! App-wide file logger writing to `<install_root>/logs/kinetix.log`.
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    mem::zeroed,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};
use windows_sys::Win32::{Foundation::SYSTEMTIME, System::SystemInformation::GetLocalTime};

const MAX_LOG_BYTES: u64 = 2 * 1024 * 1024;

struct Logger {
    path: PathBuf,
    file: Option<File>,
    written: u64,
}

static LOGGER: OnceLock<Mutex<Logger>> = OnceLock::new();

impl Logger {
    fn open(path: PathBuf) -> Self {
        if let Some(dir) = path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        let written = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        let mut logger = Self {
            path,
            file: None,
            written,
        };
        logger.rotate_if_needed();
        logger.file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&logger.path)
            .ok();
        logger
    }

    fn rotate_if_needed(&mut self) {
        if self.written < MAX_LOG_BYTES {
            return;
        }
        self.file = None;
        let backup = self.path.with_extension("log.1");
        let _ = fs::remove_file(&backup);
        let _ = fs::rename(&self.path, &backup);
        self.written = 0;
        self.file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .ok();
    }

    fn write_line(&mut self, component: &str, level: &str, message: &str) {
        self.rotate_if_needed();
        let Some(file) = &mut self.file else {
            return;
        };
        let ts = timestamp();
        let clean = message.trim_end_matches(&['\r', '\n'][..]);
        let line = format!("[{ts}] [{level}] [{component}] {clean}\n");
        if file.write_all(line.as_bytes()).is_ok() {
            let _ = file.flush();
            self.written += line.len() as u64;
        }
    }
}

fn timestamp() -> String {
    unsafe {
        let mut st: SYSTEMTIME = zeroed();
        GetLocalTime(&mut st);
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}",
            st.wYear, st.wMonth, st.wDay, st.wHour, st.wMinute, st.wSecond, st.wMilliseconds
        )
    }
}

pub fn init(root: &Path) {
    let path = root.join("logs").join("kinetix.log");
    let _ = LOGGER.get_or_init(|| Mutex::new(Logger::open(path)));
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        log("panic", "FATAL", &panic_info.to_string());
        prev(panic_info);
    }));
    log(
        "app",
        "INFO",
        &format!(
            "Kinetix v{} started (pid={}, root={})",
            env!("CARGO_PKG_VERSION"),
            std::process::id(),
            root.display()
        ),
    );
}

pub fn log(component: &str, level: &str, message: &str) {
    let Some(lock) = LOGGER.get() else {
        return;
    };
    if let Ok(mut guard) = lock.lock() {
        guard.write_line(component, level, message);
    }
}

pub fn info(component: &str, message: impl AsRef<str>) {
    log(component, "INFO", message.as_ref());
}

pub fn warn(component: &str, message: impl AsRef<str>) {
    log(component, "WARN", message.as_ref());
}

pub fn error(component: &str, message: impl AsRef<str>) {
    log(component, "ERROR", message.as_ref());
}

pub unsafe fn open_and_copy_log(hwnd: windows_sys::Win32::Foundation::HWND, root: &Path) {
    use windows_sys::Win32::{
        Foundation::*,
        System::{DataExchange::*, Memory::*},
        UI::{Shell::*, WindowsAndMessaging::SW_SHOWNORMAL},
    };
    let dir = root.join("logs");
    let _ = fs::create_dir_all(&dir);
    let log_path = dir.join("kinetix.log");
    if !log_path.is_file() {
        let _ = fs::write(&log_path, "");
    }
    let abs = fs::canonicalize(&log_path).unwrap_or(log_path);
    // Strip Windows extended-length prefix (\\?\) so Explorer / CF_HDROP consumers accept it cleanly.
    let clean_str = abs.to_string_lossy();
    let display_path = clean_str.strip_prefix(r"\\?\").unwrap_or(&clean_str);

    // 1. Copy the log file itself (CF_HDROP) so the user can paste it directly into chat/Explorer.
    let mut wide_list = crate::model::wide(display_path);
    wide_list.push(0); // double-null terminated
    let header_bytes = std::mem::size_of::<DROPFILES>();
    let total_bytes = header_bytes + wide_list.len() * 2;
    let hmem = GlobalAlloc(GMEM_MOVEABLE | GMEM_ZEROINIT, total_bytes);
    if !hmem.is_null() {
        let ptr = GlobalLock(hmem) as *mut u8;
        if !ptr.is_null() {
            let df = ptr as *mut DROPFILES;
            (*df).pFiles = header_bytes as u32;
            (*df).fWide = 1;
            std::ptr::copy_nonoverlapping(
                wide_list.as_ptr() as *const u8,
                ptr.add(header_bytes),
                wide_list.len() * 2,
            );
            GlobalUnlock(hmem);
            if OpenClipboard(hwnd) != 0 {
                EmptyClipboard();
                const CF_HDROP: u32 = 15;
                if SetClipboardData(CF_HDROP, hmem).is_null() {
                    GlobalFree(hmem);
                }
                CloseClipboard();
            } else {
                GlobalFree(hmem);
            }
        } else {
            GlobalFree(hmem);
        }
    }

    // 2. Open the logs folder in Explorer with kinetix.log selected.
    let params = crate::model::wide(&format!("/select,\"{display_path}\""));
    ShellExecuteW(
        hwnd,
        crate::model::wide("open").as_ptr(),
        crate::model::wide("explorer.exe").as_ptr(),
        params.as_ptr(),
        std::ptr::null(),
        SW_SHOWNORMAL,
    );
}
