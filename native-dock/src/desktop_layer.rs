//! Keep widget and fused decoration HWNDs below application windows immediately,
//! including activation/drag requests; a periodic pin alone permits visible jumps.
//!
//! Widgets sit directly ABOVE the desktop (Progman/WorkerW) and below the lowest
//! real application window. Clicking or right-clicking the desktop makes Explorer
//! reshuffle the z-order; a foreground WinEvent hook re-pins every member at once
//! (instead of waiting 1-60 s for per-widget timers), so widgets never vanish.
use std::{
    cell::{Cell, RefCell},
    ptr::{null, null_mut},
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED},
    UI::{
        Accessibility::{SetWinEventHook, HWINEVENTHOOK},
        Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
        WindowsAndMessaging::*,
    },
};
const ID: usize = 0x4c44534b;
/// Explorer finishes its own reordering a moment after focus moves; re-check then.
const SETTLE_RECHECK_MS: u32 = 120;
thread_local! {
    static MEMBERS: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };
    static HOOK: Cell<usize> = const { Cell::new(0) };
}
fn property() -> Vec<u16> {
    crate::model::wide("Kinetix.DesktopLayer")
}
pub unsafe fn contains(hwnd: HWND) -> bool {
    GetPropW(hwnd, property().as_ptr()) as usize == 1
}
pub unsafe fn register(hwnd: HWND, enabled: bool) {
    if contains(hwnd)==enabled {return;}
    if enabled {
        SetPropW(hwnd, property().as_ptr(), 1usize as HANDLE);
        SetWindowSubclass(hwnd, Some(proc), ID, 0);
        MEMBERS.with(|m| m.borrow_mut().push(hwnd as usize));
        ensure_foreground_hook();
    } else {
        RemovePropW(hwnd, property().as_ptr());
        RemoveWindowSubclass(hwnd, Some(proc), ID);
        MEMBERS.with(|m| m.borrow_mut().retain(|h| *h != hwnd as usize));
    }
}
fn class_name(hwnd: HWND) -> String {
    let mut name = [0u16; 128];
    let len = unsafe { GetClassNameW(hwnd, name.as_mut_ptr(), 128) }.max(0) as usize;
    String::from_utf16_lossy(&name[..len])
}
fn is_desktop_class(class: &str) -> bool {
    matches!(class, "Progman" | "WorkerW")
}
/// Cloaked windows (suspended UWP frames, DWM helper windows) report visible
/// but are never drawn; anchoring to them put widgets in unpredictable places.
unsafe fn is_cloaked(hwnd: HWND) -> bool {
    let mut cloaked = 0u32;
    DwmGetWindowAttribute(
        hwnd,
        DWMWA_CLOAKED as u32,
        &mut cloaked as *mut u32 as _,
        std::mem::size_of::<u32>() as u32,
    ) >= 0
        && cloaked != 0
}
/// The lowest real application window ABOVE the desktop surface, or HWND_TOP
/// (top of the non-topmost band, i.e. directly above the desktop) when none.
/// Never HWND_BOTTOM: that would push widgets underneath the wallpaper.
unsafe fn anchor(hwnd: HWND) -> HWND {
    let mut current = GetTopWindow(null_mut());
    let mut last = HWND_TOP;
    for _ in 0..4096 {
        if current.is_null() {
            break;
        }
        if current != hwnd && IsWindowVisible(current) != 0 {
            let class = class_name(current);
            if is_desktop_class(&class) {
                break;
            }
            if !contains(current)
                && !matches!(
                    class.as_str(),
                    "KinetixNativeGlassShadow" | "KinetixNativeFusionSurface"
                )
                && GetWindowLongPtrW(current, GWL_EXSTYLE) & WS_EX_TOPMOST as isize == 0
                && !is_cloaked(current)
            {
                last = current;
            }
        }
        current = GetWindow(current, GW_HWNDNEXT);
    }
    last
}
/// Visible members in current z-order (top first), and whether any of them has
/// fallen below the desktop surface.
unsafe fn members_in_z_order() -> (Vec<HWND>, bool) {
    let members = MEMBERS.with(|m| m.borrow().clone());
    let mut ordered = Vec::new();
    let mut below_desktop = false;
    if members.is_empty() {
        return (ordered, false);
    }
    let mut seen_desktop = false;
    let mut current = GetTopWindow(null_mut());
    for _ in 0..4096 {
        if current.is_null() {
            break;
        }
        if IsWindowVisible(current) != 0 {
            if members.contains(&(current as usize)) {
                below_desktop |= seen_desktop;
                ordered.push(current);
            } else if !seen_desktop && is_desktop_class(&class_name(current)) {
                seen_desktop = true;
            }
        }
        current = GetWindow(current, GW_HWNDNEXT);
    }
    (ordered, below_desktop)
}
/// Re-pin all members above the desktop, preserving their relative order
/// (fusion bridges intentionally sit behind their member widgets).
pub unsafe fn repin_if_buried() -> bool {
    let (ordered, below_desktop) = members_in_z_order();
    if !below_desktop {
        return false;
    }
    for hwnd in ordered.into_iter().rev() {
        pin(hwnd);
    }
    true
}
unsafe extern "system" fn settle_timer(_hwnd: HWND, _msg: u32, id: usize, _time: u32) {
    KillTimer(null_mut(), id);
    repin_if_buried();
}
unsafe extern "system" fn on_foreground(
    _hook: HWINEVENTHOOK,
    _event: u32,
    _hwnd: HWND,
    _object: i32,
    _child: i32,
    _thread: u32,
    _time: u32,
) {
    repin_if_buried();
    SetTimer(null_mut(), 0, SETTLE_RECHECK_MS, Some(settle_timer));
}
unsafe fn ensure_foreground_hook() {
    if HOOK.with(|h| h.get()) != 0 {
        return;
    }
    // Out-of-context: delivered through this UI thread's message loop, no DLL injection.
    let hook = SetWinEventHook(
        EVENT_SYSTEM_FOREGROUND,
        EVENT_SYSTEM_FOREGROUND,
        null_mut(),
        Some(on_foreground),
        0,
        0,
        WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
    );
    HOOK.with(|h| h.set(hook as usize));
}
pub unsafe fn pin(hwnd: HWND) {
    SetWindowPos(
        hwnd,
        anchor(hwnd),
        0,
        0,
        0,
        0,
        SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER,
    );
}
unsafe extern "system" fn proc(
    hwnd: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    match msg {
        WM_WINDOWPOSCHANGING => {
            let position = &mut *(lp as *mut WINDOWPOS);
            if position.flags & SWP_NOZORDER == 0 {
                // A bridge can deliberately sit behind its own member. Preserve
                // that ordering, while rejecting TOP/TOPMOST/application anchors.
                if position.hwndInsertAfter == hwnd || !contains(position.hwndInsertAfter) {
                    position.hwndInsertAfter = anchor(hwnd);
                }
                position.flags |= SWP_NOACTIVATE | SWP_NOOWNERZORDER;
            }
        }
        WM_NCDESTROY => {
            register(hwnd, false);
        }
        _ => {}
    }
    DefSubclassProc(hwnd, msg, wp, lp)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn activation_and_topmost_requests_keep_widgets_below_applications() {
        let _lock = crate::native_ui_test_lock();
        unsafe {
            let make = || {
                CreateWindowExW(
                    WS_EX_TOOLWINDOW,
                    crate::model::wide("STATIC").as_ptr(),
                    crate::model::wide("Kinetix desktop-layer test").as_ptr(),
                    WS_POPUP,
                    -20000,
                    -20000,
                    20,
                    20,
                    null_mut(),
                    null_mut(),
                    null_mut(),
                    null(),
                )
            };
            let app = make();
            let widget = make();
            let bridge = make();
            assert!(!app.is_null() && !widget.is_null() && !bridge.is_null());
            for h in [app, widget, bridge] {
                ShowWindow(h, SW_SHOWNOACTIVATE);
            }
            register(widget, true);
            register(bridge, true);
            pin(widget);
            SetWindowPos(widget, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE);
            assert_eq!(
                GetWindowLongPtrW(widget, GWL_EXSTYLE) & WS_EX_TOPMOST as isize,
                0
            );
            let mut cursor = GetWindow(app, GW_HWNDNEXT);
            let mut below = false;
            for _ in 0..4096 {
                if cursor.is_null() {
                    break;
                }
                if cursor == widget {
                    below = true;
                    break;
                }
                cursor = GetWindow(cursor, GW_HWNDNEXT);
            }
            assert!(below, "widget must remain below a normal application");
            SetWindowPos(
                bridge,
                widget,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
            assert_eq!(GetWindow(bridge, GW_HWNDPREV), widget);
            // Keep keyboard focus possible for Dock search; constrain stacking,
            // rather than making all widget input non-activating.
            for h in [bridge, widget, app] {
                DestroyWindow(h);
            }
        }
    }
}
