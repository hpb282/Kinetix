//! Shared window move/resize lifecycle, throttling, and shadow coordination
//! across all native widgets.

use crate::compositor::WindowShadow;
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::WindowsAndMessaging::WINDOWPOS;

pub fn enter_size_move(moving: &mut bool) {
    *moving = true;
}

pub fn use_live_nested_glass(hwnd: HWND, moving: bool, resizing: bool) -> bool {
    moving || resizing || crate::fusion::is_moving(hwnd) || crate::glass::is_resizing(hwnd)
}

pub unsafe fn draw_translucent_nested_glass(
    c: &crate::render::Canvas,
    rect: [f32; 4],
    radius: f32,
    tint_color: u32,
    tint: f32,
) {
    let alpha = if crate::glass::frosted_glass() {
        (38. + tint * 0.72).round().clamp(38., 115.) as u8
    } else {
        255
    };
    c.round(rect, radius, tint_color, alpha, false, false);
    c.coating(rect, radius);
}

/// Draws the shared translucent Liquid Glass search surface above `backing`
/// at any widget opacity (including `100%`), restoring optical edge refraction
/// when stationary and keeping the search pill rendered above the backdrop.
#[allow(clippy::too_many_arguments)]
pub unsafe fn draw_search_glass(
    dc: windows_sys::Win32::Graphics::Gdi::HDC,
    dpi: f32,
    backing: Option<&crate::compositor::Buffer>,
    rect: [f32; 4],
    radius: f32,
    accent: u32,
    light: bool,
    opacity: f32,
    moving: bool,
    draw_pill_surface: bool,
) {
    if !moving && (opacity < 100. || crate::glass::liquid_glass()) {
        if let Some(bg) = backing {
            bg.restore_glass_card(dc, rect, radius);
        }
    }
    if draw_pill_surface {
        let c = crate::render::Canvas::new(dc, dpi);
        c.search_surface(rect, radius, accent, light);
        c.round(rect, radius, accent, 35, false, false);
    }
}

pub unsafe fn start_resize(hwnd: HWND, resizing: &mut bool, shadow: Option<&WindowShadow>) {
    *resizing = true;
    crate::glass::set_resizing(hwnd, true);
    if let Some(s) = shadow {
        s.hide();
    }
}

pub fn exit_size_move(hwnd: HWND, moving: &mut bool, resizing: &mut bool) {
    *moving = false;
    *resizing = false;
    crate::glass::set_resizing(hwnd, false);
}

pub fn should_paint_move(hwnd: HWND, resizing: bool) -> bool {
    !resizing
        && !crate::fusion::is_atomic_moving(hwnd)
        && crate::glass::frosted_glass()
        && crate::glass::move_paint_due(hwnd)
}

pub fn should_paint_resize(hwnd: HWND, busy: bool, had_frame: bool) -> bool {
    !busy || !had_frame || crate::glass::resize_paint_due(hwnd)
}

pub unsafe fn sync_shadow(
    hwnd: HWND,
    moving: bool,
    resizing: bool,
    shadow: &mut Option<WindowShadow>,
    dpi: f32,
    opacity: u8,
) {
    if resizing {
        if let Some(s) = shadow {
            s.hide();
        }
    } else if !moving {
        if shadow.is_none() {
            *shadow = WindowShadow::new(hwnd);
        }
        if let Some(s) = shadow {
            s.sync(hwnd, dpi, opacity);
        }
    }
}

pub unsafe fn on_window_pos_changed(
    wp: &WINDOWPOS,
    resizing: bool,
    shadow: &mut Option<WindowShadow>,
    dpi: f32,
) {
    if resizing {
        if let Some(s) = shadow {
            s.hide();
        }
    } else if let Some(s) = shadow {
        s.follow_position(wp, dpi);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compositor::Buffer;
    use crate::render::Canvas;
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::Foundation::{GetLastError, RECT};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, GetWindowRect, IsWindowVisible,
        RegisterClassW, SWP_NOACTIVATE, SWP_NOZORDER, SW_HIDE, SW_SHOWNOACTIVATE, ShowWindow,
        WNDCLASSW, WS_EX_LAYERED, WS_EX_TOOLWINDOW, WS_POPUP,
    };

    unsafe fn test_owner_window(x: i32, y: i32, w: i32, h: i32) -> HWND {
        let instance = GetModuleHandleW(null());
        let class_name: Vec<u16> = "KinetixInteractionTestOwner\0".encode_utf16().collect();
        let wc = WNDCLASSW {
            lpfnWndProc: Some(DefWindowProcW),
            hInstance: instance,
            lpszClassName: class_name.as_ptr(),
            ..std::mem::zeroed()
        };
        let _ = RegisterClassW(&wc);
        let _ = GetLastError();
        CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOOLWINDOW,
            class_name.as_ptr(),
            null(),
            WS_POPUP,
            x,
            y,
            w,
            h,
            null_mut(),
            null_mut(),
            instance,
            null(),
        )
    }

    #[test]
    fn shared_interaction_and_shadow_sync_match_in_normal_and_low_power() {
        let _ui = crate::native_ui_test_lock();
        let _render = crate::render::Runtime::start().expect("gdi+ runtime");
        let prev_low = crate::motion::low_power();
        let prev_frosted = crate::glass::frosted_glass();

        unsafe {
            let owner = test_owner_window(120, 140, 320, 220);
            assert!(!owner.is_null());
            ShowWindow(owner, SW_SHOWNOACTIVATE);

            for low_power in [false, true] {
                crate::motion::set_low_power(low_power);
                crate::glass::set_frosted_glass(true);

                let mut moving = false;
                let mut resizing = false;
                let mut shadow = WindowShadow::new(owner);
                assert!(shadow.is_some());

                // Idle: shadow syncs and becomes visible.
                sync_shadow(owner, moving, resizing, &mut shadow, 1., 255);
                let shadow_hwnd = shadow.as_ref().unwrap().hwnd;
                assert_ne!(IsWindowVisible(shadow_hwnd), 0);
                assert!(!use_live_nested_glass(owner, moving, resizing));

                // Move: shadow stays attached and follows WM_WINDOWPOSCHANGING / CHANGED.
                enter_size_move(&mut moving);
                assert!(use_live_nested_glass(owner, moving, resizing));
                let wp = WINDOWPOS {
                    hwnd: owner,
                    hwndInsertAfter: null_mut(),
                    x: 260,
                    y: 280,
                    cx: 320,
                    cy: 220,
                    flags: SWP_NOZORDER | SWP_NOACTIVATE,
                };
                on_window_pos_changed(&wp, resizing, &mut shadow, 1.);
                let mut sr: RECT = std::mem::zeroed();
                GetWindowRect(shadow_hwnd, &mut sr);
                assert_eq!(sr.left, 260 - 40);
                assert_eq!(sr.top, 280 - 40);
                assert_ne!(IsWindowVisible(shadow_hwnd), 0);

                // Resize: shadow hides immediately and glass::is_resizing is armed.
                start_resize(owner, &mut resizing, shadow.as_ref());
                assert!(crate::glass::is_resizing(owner));
                assert!(use_live_nested_glass(owner, moving, resizing));
                assert_eq!(IsWindowVisible(shadow_hwnd), 0);
                assert!(!should_paint_move(owner, resizing));

                // Release: shadow restores and resizing clears.
                exit_size_move(owner, &mut moving, &mut resizing);
                assert!(!crate::glass::is_resizing(owner));
                assert!(!use_live_nested_glass(owner, moving, resizing));
                sync_shadow(owner, moving, resizing, &mut shadow, 1., 255);
                assert_ne!(IsWindowVisible(shadow_hwnd), 0);
            }

            // Nested translucent glass stays visible when frosted is on and solid when off.
            for (frosted, expected_opaque) in [(true, false), (false, true)] {
                crate::glass::set_frosted_glass(frosted);
                let buf = Buffer::plain(120, 60, 1.).expect("plain buffer");
                let batch = crate::render::AlphaBatch::new(buf.dc, 1.);
                let c = Canvas::new(buf.dc, 1.);
                draw_translucent_nested_glass(&c, [4., 4., 116., 56.], 16., 0x00203040, 50.);
                drop(batch);
                crate::render::flush(buf.dc);
                let px = buf.snapshot();
                let center = ((30 * 120 + 60) * 4) as usize;
                if expected_opaque {
                    assert_eq!(px[center + 3], 255);
                } else {
                    assert!(
                        px[center + 3] > 20 && px[center + 3] < 200,
                        "expected translucent alpha, got {}",
                        px[center + 3]
                    );
                }
            }

            ShowWindow(owner, SW_HIDE);
            DestroyWindow(owner);
        }

        // Shared search glass renders curved liquid glass + tint above the backdrop
        // at both 100% and reduced widget opacities, without stamping stale backing
        // while the window is moving.
        let prev_liquid = crate::glass::liquid_glass();
        crate::glass::set_liquid_glass(true);
        unsafe {
            let backing = Buffer::plain(160, 80, 1.).expect("backing buffer");
            {
                let c = Canvas::new(backing.dc, 1.);
                c.round([0., 0., 160., 80.], 0., 0x001e3c72, 255, false, false);
                c.round([12., 12., 148., 44.], 8., 0x003468c8, 255, false, false);
            }
            crate::render::flush(backing.dc);
            let backing_px = backing.snapshot();

            // 100% opacity: curved rim + surface render distinctly on top of backdrop.
            let full = Buffer::plain(160, 80, 1.).expect("full frame");
            full.copy_from(backing.dc);
            draw_search_glass(
                full.dc,
                1.,
                Some(&backing),
                [16., 14., 144., 46.],
                16.,
                0x00583818,
                false,
                100.,
                false,
                true,
            );
            crate::render::flush(full.dc);
            let full_px = full.snapshot();
            let rim = ((16 * 160 + 24) * 4) as usize;
            let rim_diff = full_px[rim].abs_diff(backing_px[rim]) as u32
                + full_px[rim + 1].abs_diff(backing_px[rim + 1]) as u32
                + full_px[rim + 2].abs_diff(backing_px[rim + 2]) as u32;
            assert!(
                rim_diff > 6,
                "search glass rim must render visibly above backdrop at 100% opacity (diff={rim_diff})"
            );
            assert_eq!(full_px[rim + 3], 255);

            // Reduced opacity: glass pill restores full alpha over translucent chrome.
            let mut faded = Buffer::plain(160, 80, 1.).expect("faded frame");
            faded.copy_from(backing.dc);
            faded.background_alpha(180);
            draw_search_glass(
                faded.dc,
                1.,
                Some(&backing),
                [16., 14., 144., 46.],
                16.,
                0x00583818,
                false,
                70.,
                false,
                true,
            );
            crate::render::flush(faded.dc);
            let faded_px = faded.snapshot();
            let inside = ((30 * 160 + 80) * 4) as usize;
            let outside = ((70 * 160 + 80) * 4) as usize;
            assert_eq!(faded_px[inside + 3], 255);
            assert_eq!(faded_px[outside + 3], 180);
        }
        crate::glass::set_liquid_glass(prev_liquid);

        crate::motion::set_low_power(prev_low);
        crate::glass::set_frosted_glass(prev_frosted);
    }

    #[test]
    fn start_resize_followed_by_exit_size_move_clears_resizing_and_restores_shadow() {
        let _ui = crate::native_ui_test_lock();
        unsafe {
            let owner = test_owner_window(120, 140, 200, 160);
            assert!(!owner.is_null());
            ShowWindow(owner, SW_SHOWNOACTIVATE);

            let mut shadow = WindowShadow::new(owner);
            assert!(shadow.is_some());
            let mut resizing = false;
            let mut moving = false;

            start_resize(owner, &mut resizing, shadow.as_ref());
            assert!(resizing);
            assert!(crate::glass::is_resizing(owner));

            exit_size_move(owner, &mut moving, &mut resizing);
            assert!(!resizing);
            assert!(!moving);
            assert!(!crate::glass::is_resizing(owner));

            sync_shadow(owner, moving, resizing, &mut shadow, 1., 255);
            assert_ne!(IsWindowVisible(shadow.as_ref().unwrap().hwnd), 0);

            shadow.take();
            DestroyWindow(owner);
        }
    }
}
