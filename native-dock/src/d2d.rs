//! Direct2D 1.1 + DirectWrite native rendering and text measurement module.
//!
//! Incrementally replaces GDI+ vector/text drawing across Kinetix's native Rust
//! host without adding any external crates.
//!
//! Architecture:
//! - Links directly against `d2d1.dll` (`D2D1CreateFactory`) and `dwrite.dll`
//!   (`DWriteCreateFactory`) using RAII [`ComPtr`] wrappers and verified Windows
//!   SDK GUIDs/vtable indices.
//! - Supports `ID2D1Factory1::CreateDevice(dxgi_device)` -> `ID2D1DeviceContext`
//!   for direct D3D11/DXGI interop, and `ID2D1Factory::CreateDCRenderTarget`
//!   (premultiplied `BGRA8`) for hybrid HDC interop with `compositor::Buffer`.
//! - Uses `D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE` on DC targets so text renders
//!   with clean premultiplied alpha coverage over transparent glass surfaces.
//! - Sticky Notes intentionally keeps Win32 `RichEdit`
//!   (`widgets/notes/native/editor.rs`) for IME/clipboard/undo/rich text, while
//!   `d2d.rs` incrementally replaces GDI+.

#![allow(dead_code)]

use std::{
    cell::RefCell,
    collections::HashMap,
    ffi::c_void,
    ptr::{null, null_mut},
};
use windows_sys::Win32::{
    Foundation::RECT,
    Graphics::Gdi::{GdiFlush, HDC},
};

pub type HRESULT = i32;

const D2DERR_RECREATE_TARGET: HRESULT = 0x8899000Cu32 as i32;

const D2D1_FACTORY_TYPE_SINGLE_THREADED: u32 = 0;
const DWRITE_FACTORY_TYPE_SHARED: u32 = 0;

const DXGI_FORMAT_B8G8R8A8_UNORM: u32 = 87;
const D2D1_ALPHA_MODE_PREMULTIPLIED: u32 = 1;
const D2D1_RENDER_TARGET_TYPE_DEFAULT: u32 = 0;
const D2D1_RENDER_TARGET_USAGE_NONE: u32 = 0;
const D2D1_FEATURE_LEVEL_DEFAULT: u32 = 0;
const D2D1_DEVICE_CONTEXT_OPTIONS_NONE: u32 = 0;

const D2D1_ANTIALIAS_MODE_PER_PRIMITIVE: u32 = 0;
const D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE: u32 = 2;
const D2D1_DRAW_TEXT_OPTIONS_CLIP: u32 = 2;

const D2D1_BITMAP_OPTIONS_TARGET: u32 = 0x1;
const D2D1_BITMAP_OPTIONS_CANNOT_DRAW: u32 = 0x2;

const DWRITE_FONT_STYLE_NORMAL: u32 = 0;
const DWRITE_FONT_STYLE_ITALIC: u32 = 2;
const DWRITE_FONT_STRETCH_NORMAL: u32 = 5;
const DWRITE_TRIMMING_GRANULARITY_CHARACTER: u32 = 1;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct GUID {
    pub data1: u32,
    pub data2: u16,
    pub data3: u16,
    pub data4: [u8; 8],
}

// 06152247-6f50-465a-9245-118bfd3b6007
const IID_ID2D1FACTORY: GUID = GUID {
    data1: 0x06152247,
    data2: 0x6f50,
    data3: 0x465a,
    data4: [0x92, 0x45, 0x11, 0x8b, 0xfd, 0x3b, 0x60, 0x07],
};

// bb12d362-daee-4b9a-aa1d-14ba401cfa1f
const IID_ID2D1FACTORY1: GUID = GUID {
    data1: 0xbb12d362,
    data2: 0xdaee,
    data3: 0x4b9a,
    data4: [0xaa, 0x1d, 0x14, 0xba, 0x40, 0x1c, 0xfa, 0x1f],
};

// b859ee5a-d838-4b5b-a2e8-1adc7d93db48
const IID_IDWRITEFACTORY: GUID = GUID {
    data1: 0xb859ee5a,
    data2: 0xd838,
    data3: 0x4b5b,
    data4: [0xa2, 0xe8, 0x1a, 0xdc, 0x7d, 0x93, 0xdb, 0x48],
};

#[repr(C)]
#[derive(Clone, Copy)]
struct D2D1FactoryOptions {
    debug_level: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct D2D1PixelFormat {
    pub format: u32,
    pub alpha_mode: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct D2D1RenderTargetProperties {
    pub rt_type: u32,
    pub pixel_format: D2D1PixelFormat,
    pub dpi_x: f32,
    pub dpi_y: f32,
    pub usage: u32,
    pub min_level: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct D2D1BitmapProperties1 {
    pixel_format: D2D1PixelFormat,
    dpi_x: f32,
    dpi_y: f32,
    bitmap_options: u32,
    color_context: *mut c_void,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct D2D1ColorF {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl D2D1ColorF {
    /// Converts Kinetix's `0x00BBGGRR` color value + separate alpha byte into `D2D1ColorF`.
    #[inline]
    pub fn from_colorref(color: u32, alpha: u8) -> Self {
        Self {
            r: (color & 0xff) as f32 / 255.0,
            g: ((color >> 8) & 0xff) as f32 / 255.0,
            b: ((color >> 16) & 0xff) as f32 / 255.0,
            a: alpha as f32 / 255.0,
        }
    }

    /// Converts an `0xAARRGGBB` color value into `D2D1ColorF`.
    #[inline]
    pub fn from_argb(argb: u32) -> Self {
        Self {
            r: ((argb >> 16) & 0xff) as f32 / 255.0,
            g: ((argb >> 8) & 0xff) as f32 / 255.0,
            b: (argb & 0xff) as f32 / 255.0,
            a: ((argb >> 24) & 0xff) as f32 / 255.0,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct D2D1RectF {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct D2D1RoundedRect {
    pub rect: D2D1RectF,
    pub radius_x: f32,
    pub radius_y: f32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct D2D1Matrix3x2F {
    pub m11: f32,
    pub m12: f32,
    pub m21: f32,
    pub m22: f32,
    pub dx: f32,
    pub dy: f32,
}

impl D2D1Matrix3x2F {
    #[inline]
    pub const fn identity() -> Self {
        Self {
            m11: 1.0,
            m12: 0.0,
            m21: 0.0,
            m22: 1.0,
            dx: 0.0,
            dy: 0.0,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct DWriteTrimming {
    granularity: u32,
    delimiter: u32,
    delimiter_count: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct DWriteTextMetricsRaw {
    left: f32,
    top: f32,
    width: f32,
    width_including_trailing_whitespace: f32,
    height: f32,
    layout_width: f32,
    layout_height: f32,
    max_bidi_reordering_depth: u32,
    line_count: u32,
}

#[link(name = "d2d1")]
extern "system" {
    fn D2D1CreateFactory(
        factory_type: u32,
        riid: *const GUID,
        factory_options: *const D2D1FactoryOptions,
        pp_i_factory: *mut *mut c_void,
    ) -> HRESULT;
}

#[link(name = "dwrite")]
extern "system" {
    fn DWriteCreateFactory(
        factory_type: u32,
        iid: *const GUID,
        factory: *mut *mut c_void,
    ) -> HRESULT;
}

/// Safe RAII wrapper for a raw COM interface pointer.
pub struct ComPtr(*mut c_void);

impl ComPtr {
    /// Takes ownership of an already-AddRef'd COM pointer if non-null.
    ///
    /// # Safety
    /// `ptr` must be null or a valid COM interface pointer with an owned reference count.
    #[inline]
    pub unsafe fn from_raw(ptr: *mut c_void) -> Option<Self> {
        if ptr.is_null() {
            None
        } else {
            Some(Self(ptr))
        }
    }

    #[inline]
    pub fn as_ptr(&self) -> *mut c_void {
        self.0
    }

    #[inline]
    unsafe fn vtable(&self) -> *const *const c_void {
        *(self.0 as *const *const *const c_void)
    }

    /// Queries the COM object for `iid`, returning a new owned [`ComPtr`] on success.
    ///
    /// # Safety
    /// Underlying COM pointer must be valid.
    pub unsafe fn query_interface(&self, iid: &GUID) -> Option<ComPtr> {
        let f: extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT =
            std::mem::transmute(*self.vtable().add(0));
        let mut out = null_mut();
        if f(self.0, iid, &mut out) >= 0 && !out.is_null() {
            Some(ComPtr(out))
        } else {
            None
        }
    }
}

impl Clone for ComPtr {
    fn clone(&self) -> Self {
        unsafe {
            let add_ref: extern "system" fn(*mut c_void) -> u32 =
                std::mem::transmute(*self.vtable().add(1));
            add_ref(self.0);
        }
        Self(self.0)
    }
}

impl Drop for ComPtr {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                let release: extern "system" fn(*mut c_void) -> u32 =
                    std::mem::transmute(*self.vtable().add(2));
                release(self.0);
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum TextAlign {
    Leading = 0,
    Trailing = 1,
    Center = 2,
    Justified = 3,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum ParagraphAlign {
    Near = 0,
    Far = 1,
    Center = 2,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum WordWrap {
    Wrap = 0,
    NoWrap = 1,
    EmergencyBreak = 2,
    WholeWord = 3,
    Character = 4,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextStyle<'a> {
    pub font_family: &'a str,
    pub size: f32,
    pub weight: u16,
    pub italic: bool,
    pub align: TextAlign,
    pub valign: ParagraphAlign,
    pub wrap: WordWrap,
    pub trim_ellipsis: bool,
}

impl Default for TextStyle<'static> {
    fn default() -> Self {
        Self {
            font_family: "Segoe UI Variable Text",
            size: 13.0,
            weight: 400,
            italic: false,
            align: TextAlign::Leading,
            valign: ParagraphAlign::Near,
            wrap: WordWrap::NoWrap,
            trim_ellipsis: false,
        }
    }
}

impl<'a> TextStyle<'a> {
    #[inline]
    pub const fn ui(size: f32, weight: u16) -> TextStyle<'static> {
        TextStyle {
            font_family: "Segoe UI Variable Text",
            size,
            weight,
            italic: false,
            align: TextAlign::Leading,
            valign: ParagraphAlign::Near,
            wrap: WordWrap::NoWrap,
            trim_ellipsis: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TextMetrics {
    pub left: f32,
    pub top: f32,
    pub width: f32,
    pub width_including_trailing_whitespace: f32,
    pub height: f32,
    pub line_count: u32,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct FormatKey {
    family: String,
    size_q6: i32,
    weight: u16,
    italic: bool,
    align: u32,
    valign: u32,
    wrap: u32,
    trim_ellipsis: bool,
}

struct RuntimeState {
    d2d_factory: ComPtr,
    is_factory1: bool,
    dwrite_factory: ComPtr,
    dc_rt: Option<ComPtr>,
    dc_brush: Option<ComPtr>,
    formats: HashMap<FormatKey, ComPtr>,
}

impl RuntimeState {
    unsafe fn create() -> Option<Self> {
        let mut raw_d2d = null_mut();
        let mut is_factory1 = true;
        let hr = D2D1CreateFactory(
            D2D1_FACTORY_TYPE_SINGLE_THREADED,
            &IID_ID2D1FACTORY1,
            null(),
            &mut raw_d2d,
        );
        if hr < 0 || raw_d2d.is_null() {
            is_factory1 = false;
            let hr0 = D2D1CreateFactory(
                D2D1_FACTORY_TYPE_SINGLE_THREADED,
                &IID_ID2D1FACTORY,
                null(),
                &mut raw_d2d,
            );
            if hr0 < 0 || raw_d2d.is_null() {
                return None;
            }
        }
        let d2d_factory = ComPtr::from_raw(raw_d2d)?;

        let mut raw_dw = null_mut();
        let hr_dw = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED, &IID_IDWRITEFACTORY, &mut raw_dw);
        if hr_dw < 0 || raw_dw.is_null() {
            return None;
        }
        let dwrite_factory = ComPtr::from_raw(raw_dw)?;

        Some(Self {
            d2d_factory,
            is_factory1,
            dwrite_factory,
            dc_rt: None,
            dc_brush: None,
            formats: HashMap::new(),
        })
    }

    unsafe fn ensure_dc_target(&mut self) -> Option<(&ComPtr, &ComPtr)> {
        if self.dc_rt.is_none() {
            let props = D2D1RenderTargetProperties {
                rt_type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
                pixel_format: D2D1PixelFormat {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alpha_mode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                // 96 DPI maps 1 D2D unit directly to 1 device pixel in Buffer DIBs.
                dpi_x: 96.0,
                dpi_y: 96.0,
                usage: D2D1_RENDER_TARGET_USAGE_NONE,
                min_level: D2D1_FEATURE_LEVEL_DEFAULT,
            };
            // ID2D1Factory::CreateDCRenderTarget (vtable index 16)
            let create_dc_rt: extern "system" fn(
                *mut c_void,
                *const D2D1RenderTargetProperties,
                *mut *mut c_void,
            ) -> HRESULT = std::mem::transmute(*self.d2d_factory.vtable().add(16));
            let mut raw_rt = null_mut();
            if create_dc_rt(self.d2d_factory.as_ptr(), &props, &mut raw_rt) < 0 || raw_rt.is_null() {
                return None;
            }
            let rt = ComPtr::from_raw(raw_rt)?;
            let brush = rt_create_solid_brush(&rt, D2D1ColorF::from_colorref(0xffffff, 255))?;
            self.dc_rt = Some(rt);
            self.dc_brush = Some(brush);
        }
        Some((self.dc_rt.as_ref()?, self.dc_brush.as_ref()?))
    }

    unsafe fn text_format(&mut self, style: &TextStyle<'_>) -> Option<ComPtr> {
        let family_name = if style.font_family.trim().is_empty() {
            "Segoe UI Variable Text"
        } else {
            style.font_family.trim()
        };
        let size = style.size.max(1.0);
        let weight = style.weight.clamp(1, 999);
        let key = FormatKey {
            family: family_name.to_ascii_lowercase(),
            size_q6: (size * 64.0).round() as i32,
            weight,
            italic: style.italic,
            align: style.align as u32,
            valign: style.valign as u32,
            wrap: style.wrap as u32,
            trim_ellipsis: style.trim_ellipsis,
        };
        if let Some(cached) = self.formats.get(&key) {
            return Some(cached.clone());
        }
        if self.formats.len() >= 128 {
            self.formats.clear();
        }

        let wide_family = to_wide(family_name);
        let locale: [u16; 1] = [0];
        // IDWriteFactory::CreateTextFormat (vtable index 15)
        let create_fmt: extern "system" fn(
            *mut c_void,
            *const u16,
            *mut c_void,
            u32,
            u32,
            u32,
            f32,
            *const u16,
            *mut *mut c_void,
        ) -> HRESULT = std::mem::transmute(*self.dwrite_factory.vtable().add(15));
        let mut raw_fmt = null_mut();
        let hr = create_fmt(
            self.dwrite_factory.as_ptr(),
            wide_family.as_ptr(),
            null_mut(),
            weight as u32,
            if style.italic {
                DWRITE_FONT_STYLE_ITALIC
            } else {
                DWRITE_FONT_STYLE_NORMAL
            },
            DWRITE_FONT_STRETCH_NORMAL,
            size,
            locale.as_ptr(),
            &mut raw_fmt,
        );
        if hr < 0 || raw_fmt.is_null() {
            return None;
        }
        let fmt = ComPtr::from_raw(raw_fmt)?;

        // IDWriteTextFormat::SetTextAlignment (3), SetParagraphAlignment (4), SetWordWrapping (5)
        let set_align: extern "system" fn(*mut c_void, u32) -> HRESULT =
            std::mem::transmute(*fmt.vtable().add(3));
        let set_valign: extern "system" fn(*mut c_void, u32) -> HRESULT =
            std::mem::transmute(*fmt.vtable().add(4));
        let set_wrap: extern "system" fn(*mut c_void, u32) -> HRESULT =
            std::mem::transmute(*fmt.vtable().add(5));
        set_align(fmt.as_ptr(), style.align as u32);
        set_valign(fmt.as_ptr(), style.valign as u32);
        set_wrap(fmt.as_ptr(), style.wrap as u32);

        if style.trim_ellipsis {
            // IDWriteFactory::CreateEllipsisTrimmingSign (vtable index 20)
            let create_sign: extern "system" fn(
                *mut c_void,
                *mut c_void,
                *mut *mut c_void,
            ) -> HRESULT = std::mem::transmute(*self.dwrite_factory.vtable().add(20));
            let mut raw_sign = null_mut();
            if create_sign(self.dwrite_factory.as_ptr(), fmt.as_ptr(), &mut raw_sign) >= 0 {
                if let Some(sign) = ComPtr::from_raw(raw_sign) {
                    let trim = DWriteTrimming {
                        granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER,
                        delimiter: 0,
                        delimiter_count: 0,
                    };
                    // IDWriteTextFormat::SetTrimming (vtable index 9)
                    let set_trimming: extern "system" fn(
                        *mut c_void,
                        *const DWriteTrimming,
                        *mut c_void,
                    ) -> HRESULT = std::mem::transmute(*fmt.vtable().add(9));
                    set_trimming(fmt.as_ptr(), &trim, sign.as_ptr());
                }
            }
        }

        self.formats.insert(key, fmt.clone());
        Some(fmt)
    }

    unsafe fn create_text_layout(
        &mut self,
        text: &str,
        style: &TextStyle<'_>,
        max_width: f32,
        max_height: f32,
    ) -> Option<ComPtr> {
        let fmt = self.text_format(style)?;
        let utf16: Vec<u16> = text.encode_utf16().collect();
        let w = if max_width.is_finite() {
            max_width.max(0.0)
        } else {
            1_000_000.0
        };
        let h = if max_height.is_finite() {
            max_height.max(0.0)
        } else {
            1_000_000.0
        };
        // IDWriteFactory::CreateTextLayout (vtable index 18)
        let create_layout: extern "system" fn(
            *mut c_void,
            *const u16,
            u32,
            *mut c_void,
            f32,
            f32,
            *mut *mut c_void,
        ) -> HRESULT = std::mem::transmute(*self.dwrite_factory.vtable().add(18));
        let mut raw_layout = null_mut();
        let hr = create_layout(
            self.dwrite_factory.as_ptr(),
            utf16.as_ptr(),
            utf16.len() as u32,
            fmt.as_ptr(),
            w,
            h,
            &mut raw_layout,
        );
        if hr < 0 {
            return None;
        }
        ComPtr::from_raw(raw_layout)
    }

    unsafe fn create_text_layout_styled(
        &mut self,
        text: &str,
        style: &TextStyle<'_>,
        max_width: f32,
        max_height: f32,
        underline: bool,
        strike: bool,
    ) -> Option<ComPtr> {
        let layout = self.create_text_layout(text, style, max_width, max_height)?;
        let utf16_len = text.encode_utf16().count() as u32;
        if utf16_len > 0 {
            let range = pack_text_range(0, utf16_len);
            if underline {
                // IDWriteTextLayout::SetUnderline (vtable index 36)
                let set_underline: extern "system" fn(*mut c_void, i32, u64) -> HRESULT =
                    std::mem::transmute(*layout.vtable().add(36));
                let _ = set_underline(layout.as_ptr(), 1, range);
            }
            if strike {
                // IDWriteTextLayout::SetStrikethrough (vtable index 37)
                let set_strike: extern "system" fn(*mut c_void, i32, u64) -> HRESULT =
                    std::mem::transmute(*layout.vtable().add(37));
                let _ = set_strike(layout.as_ptr(), 1, range);
            }
        }
        Some(layout)
    }

    fn clear(&mut self) {
        self.dc_brush = None;
        self.dc_rt = None;
        self.formats.clear();
    }
}

thread_local! {
    static RUNTIME: RefCell<Option<RuntimeState>> = const { RefCell::new(None) };
}

fn with_runtime<R>(f: impl FnOnce(&mut RuntimeState) -> Option<R>) -> Option<R> {
    RUNTIME.with(|slot| {
        let mut borrow = slot.borrow_mut();
        if borrow.is_none() {
            *borrow = unsafe { RuntimeState::create() };
        }
        let state = borrow.as_mut()?;
        f(state)
    })
}

#[inline]
fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

#[inline]
fn pack_point_2f(x: f32, y: f32) -> u64 {
    (x.to_bits() as u64) | ((y.to_bits() as u64) << 32)
}

#[inline]
fn pack_text_range(start: u32, length: u32) -> u64 {
    (start as u64) | ((length as u64) << 32)
}

#[inline]
fn to_d2d_rect(r: [f32; 4]) -> D2D1RectF {
    D2D1RectF {
        left: r[0],
        top: r[1],
        right: r[2],
        bottom: r[3],
    }
}

unsafe fn rt_create_solid_brush(rt: &ComPtr, color: D2D1ColorF) -> Option<ComPtr> {
    // ID2D1RenderTarget::CreateSolidColorBrush (vtable index 8)
    let create_brush: extern "system" fn(
        *mut c_void,
        *const D2D1ColorF,
        *const c_void,
        *mut *mut c_void,
    ) -> HRESULT = std::mem::transmute(*rt.vtable().add(8));
    let mut raw_brush = null_mut();
    if create_brush(rt.as_ptr(), &color, null(), &mut raw_brush) < 0 {
        return None;
    }
    ComPtr::from_raw(raw_brush)
}

unsafe fn brush_set_color(brush: &ComPtr, color: D2D1ColorF) {
    // ID2D1SolidColorBrush::SetColor (vtable index 8)
    let set_color: extern "system" fn(*mut c_void, *const D2D1ColorF) =
        std::mem::transmute(*brush.vtable().add(8));
    set_color(brush.as_ptr(), &color);
}

unsafe fn rt_set_transform(rt: &ComPtr, m: &D2D1Matrix3x2F) {
    // ID2D1RenderTarget::SetTransform (vtable index 30)
    let set_xform: extern "system" fn(*mut c_void, *const D2D1Matrix3x2F) =
        std::mem::transmute(*rt.vtable().add(30));
    set_xform(rt.as_ptr(), m);
}

unsafe fn rt_set_antialias_mode(rt: &ComPtr, mode: u32) {
    // ID2D1RenderTarget::SetAntialiasMode (vtable index 32)
    let set_aa: extern "system" fn(*mut c_void, u32) =
        std::mem::transmute(*rt.vtable().add(32));
    set_aa(rt.as_ptr(), mode);
}

unsafe fn rt_set_text_antialias_mode(rt: &ComPtr, mode: u32) {
    // ID2D1RenderTarget::SetTextAntialiasMode (vtable index 34)
    let set_taa: extern "system" fn(*mut c_void, u32) =
        std::mem::transmute(*rt.vtable().add(34));
    set_taa(rt.as_ptr(), mode);
}

unsafe fn rt_begin_draw(rt: &ComPtr) {
    // ID2D1RenderTarget::BeginDraw (vtable index 48)
    let begin: extern "system" fn(*mut c_void) = std::mem::transmute(*rt.vtable().add(48));
    begin(rt.as_ptr());
}

unsafe fn rt_end_draw(rt: &ComPtr) -> HRESULT {
    // ID2D1RenderTarget::EndDraw (vtable index 49)
    let end: extern "system" fn(*mut c_void, *mut u64, *mut u64) -> HRESULT =
        std::mem::transmute(*rt.vtable().add(49));
    end(rt.as_ptr(), null_mut(), null_mut())
}

unsafe fn rt_fill_rounded_rect(rt: &ComPtr, rr: &D2D1RoundedRect, brush: &ComPtr) {
    // ID2D1RenderTarget::FillRoundedRectangle (vtable index 19)
    let fill: extern "system" fn(*mut c_void, *const D2D1RoundedRect, *mut c_void) =
        std::mem::transmute(*rt.vtable().add(19));
    fill(rt.as_ptr(), rr, brush.as_ptr());
}

unsafe fn rt_draw_rounded_rect(
    rt: &ComPtr,
    rr: &D2D1RoundedRect,
    brush: &ComPtr,
    stroke_width: f32,
) {
    // ID2D1RenderTarget::DrawRoundedRectangle (vtable index 18)
    let stroke: extern "system" fn(
        *mut c_void,
        *const D2D1RoundedRect,
        *mut c_void,
        f32,
        *mut c_void,
    ) = std::mem::transmute(*rt.vtable().add(18));
    stroke(rt.as_ptr(), rr, brush.as_ptr(), stroke_width, null_mut());
}

unsafe fn rt_draw_text_layout(
    rt: &ComPtr,
    origin_x: f32,
    origin_y: f32,
    layout: &ComPtr,
    brush: &ComPtr,
    options: u32,
) {
    // ID2D1RenderTarget::DrawTextLayout (vtable index 28)
    let draw: extern "system" fn(*mut c_void, u64, *mut c_void, *mut c_void, u32) =
        std::mem::transmute(*rt.vtable().add(28));
    draw(
        rt.as_ptr(),
        pack_point_2f(origin_x, origin_y),
        layout.as_ptr(),
        brush.as_ptr(),
        options,
    );
}

unsafe fn dc_rt_bind(rt: &ComPtr, dc: HDC, width: i32, height: i32) -> HRESULT {
    let sub_rect = RECT {
        left: 0,
        top: 0,
        right: width,
        bottom: height,
    };
    // ID2D1DCRenderTarget::BindDC (vtable index 57)
    let bind: extern "system" fn(*mut c_void, HDC, *const RECT) -> HRESULT =
        std::mem::transmute(*rt.vtable().add(57));
    bind(rt.as_ptr(), dc, &sub_rect)
}

unsafe fn with_dc_target(
    dc: HDC,
    dc_width: i32,
    dc_height: i32,
    draw_fn: impl FnOnce(&mut RuntimeState, &ComPtr, &ComPtr),
) -> bool {
    if dc.is_null() || dc_width <= 0 || dc_height <= 0 {
        return false;
    }
    GdiFlush();
    with_runtime(|state| {
        let (rt, brush) = {
            let (rt, brush) = state.ensure_dc_target()?;
            (rt.clone(), brush.clone())
        };
        if dc_rt_bind(&rt, dc, dc_width, dc_height) < 0 {
            state.dc_brush = None;
            state.dc_rt = None;
            return None;
        }
        rt_begin_draw(&rt);
        rt_set_transform(&rt, &D2D1Matrix3x2F::identity());
        rt_set_antialias_mode(&rt, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
        rt_set_text_antialias_mode(&rt, D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
        draw_fn(state, &rt, &brush);
        let hr = rt_end_draw(&rt);
        if hr == D2DERR_RECREATE_TARGET {
            state.dc_brush = None;
            state.dc_rt = None;
        }
        (hr >= 0).then_some(())
    })
    .is_some()
}

/// Returns `true` if Direct2D 1.1 and DirectWrite initialize on the current thread.
pub fn available() -> bool {
    with_runtime(|_| Some(())).is_some()
}

/// Releases all thread-local Direct2D/DirectWrite factories, caches, and render targets.
pub fn shutdown() {
    RUNTIME.with(|slot| {
        if let Some(mut state) = slot.borrow_mut().take() {
            state.clear();
        }
    });
}

/// Measures `text` using DirectWrite layout metrics without allocating a render target.
pub fn measure_text(
    text: &str,
    style: &TextStyle<'_>,
    max_width: f32,
    max_height: f32,
) -> Option<TextMetrics> {
    with_runtime(|state| unsafe {
        let layout = state.create_text_layout(text, style, max_width, max_height)?;
        // IDWriteTextLayout::GetMetrics (vtable index 60)
        let get_metrics: extern "system" fn(*mut c_void, *mut DWriteTextMetricsRaw) -> HRESULT =
            std::mem::transmute(*layout.vtable().add(60));
        let mut raw = DWriteTextMetricsRaw::default();
        if get_metrics(layout.as_ptr(), &mut raw) < 0 {
            return None;
        }
        Some(TextMetrics {
            left: raw.left,
            top: raw.top,
            width: raw.width,
            width_including_trailing_whitespace: raw.width_including_trailing_whitespace,
            height: raw.height,
            line_count: raw.line_count,
        })
    })
}

/// Draws DirectWrite text onto a premultiplied BGRA32 memory `HDC` using grayscale AA.
///
/// `color` uses Kinetix's `0x00BBGGRR` encoding combined with `alpha`.
///
/// # Safety
/// `dc` must be a valid `HDC` backed by a surface of at least `dc_width` x `dc_height` pixels.
pub unsafe fn draw_text_dc(
    dc: HDC,
    dc_width: i32,
    dc_height: i32,
    text: &str,
    rect: [f32; 4],
    style: &TextStyle<'_>,
    color: u32,
    alpha: u8,
) -> bool {
    let w = (rect[2] - rect[0]).max(0.0);
    let h = (rect[3] - rect[1]).max(0.0);
    if w <= 0.0 || h <= 0.0 {
        return false;
    }
    let d2d_color = D2D1ColorF::from_colorref(color, alpha);
    with_dc_target(dc, dc_width, dc_height, |state, rt, brush| {
        if let Some(layout) = state.create_text_layout(text, style, w, h) {
            brush_set_color(brush, d2d_color);
            rt_draw_text_layout(
                rt,
                rect[0],
                rect[1],
                &layout,
                brush,
                D2D1_DRAW_TEXT_OPTIONS_CLIP,
            );
        }
    })
}

/// A single styled text run for batched DirectWrite rendering via [`draw_rich_runs_dc`].
#[derive(Clone, Copy, Debug)]
pub struct RichRun<'a> {
    pub text: &'a str,
    pub rect: [f32; 4],
    pub style: TextStyle<'a>,
    pub underline: bool,
    pub strike: bool,
    pub color: u32,
    pub alpha: u8,
}

/// Draws a batch of styled DirectWrite text runs onto a premultiplied BGRA32 memory `HDC`
/// within a single `BeginDraw`/`EndDraw` pass using grayscale antialiasing.
///
/// `color` uses Kinetix's `0x00BBGGRR` encoding combined with `alpha`.
///
/// # Safety
/// `dc` must be a valid `HDC` backed by a surface of at least `dc_width` x `dc_height` pixels.
pub unsafe fn draw_rich_runs_dc(
    dc: HDC,
    dc_width: i32,
    dc_height: i32,
    runs: &[RichRun<'_>],
) -> bool {
    if runs.is_empty() {
        return true;
    }
    with_dc_target(dc, dc_width, dc_height, |state, rt, brush| {
        for run in runs {
            if run.text.is_empty() {
                continue;
            }
            let w = run.rect[2] - run.rect[0];
            let h = run.rect[3] - run.rect[1];
            if !(w > 0.0 && h > 0.0) {
                continue;
            }
            if let Some(layout) = state.create_text_layout_styled(
                run.text,
                &run.style,
                w,
                h,
                run.underline,
                run.strike,
            ) {
                brush_set_color(brush, D2D1ColorF::from_colorref(run.color, run.alpha));
                rt_draw_text_layout(
                    rt,
                    run.rect[0],
                    run.rect[1],
                    &layout,
                    brush,
                    D2D1_DRAW_TEXT_OPTIONS_CLIP,
                );
            }
        }
    })
}

/// Fills an antialiased rounded rectangle onto a premultiplied BGRA32 memory `HDC`.
///
/// `color` uses Kinetix's `0x00BBGGRR` encoding combined with `alpha`.
///
/// # Safety
/// `dc` must be a valid `HDC` backed by a surface of at least `dc_width` x `dc_height` pixels.
pub unsafe fn fill_round_rect_dc(
    dc: HDC,
    dc_width: i32,
    dc_height: i32,
    rect: [f32; 4],
    radius: f32,
    color: u32,
    alpha: u8,
) -> bool {
    let rr = D2D1RoundedRect {
        rect: to_d2d_rect(rect),
        radius_x: radius.max(0.0),
        radius_y: radius.max(0.0),
    };
    let d2d_color = D2D1ColorF::from_colorref(color, alpha);
    with_dc_target(dc, dc_width, dc_height, |_state, rt, brush| {
        brush_set_color(brush, d2d_color);
        rt_fill_rounded_rect(rt, &rr, brush);
    })
}

/// Strokes an antialiased rounded rectangle onto a premultiplied BGRA32 memory `HDC`.
///
/// `color` uses Kinetix's `0x00BBGGRR` encoding combined with `alpha`.
///
/// # Safety
/// `dc` must be a valid `HDC` backed by a surface of at least `dc_width` x `dc_height` pixels.
pub unsafe fn stroke_round_rect_dc(
    dc: HDC,
    dc_width: i32,
    dc_height: i32,
    rect: [f32; 4],
    radius: f32,
    stroke_width: f32,
    color: u32,
    alpha: u8,
) -> bool {
    if stroke_width <= 0.0 {
        return false;
    }
    let rr = D2D1RoundedRect {
        rect: to_d2d_rect(rect),
        radius_x: radius.max(0.0),
        radius_y: radius.max(0.0),
    };
    let d2d_color = D2D1ColorF::from_colorref(color, alpha);
    with_dc_target(dc, dc_width, dc_height, |_state, rt, brush| {
        brush_set_color(brush, d2d_color);
        rt_draw_rounded_rect(rt, &rr, brush, stroke_width);
    })
}

/// Owned `ID2D1Device` + `ID2D1DeviceContext` pair bound to a DXGI device.
pub struct D2dDeviceContext {
    device: ComPtr,
    ctx: ComPtr,
}

impl D2dDeviceContext {
    #[inline]
    pub fn as_ptr(&self) -> *mut c_void {
        self.ctx.as_ptr()
    }

    #[inline]
    pub fn device_ptr(&self) -> *mut c_void {
        self.device.as_ptr()
    }

    /// Creates a target `ID2D1Bitmap1` backed by `dxgi_surface` (`IDXGISurface`).
    ///
    /// # Safety
    /// `dxgi_surface` must be a valid `IDXGISurface` pointer compatible with the DXGI device.
    pub unsafe fn create_bitmap_from_dxgi_surface(
        &self,
        dxgi_surface: *mut c_void,
        dpi_x: f32,
        dpi_y: f32,
    ) -> Option<ComPtr> {
        if dxgi_surface.is_null() {
            return None;
        }
        let props = D2D1BitmapProperties1 {
            pixel_format: D2D1PixelFormat {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alpha_mode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpi_x,
            dpi_y,
            bitmap_options: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
            color_context: null_mut(),
        };
        // ID2D1DeviceContext::CreateBitmapFromDxgiSurface (vtable index 62)
        let create_bmp: extern "system" fn(
            *mut c_void,
            *mut c_void,
            *const D2D1BitmapProperties1,
            *mut *mut c_void,
        ) -> HRESULT = std::mem::transmute(*self.ctx.vtable().add(62));
        let mut raw_bmp = null_mut();
        if create_bmp(self.ctx.as_ptr(), dxgi_surface, &props, &mut raw_bmp) < 0 {
            return None;
        }
        ComPtr::from_raw(raw_bmp)
    }

    /// Sets the active render target image (`ID2D1Image` / `ID2D1Bitmap1`) on this device context.
    ///
    /// # Safety
    /// `image` must be null or a valid `ID2D1Image` pointer created from this device context.
    pub unsafe fn set_target(&self, image: *mut c_void) {
        // ID2D1DeviceContext::SetTarget (vtable index 74)
        let set_target: extern "system" fn(*mut c_void, *mut c_void) =
            std::mem::transmute(*self.ctx.vtable().add(74));
        set_target(self.ctx.as_ptr(), image);
    }

    /// Begins a Direct2D drawing batch on the current target.
    ///
    /// # Safety
    /// A valid target must be bound before drawing commands are issued.
    pub unsafe fn begin_draw(&self) {
        rt_begin_draw(&self.ctx);
        rt_set_transform(&self.ctx, &D2D1Matrix3x2F::identity());
        rt_set_antialias_mode(&self.ctx, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
        rt_set_text_antialias_mode(&self.ctx, D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
    }

    /// Ends the Direct2D drawing batch and returns the `HRESULT`.
    ///
    /// # Safety
    /// Must pair with a prior [`Self::begin_draw`] call.
    pub unsafe fn end_draw(&self) -> HRESULT {
        rt_end_draw(&self.ctx)
    }

    /// Clears the current target to `color`.
    ///
    /// # Safety
    /// Must be called between [`Self::begin_draw`] and [`Self::end_draw`].
    pub unsafe fn clear(&self, color: D2D1ColorF) {
        // ID2D1RenderTarget::Clear (vtable index 47)
        let clear: extern "system" fn(*mut c_void, *const D2D1ColorF) =
            std::mem::transmute(*self.ctx.vtable().add(47));
        clear(self.ctx.as_ptr(), &color);
    }

    /// Updates the device context DPI.
    ///
    /// # Safety
    /// `self.ctx` must be valid.
    pub unsafe fn set_dpi(&self, dpi_x: f32, dpi_y: f32) {
        // ID2D1RenderTarget::SetDpi (vtable index 51)
        let set_dpi: extern "system" fn(*mut c_void, f32, f32) =
            std::mem::transmute(*self.ctx.vtable().add(51));
        set_dpi(self.ctx.as_ptr(), dpi_x, dpi_y);
    }

    /// Creates a solid-color brush bound to this device context.
    ///
    /// # Safety
    /// `self.ctx` must be valid.
    pub unsafe fn create_solid_color_brush(&self, color: u32, alpha: u8) -> Option<ComPtr> {
        rt_create_solid_brush(&self.ctx, D2D1ColorF::from_colorref(color, alpha))
    }

    /// Fills an antialiased rounded rectangle using a brush created by this context.
    ///
    /// # Safety
    /// Must be called between `begin_draw` and `end_draw` with a valid brush from this context.
    pub unsafe fn fill_round_rect(&self, rect: [f32; 4], radius: f32, brush: &ComPtr) {
        let rr = D2D1RoundedRect {
            rect: to_d2d_rect(rect),
            radius_x: radius.max(0.0),
            radius_y: radius.max(0.0),
        };
        rt_fill_rounded_rect(&self.ctx, &rr, brush);
    }

    /// Strokes an antialiased rounded rectangle using a brush created by this context.
    ///
    /// # Safety
    /// Must be called between `begin_draw` and `end_draw` with a valid brush from this context.
    pub unsafe fn stroke_round_rect(
        &self,
        rect: [f32; 4],
        radius: f32,
        stroke_width: f32,
        brush: &ComPtr,
    ) {
        let rr = D2D1RoundedRect {
            rect: to_d2d_rect(rect),
            radius_x: radius.max(0.0),
            radius_y: radius.max(0.0),
        };
        rt_draw_rounded_rect(&self.ctx, &rr, brush, stroke_width);
    }

    /// Draws DirectWrite text onto this device context using a brush created by this context.
    ///
    /// # Safety
    /// Must be called between `begin_draw` and `end_draw` with a valid brush from this context.
    pub unsafe fn draw_text(
        &self,
        text: &str,
        rect: [f32; 4],
        style: &TextStyle<'_>,
        brush: &ComPtr,
    ) -> bool {
        let w = (rect[2] - rect[0]).max(0.0);
        let h = (rect[3] - rect[1]).max(0.0);
        if w <= 0.0 || h <= 0.0 {
            return false;
        }
        with_runtime(|state| {
            let layout = state.create_text_layout(text, style, w, h)?;
            rt_draw_text_layout(
                &self.ctx,
                rect[0],
                rect[1],
                &layout,
                brush,
                D2D1_DRAW_TEXT_OPTIONS_CLIP,
            );
            Some(())
        })
        .is_some()
    }
}

/// Creates an `ID2D1Device` + `ID2D1DeviceContext` via `ID2D1Factory1::CreateDevice`.
///
/// # Safety
/// `dxgi_device` must be a valid `IDXGIDevice` pointer.
pub unsafe fn create_d2d_device_context(dxgi_device: *mut c_void) -> Option<D2dDeviceContext> {
    if dxgi_device.is_null() {
        return None;
    }
    with_runtime(|state| {
        if !state.is_factory1 {
            return None;
        }
        // ID2D1Factory1::CreateDevice (vtable index 17)
        let create_device: extern "system" fn(
            *mut c_void,
            *mut c_void,
            *mut *mut c_void,
        ) -> HRESULT = std::mem::transmute(*state.d2d_factory.vtable().add(17));
        let mut raw_device = null_mut();
        if create_device(state.d2d_factory.as_ptr(), dxgi_device, &mut raw_device) < 0 {
            return None;
        }
        let device = ComPtr::from_raw(raw_device)?;

        // ID2D1Device::CreateDeviceContext (vtable index 4)
        let create_ctx: extern "system" fn(
            *mut c_void,
            u32,
            *mut *mut c_void,
        ) -> HRESULT = std::mem::transmute(*device.vtable().add(4));
        let mut raw_ctx = null_mut();
        if create_ctx(device.as_ptr(), D2D1_DEVICE_CONTEXT_OPTIONS_NONE, &mut raw_ctx) < 0 {
            return None;
        }
        let ctx = ComPtr::from_raw(raw_ctx)?;
        Some(D2dDeviceContext { device, ctx })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dwrite_initializes_and_measures_text_monotonically() {
        assert!(available());

        let style = TextStyle::default();
        let m1 = measure_text("Hi", &style, 1000.0, 200.0).expect("measure short text");
        let m2 = measure_text("Hello, Kinetix!", &style, 1000.0, 200.0).expect("measure medium text");
        let m3 = measure_text("Hello, Kinetix! Native 120Hz", &style, 1000.0, 200.0)
            .expect("measure long text");

        assert!(m1.width > 0.0 && m1.height > 0.0);
        assert_eq!(m1.line_count, 1);
        assert!(m1.width < m2.width);
        assert!(m2.width < m3.width);

        let big_style = TextStyle {
            size: 24.0,
            weight: 700,
            ..style
        };
        let m_big = measure_text("Hi", &big_style, 1000.0, 200.0).expect("measure larger text");
        assert!(m_big.height > m1.height);
        assert!(m_big.width > m1.width);

        shutdown();
        assert!(available());
        shutdown();
    }

    #[test]
    fn d2d_dc_target_draws_antialiased_text_and_round_rect() {
        assert!(available());

        let buf = unsafe { crate::compositor::Buffer::plain(240, 80, 1.0) }.expect("plain buffer");
        assert!(unsafe {
            fill_round_rect_dc(
                buf.dc,
                buf.width,
                buf.height,
                [4.0, 4.0, 236.0, 76.0],
                12.0,
                0x201814,
                220,
            )
        });
        assert!(unsafe {
            stroke_round_rect_dc(
                buf.dc,
                buf.width,
                buf.height,
                [4.0, 4.0, 236.0, 76.0],
                12.0,
                1.5,
                0xffffff,
                200,
            )
        });
        let text_style = TextStyle {
            size: 18.0,
            weight: 600,
            align: TextAlign::Center,
            valign: ParagraphAlign::Center,
            trim_ellipsis: true,
            ..TextStyle::default()
        };
        assert!(unsafe {
            draw_text_dc(
                buf.dc,
                buf.width,
                buf.height,
                "Kinetix 120Hz",
                [16.0, 12.0, 224.0, 68.0],
                &text_style,
                0xffffff,
                255,
            )
        });

        let pixels = unsafe { buf.snapshot() };
        assert_eq!(pixels.len(), (buf.width * buf.height * 4) as usize);

        // Outside rounded rect corner (0, 0) remains untouched transparent.
        assert_eq!(&pixels[0..4], &[0, 0, 0, 0]);

        // Inside rounded rect has premultiplied nonzero alpha.
        let inner_idx = ((12 * buf.width + 20) * 4) as usize;
        assert!(pixels[inner_idx + 3] > 0);

        // Antialiased rounded stroke + grayscale-AA white text produce intermediate
        // luminance/alpha pixels strictly between transparent/fill and full white.
        let has_aa_pixels = pixels.chunks_exact(4).any(|px| {
            let (b, g, r, a) = (px[0], px[1], px[2], px[3]);
            a > 0 && a < 255 && r > 0x20 && r < 245 && g > 0x18 && b > 0x14
        });
        let has_bright_text_or_stroke = pixels.chunks_exact(4).any(|px| {
            px[0] > 140 && px[1] > 140 && px[2] > 140 && px[3] > 180
        });
        assert!(has_bright_text_or_stroke);
        assert!(has_aa_pixels);

        shutdown();
    }

    #[test]
    fn d2d_draw_rich_runs_renders_variable_font_styles_and_vietnamese() {
        assert!(available());

        let buf = unsafe { crate::compositor::Buffer::plain(480, 120, 1.0) }.expect("plain buffer");
        assert!(unsafe { draw_rich_runs_dc(buf.dc, buf.width, buf.height, &[]) });

        let base_style = TextStyle {
            font_family: "Segoe UI Variable Text",
            size: 16.0,
            weight: 400,
            italic: false,
            align: TextAlign::Leading,
            valign: ParagraphAlign::Near,
            wrap: WordWrap::NoWrap,
            trim_ellipsis: false,
        };
        let bold_style = TextStyle {
            weight: 700,
            ..base_style
        };
        let italic_style = TextStyle {
            italic: true,
            ..base_style
        };
        let runs = [
            RichRun {
                text: "Regular",
                rect: [8.0, 8.0, 110.0, 40.0],
                style: base_style,
                underline: false,
                strike: false,
                color: 0xffffff,
                alpha: 255,
            },
            RichRun {
                text: "Bold",
                rect: [115.0, 8.0, 180.0, 40.0],
                style: bold_style,
                underline: false,
                strike: false,
                color: 0xffffff,
                alpha: 255,
            },
            RichRun {
                text: "Italic",
                rect: [185.0, 8.0, 260.0, 40.0],
                style: italic_style,
                underline: false,
                strike: false,
                color: 0xffffff,
                alpha: 255,
            },
            RichRun {
                text: "Underline",
                rect: [8.0, 44.0, 130.0, 76.0],
                style: base_style,
                underline: true,
                strike: false,
                color: 0xffffff,
                alpha: 255,
            },
            RichRun {
                text: "Strike",
                rect: [140.0, 44.0, 240.0, 76.0],
                style: base_style,
                underline: false,
                strike: true,
                color: 0xffffff,
                alpha: 255,
            },
            RichRun {
                text: "đê ăơư áệợự",
                rect: [8.0, 80.0, 260.0, 114.0],
                style: base_style,
                underline: false,
                strike: false,
                color: 0xffffff,
                alpha: 255,
            },
        ];

        assert!(unsafe { draw_rich_runs_dc(buf.dc, buf.width, buf.height, &runs) });

        let pixels = unsafe { buf.snapshot() };
        assert_eq!(pixels.len(), (buf.width * buf.height * 4) as usize);

        let has_nonzero_premul = pixels.chunks_exact(4).any(|px| {
            let (b, g, r, a) = (px[0], px[1], px[2], px[3]);
            a > 128 && b <= a && g <= a && r <= a
        });
        let has_grayscale_aa = pixels.chunks_exact(4).any(|px| {
            let (b, g, r, a) = (px[0], px[1], px[2], px[3]);
            a > 0 && a < 255 && b == g && g == r && r <= a
        });
        assert!(has_nonzero_premul);
        assert!(has_grayscale_aa);

        shutdown();
    }
}
