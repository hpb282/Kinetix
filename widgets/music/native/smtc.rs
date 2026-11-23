//! Windows System Media Transport Controls (SMTC) fallback client.
//! Runs WinRT calls on a dedicated MTA thread; never blocks the UI thread.
use sha2::{Digest, Sha256};
use std::{
    ffi::c_void,
    fs,
    mem::zeroed,
    path::{Path, PathBuf},
    ptr::{null_mut, NonNull},
    sync::{
        atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering},
        mpsc::{self, Receiver, Sender},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{FILETIME, S_OK},
    System::SystemInformation::GetSystemTimeAsFileTime,
};

#[derive(Clone, Debug, Default)]
pub struct SmtcState {
    pub has_session: bool,
    pub available: bool,
    pub playing: bool,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub position: f64,
    pub duration: f64,
    pub can_next: bool,
    pub can_previous: bool,
    pub can_seek: bool,
    pub art_url: Option<String>,
    pub art_file: Option<String>,
}

pub enum SmtcCommand {
    Toggle,
    Next,
    Previous,
    Seek(f64),
    Quit,
}

pub struct SmtcClient {
    tx: Sender<SmtcCommand>,
    pub updates: Receiver<SmtcState>,
    notify_hwnd: Arc<AtomicIsize>,
    notify_msg: Arc<AtomicU32>,
    notify_pending: Arc<AtomicBool>,
}

impl SmtcClient {
    pub fn start(root: &Path) -> Self {
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (upd_tx, upd_rx) = mpsc::channel();
        let art_dir = root.join("config/native-music-player/art");
        let notify_hwnd = Arc::new(AtomicIsize::new(0));
        let notify_msg = Arc::new(AtomicU32::new(0));
        let notify_pending = Arc::new(AtomicBool::new(false));
        let w_hwnd = Arc::clone(&notify_hwnd);
        let w_msg = Arc::clone(&notify_msg);
        let w_pending = Arc::clone(&notify_pending);
        let _ = thread::Builder::new()
            .name("kinetix-smtc".into())
            .spawn(move || worker_loop(art_dir, cmd_rx, upd_tx, w_hwnd, w_msg, w_pending));
        Self {
            tx: cmd_tx,
            updates: upd_rx,
            notify_hwnd,
            notify_msg,
            notify_pending,
        }
    }

    pub fn set_notify(&self, hwnd: windows_sys::Win32::Foundation::HWND, msg: u32) {
        self.notify_hwnd.store(hwnd as isize, Ordering::Release);
        self.notify_msg.store(msg, Ordering::Release);
        if hwnd.is_null() || msg == 0 {
            self.notify_pending.store(false, Ordering::Release);
        }
    }

    pub fn clear_notify_pending(&self) {
        self.notify_pending.store(false, Ordering::Release);
    }

    pub fn send(&self, cmd: SmtcCommand) {
        let _ = self.tx.send(cmd);
    }
}

impl Drop for SmtcClient {
    fn drop(&mut self) {
        let _ = self.tx.send(SmtcCommand::Quit);
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Guid {
    data1: u32,
    data2: u16,
    data3: u16,
    data4: [u8; 8],
}

impl Guid {
    const fn new(d1: u32, d2: u16, d3: u16, d4: [u8; 8]) -> Self {
        Self {
            data1: d1,
            data2: d2,
            data3: d3,
            data4: d4,
        }
    }
}

type HString = *mut c_void;

#[repr(C)]
struct HStringHeader {
    reserved: [u8; 24],
}

#[link(name = "runtimeobject")]
extern "system" {
    fn RoInitialize(init_type: u32) -> i32;
    fn RoUninitialize();
    fn RoGetActivationFactory(
        activatable_class_id: HString,
        iid: *const Guid,
        factory: *mut *mut c_void,
    ) -> i32;
    fn WindowsCreateStringReference(
        source_string: *const u16,
        length: u32,
        hstring_header: *mut HStringHeader,
        string: *mut HString,
    ) -> i32;
    fn WindowsGetStringRawBuffer(string: HString, length: *mut u32) -> *const u16;
    fn WindowsDeleteString(string: HString) -> i32;
}

const IID_ASYNC_INFO: Guid = Guid::new(0x0000_0036, 0x0000, 0x0000, [0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46]);
const IID_SMTC_MANAGER_STATICS: Guid = Guid::new(0x2050_C4EE, 0x11A0, 0x57DE, [0xAE, 0xD7, 0xC9, 0x7C, 0x70, 0x33, 0x82, 0x45]);
const IID_IINPUT_STREAM: Guid = Guid::new(0x905A_0FE2, 0xBC53, 0x11DF, [0x8C, 0x49, 0x00, 0x1E, 0x4F, 0xC6, 0x86, 0xDA]);
const IID_IBUFFER: Guid = Guid::new(0x905A_0FE0, 0xBC53, 0x11DF, [0x8C, 0x49, 0x00, 0x1E, 0x4F, 0xC6, 0x86, 0xDA]);
const IID_IBUFFER_FACTORY: Guid = Guid::new(0x71AF_914D, 0xC10F, 0x484B, [0xBC, 0x50, 0x14, 0xBC, 0x62, 0x3B, 0x3A, 0x27]);
const IID_IBUFFER_BYTE_ACCESS: Guid = Guid::new(0x905A_0FEF, 0xBC53, 0x11DF, [0x8C, 0x49, 0x00, 0x1E, 0x4F, 0xC6, 0x86, 0xDA]);

struct ComPtr(NonNull<c_void>);

impl ComPtr {
    unsafe fn from_raw(ptr: *mut c_void) -> Option<Self> {
        NonNull::new(ptr).map(Self)
    }

    fn as_ptr(&self) -> *mut c_void {
        self.0.as_ptr()
    }

    unsafe fn vtable(&self) -> *const *const usize {
        *(self.0.as_ptr() as *const *const *const usize)
    }

    unsafe fn slot(&self, index: usize) -> *const usize {
        *self.vtable().add(index)
    }

    unsafe fn qi(&self, iid: &Guid) -> Option<Self> {
        type QueryInterfaceFn = unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> i32;
        let func: QueryInterfaceFn = std::mem::transmute(self.slot(0));
        let mut out = null_mut();
        if func(self.as_ptr(), iid, &mut out) >= 0 {
            Self::from_raw(out)
        } else {
            None
        }
    }
}

impl Drop for ComPtr {
    fn drop(&mut self) {
        unsafe {
            type ReleaseFn = unsafe extern "system" fn(*mut c_void) -> u32;
            let func: ReleaseFn = std::mem::transmute(self.slot(2));
            func(self.as_ptr());
        }
    }
}

unsafe fn with_hstring<T>(utf16: &[u16], f: impl FnOnce(HString) -> T) -> Result<T, i32> {
    let mut header: HStringHeader = zeroed();
    let mut hs: HString = null_mut();
    let len = utf16.iter().position(|&c| c == 0).unwrap_or(utf16.len()) as u32;
    let hr = WindowsCreateStringReference(utf16.as_ptr(), len, &mut header, &mut hs);
    if hr < 0 {
        return Err(hr);
    }
    Ok(f(hs))
}

unsafe fn hstring_to_string(hs: HString) -> String {
    if hs.is_null() {
        return String::new();
    }
    let mut len = 0u32;
    let ptr = WindowsGetStringRawBuffer(hs, &mut len);
    let text = if !ptr.is_null() && len > 0 {
        String::from_utf16_lossy(std::slice::from_raw_parts(ptr, len as usize))
    } else {
        String::new()
    };
    let _ = WindowsDeleteString(hs);
    text
}

unsafe fn wait_async(op: &ComPtr) -> bool {
    let Some(info) = op.qi(&IID_ASYNC_INFO) else {
        return false;
    };
    type GetStatusFn = unsafe extern "system" fn(*mut c_void, *mut i32) -> i32;
    let get_status: GetStatusFn = std::mem::transmute(info.slot(7));
    let deadline = Instant::now() + Duration::from_millis(1500);
    loop {
        let mut status = 0i32;
        if get_status(info.as_ptr(), &mut status) < 0 {
            return false;
        }
        match status {
            1 => return true,  // Completed
            2 | 3 => return false, // Canceled / Error
            _ => {
                if Instant::now() >= deadline {
                    return false;
                }
                thread::sleep(Duration::from_millis(2));
            }
        }
    }
}

unsafe fn await_ptr_op(op: &ComPtr, results_slot: usize) -> Option<ComPtr> {
    if !wait_async(op) {
        return None;
    }
    type GetResultsFn = unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> i32;
    let get_results: GetResultsFn = std::mem::transmute(op.slot(results_slot));
    let mut out = null_mut();
    if get_results(op.as_ptr(), &mut out) >= 0 {
        ComPtr::from_raw(out)
    } else {
        None
    }
}

unsafe fn await_bool_op(op: &ComPtr) -> bool {
    if !wait_async(op) {
        return false;
    }
    type GetResultsFn = unsafe extern "system" fn(*mut c_void, *mut u8) -> i32;
    let get_results: GetResultsFn = std::mem::transmute(op.slot(8));
    let mut out = 0u8;
    get_results(op.as_ptr(), &mut out) >= 0 && out != 0
}

unsafe fn get_session_manager() -> Option<ComPtr> {
    let class_id: Vec<u16> = "Windows.Media.Control.GlobalSystemMediaTransportControlsSessionManager\0"
        .encode_utf16()
        .collect();
    let mut factory_ptr = null_mut();
    let hr = with_hstring(&class_id, |hs| {
        RoGetActivationFactory(hs, &IID_SMTC_MANAGER_STATICS, &mut factory_ptr)
    })
    .ok()?;
    if hr < 0 {
        return None;
    }
    let factory = ComPtr::from_raw(factory_ptr)?;
    type RequestAsyncFn = unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> i32;
    let request_async: RequestAsyncFn = std::mem::transmute(factory.slot(6));
    let mut op_ptr = null_mut();
    if request_async(factory.as_ptr(), &mut op_ptr) < 0 {
        return None;
    }
    let op = ComPtr::from_raw(op_ptr)?;
    await_ptr_op(&op, 8)
}

unsafe fn get_current_session(manager: &ComPtr) -> Option<ComPtr> {
    type GetCurrentSessionFn = unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> i32;
    let func: GetCurrentSessionFn = std::mem::transmute(manager.slot(6));
    let mut ptr = null_mut();
    if func(manager.as_ptr(), &mut ptr) < 0 {
        return None;
    }
    ComPtr::from_raw(ptr)
}

unsafe fn read_thumbnail_bytes(thumb_ref: &ComPtr) -> Option<Vec<u8>> {
    type OpenReadAsyncFn = unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> i32;
    let open_read: OpenReadAsyncFn = std::mem::transmute(thumb_ref.slot(6));
    let mut op_ptr = null_mut();
    if open_read(thumb_ref.as_ptr(), &mut op_ptr) < 0 {
        return None;
    }
    let op = ComPtr::from_raw(op_ptr)?;
    let stream = await_ptr_op(&op, 8)?;
    let input = stream.qi(&IID_IINPUT_STREAM)?;

    let buf_class: Vec<u16> = "Windows.Storage.Streams.Buffer\0".encode_utf16().collect();
    let mut factory_ptr = null_mut();
    let hr = with_hstring(&buf_class, |hs| {
        RoGetActivationFactory(hs, &IID_IBUFFER_FACTORY, &mut factory_ptr)
    })
    .ok()?;
    if hr < 0 {
        return None;
    }
    let factory = ComPtr::from_raw(factory_ptr)?;
    type CreateBufFn = unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> i32;
    let create_buf: CreateBufFn = std::mem::transmute(factory.slot(6));
    const CHUNK: u32 = 256 * 1024;
    const MAX_BYTES: usize = 4 * 1024 * 1024;
    let mut buf_ptr = null_mut();
    if create_buf(factory.as_ptr(), CHUNK, &mut buf_ptr) < 0 {
        return None;
    }
    let buffer = ComPtr::from_raw(buf_ptr)?;

    type ReadAsyncFn = unsafe extern "system" fn(*mut c_void, *mut c_void, u32, i32, *mut *mut c_void) -> i32;
    let read_async: ReadAsyncFn = std::mem::transmute(input.slot(6));
    type GetLengthFn = unsafe extern "system" fn(*mut c_void, *mut u32) -> i32;
    type BufferPtrFn = unsafe extern "system" fn(*mut c_void, *mut *mut u8) -> i32;

    let mut result = Vec::new();
    loop {
        let mut read_op_ptr = null_mut();
        if read_async(input.as_ptr(), buffer.as_ptr(), CHUNK, 0, &mut read_op_ptr) < 0 {
            break;
        }
        let Some(read_op) = ComPtr::from_raw(read_op_ptr) else {
            break;
        };
        let Some(filled) = await_ptr_op(&read_op, 10) else {
            break;
        };
        let Some(ibuf) = filled.qi(&IID_IBUFFER) else {
            break;
        };
        let get_len: GetLengthFn = std::mem::transmute(ibuf.slot(7));
        let mut len = 0u32;
        if get_len(ibuf.as_ptr(), &mut len) < 0 || len == 0 {
            break;
        }
        let Some(access) = filled.qi(&IID_IBUFFER_BYTE_ACCESS) else {
            break;
        };
        let get_ptr: BufferPtrFn = std::mem::transmute(access.slot(3));
        let mut data = null_mut();
        if get_ptr(access.as_ptr(), &mut data) < 0 || data.is_null() {
            break;
        }
        if result.len() + len as usize > MAX_BYTES {
            return None;
        }
        result.extend_from_slice(std::slice::from_raw_parts(data, len as usize));
        if (len as u32) < CHUNK {
            break;
        }
    }
    if result.is_empty() {
        None
    } else {
        Some(result)
    }
}

fn cache_thumbnail(art_dir: &Path, bytes: &[u8]) -> Option<(String, String)> {
    let hex = format!("{:x}", Sha256::digest(bytes));
    if hex.len() != 64 {
        return None;
    }
    let file = format!("{hex}.img");
    let dest = art_dir.join(&file);
    if dest.is_file() {
        if let Ok(opened) = fs::OpenOptions::new().write(true).open(&dest) {
            let _ = opened.set_modified(std::time::SystemTime::now());
        }
    } else {
        fs::create_dir_all(art_dir).ok()?;
        let tmp = art_dir.join(format!("{file}.tmp"));
        fs::write(&tmp, bytes).ok()?;
        fs::rename(&tmp, &dest).ok()?;
        super::session::prune_art_dir(art_dir, super::session::MAX_ART_FILES);
    }
    Some((format!("smtc:{hex}"), file))
}

unsafe fn poll_session(
    session: &ComPtr,
    art_dir: &Path,
    cached_key: &mut String,
    cached_art: &mut Option<(String, String)>,
    timeline_start: &mut i64,
) -> Option<SmtcState> {
    // 1. PlaybackInfo & Controls (slots: get_PlaybackInfo = 9)
    type GetPlaybackInfoFn = unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> i32;
    let get_pb: GetPlaybackInfoFn = std::mem::transmute(session.slot(9));
    let mut pb_ptr = null_mut();
    if get_pb(session.as_ptr(), &mut pb_ptr) < 0 {
        return None;
    }
    let pb = ComPtr::from_raw(pb_ptr)?;
    type GetControlsFn = unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> i32;
    type GetStatusFn = unsafe extern "system" fn(*mut c_void, *mut i32) -> i32;
    let get_controls: GetControlsFn = std::mem::transmute(pb.slot(6));
    let get_status: GetStatusFn = std::mem::transmute(pb.slot(7));
    let mut status = 0i32;
    let _ = get_status(pb.as_ptr(), &mut status);
    let playing = status == 4;

    let mut ctrl_ptr = null_mut();
    let (mut can_next, mut can_prev, mut can_seek) = (false, false, false);
    if get_controls(pb.as_ptr(), &mut ctrl_ptr) >= 0 {
        if let Some(ctrl) = ComPtr::from_raw(ctrl_ptr) {
            type GetBoolFn = unsafe extern "system" fn(*mut c_void, *mut u8) -> i32;
            let mut b = 0u8;
            let get_next: GetBoolFn = std::mem::transmute(ctrl.slot(12));
            if get_next(ctrl.as_ptr(), &mut b) >= 0 {
                can_next = b != 0;
            }
            let get_prev: GetBoolFn = std::mem::transmute(ctrl.slot(13));
            if get_prev(ctrl.as_ptr(), &mut b) >= 0 {
                can_prev = b != 0;
            }
            let get_seek: GetBoolFn = std::mem::transmute(ctrl.slot(20));
            if get_seek(ctrl.as_ptr(), &mut b) >= 0 {
                can_seek = b != 0;
            }
        }
    }

    // 2. TimelineProperties (slot 8)
    type GetTimelineFn = unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> i32;
    let get_tl: GetTimelineFn = std::mem::transmute(session.slot(8));
    let mut tl_ptr = null_mut();
    let (mut position, mut duration) = (0.0f64, 0.0f64);
    if get_tl(session.as_ptr(), &mut tl_ptr) >= 0 {
        if let Some(tl) = ComPtr::from_raw(tl_ptr) {
            type GetI64Fn = unsafe extern "system" fn(*mut c_void, *mut i64) -> i32;
            let (mut start, mut end, mut pos, mut last_upd) = (0i64, 0i64, 0i64, 0i64);
            let get_start: GetI64Fn = std::mem::transmute(tl.slot(6));
            let get_end: GetI64Fn = std::mem::transmute(tl.slot(7));
            let get_pos: GetI64Fn = std::mem::transmute(tl.slot(10));
            let get_upd: GetI64Fn = std::mem::transmute(tl.slot(11));
            let _ = get_start(tl.as_ptr(), &mut start);
            let _ = get_end(tl.as_ptr(), &mut end);
            let _ = get_pos(tl.as_ptr(), &mut pos);
            let _ = get_upd(tl.as_ptr(), &mut last_upd);
            *timeline_start = start;
            duration = ((end - start).max(0) as f64) / 10_000_000.0;
            let mut extrapolated = pos - start;
            if playing && last_upd > 0 {
                let mut ft: FILETIME = zeroed();
                GetSystemTimeAsFileTime(&mut ft);
                let now = ((ft.dwHighDateTime as i64) << 32) | (ft.dwLowDateTime as i64);
                let delta = (now - last_upd).clamp(0, 86_400 * 10_000_000);
                extrapolated += delta;
            }
            position = (extrapolated.max(0) as f64 / 10_000_000.0).min(duration.max(0.0));
        }
    }

    // 3. MediaProperties (slot 7 -> TryGetMediaPropertiesAsync)
    type TryGetPropsFn = unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> i32;
    let try_props: TryGetPropsFn = std::mem::transmute(session.slot(7));
    let mut op_ptr = null_mut();
    if try_props(session.as_ptr(), &mut op_ptr) < 0 {
        return None;
    }
    let op = ComPtr::from_raw(op_ptr)?;
    let props = await_ptr_op(&op, 8)?;
    type GetHStringFn = unsafe extern "system" fn(*mut c_void, *mut HString) -> i32;
    let mut hs: HString = null_mut();
    let get_title: GetHStringFn = std::mem::transmute(props.slot(6));
    let title = if get_title(props.as_ptr(), &mut hs) >= 0 {
        hstring_to_string(hs)
    } else {
        String::new()
    };
    hs = null_mut();
    let get_artist: GetHStringFn = std::mem::transmute(props.slot(9));
    let artist = if get_artist(props.as_ptr(), &mut hs) >= 0 {
        hstring_to_string(hs)
    } else {
        String::new()
    };
    hs = null_mut();
    let get_album: GetHStringFn = std::mem::transmute(props.slot(10));
    let album = if get_album(props.as_ptr(), &mut hs) >= 0 {
        hstring_to_string(hs)
    } else {
        String::new()
    };

    let key = format!("{title}\0{artist}\0{album}");
    if key != *cached_key {
        *cached_key = key;
        *cached_art = None;
        type GetThumbFn = unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> i32;
        let get_thumb: GetThumbFn = std::mem::transmute(props.slot(15));
        let mut thumb_ptr = null_mut();
        if get_thumb(props.as_ptr(), &mut thumb_ptr) >= 0 {
            if let Some(thumb_ref) = ComPtr::from_raw(thumb_ptr) {
                if let Some(bytes) = read_thumbnail_bytes(&thumb_ref) {
                    *cached_art = cache_thumbnail(art_dir, &bytes);
                }
            }
        }
    }

    let (art_url, art_file) = match cached_art {
        Some((u, f)) => (Some(u.clone()), Some(f.clone())),
        None => (None, None),
    };

    Some(SmtcState {
        has_session: true,
        available: !title.is_empty(),
        playing,
        title,
        artist,
        album,
        position,
        duration,
        can_next,
        can_previous: can_prev,
        can_seek: can_seek && duration > 0.0,
        art_url,
        art_file,
    })
}

unsafe fn run_command(session: &ComPtr, cmd: &SmtcCommand, timeline_start: i64) {
    type ActionOpFn = unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> i32;
    let slot = match cmd {
        SmtcCommand::Toggle => 20,   // TryTogglePlayPauseAsync
        SmtcCommand::Next => 16,     // TrySkipNextAsync
        SmtcCommand::Previous => 17, // TrySkipPreviousAsync
        SmtcCommand::Seek(sec) => {
            type SeekFn = unsafe extern "system" fn(*mut c_void, i64, *mut *mut c_void) -> i32;
            let seek_fn: SeekFn = std::mem::transmute(session.slot(24));
            let target = timeline_start + (*sec * 10_000_000.0) as i64;
            let mut op_ptr = null_mut();
            if seek_fn(session.as_ptr(), target, &mut op_ptr) >= 0 {
                if let Some(op) = ComPtr::from_raw(op_ptr) {
                    let _ = await_bool_op(&op);
                }
            }
            return;
        }
        SmtcCommand::Quit => return,
    };
    let func: ActionOpFn = std::mem::transmute(session.slot(slot));
    let mut op_ptr = null_mut();
    if func(session.as_ptr(), &mut op_ptr) >= 0 {
        if let Some(op) = ComPtr::from_raw(op_ptr) {
            let _ = await_bool_op(&op);
        }
    }
}

fn post_notify(hwnd: &AtomicIsize, msg: &AtomicU32, pending: &AtomicBool) {
    if !pending.swap(true, Ordering::AcqRel) {
        let h = hwnd.load(Ordering::Acquire);
        let m = msg.load(Ordering::Acquire);
        if h != 0 && m != 0 {
            unsafe {
                windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW(
                    h as windows_sys::Win32::Foundation::HWND,
                    m,
                    0,
                    0,
                );
            }
        } else {
            pending.store(false, Ordering::Release);
        }
    }
}

fn worker_loop(
    art_dir: PathBuf,
    cmd_rx: Receiver<SmtcCommand>,
    upd_tx: Sender<SmtcState>,
    notify_hwnd: Arc<AtomicIsize>,
    notify_msg: Arc<AtomicU32>,
    notify_pending: Arc<AtomicBool>,
) {
    unsafe {
        let ro_ok = RoInitialize(1) >= 0; // RO_INIT_MULTITHREADED
        let manager = get_session_manager();
        let mut cached_key = String::new();
        let mut cached_art: Option<(String, String)> = None;
        let mut timeline_start = 0i64;
        let mut last_had_session = false;

        'outer: loop {
            let mut ran_cmd = false;
            while let Ok(cmd) = cmd_rx.try_recv() {
                if matches!(cmd, SmtcCommand::Quit) {
                    break 'outer;
                }
                if let Some(mgr) = &manager {
                    if let Some(session) = get_current_session(mgr) {
                        run_command(&session, &cmd, timeline_start);
                        ran_cmd = true;
                    }
                }
            }

            if let Some(mgr) = &manager {
                if let Some(session) = get_current_session(mgr) {
                    if let Some(state) = poll_session(
                        &session,
                        &art_dir,
                        &mut cached_key,
                        &mut cached_art,
                        &mut timeline_start,
                    ) {
                        last_had_session = true;
                        if upd_tx.send(state).is_err() {
                            break 'outer;
                        }
                        post_notify(&notify_hwnd, &notify_msg, &notify_pending);
                    }
                } else if last_had_session || ran_cmd {
                    last_had_session = false;
                    cached_key.clear();
                    cached_art = None;
                    if upd_tx.send(SmtcState::default()).is_err() {
                        break 'outer;
                    }
                    post_notify(&notify_hwnd, &notify_msg, &notify_pending);
                }
            }

            match cmd_rx.recv_timeout(Duration::from_millis(if ran_cmd { 80 } else { 400 })) {
                Ok(SmtcCommand::Quit) => break 'outer,
                Ok(cmd) => {
                    if let Some(mgr) = &manager {
                        if let Some(session) = get_current_session(mgr) {
                            run_command(&session, &cmd, timeline_start);
                        }
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break 'outer,
            }
        }

        drop(manager);
        if ro_ok {
            let _ = S_OK;
            RoUninitialize();
        }
    }
}
