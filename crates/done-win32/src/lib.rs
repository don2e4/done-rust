//! Win32 implementation of done's native-widget contract: standard USER32 and
//! common controls as child windows, a per-monitor-v2 DPI-aware message loop,
//! and WIC image decoding.
//!
//! Compiled and linted for Windows targets; see the milestone 3 report for
//! what has not yet run on Windows.
#![cfg(windows)]
#![deny(clippy::undocumented_unsafe_blocks)]

mod wic;

use done_core::*;
use std::{
    borrow::Cow,
    cell::{Cell, RefCell},
    collections::HashMap,
    ffi::c_void,
    mem,
    path::Path,
    ptr,
    rc::Rc,
};
use windows_sys::core::{w, PCWSTR};
use windows_sys::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    CreateDIBSection, CreateFontIndirectW, DeleteObject, GetDC, GetStockObject, GetSysColorBrush,
    GetTextExtentPoint32W, GetTextMetricsW, MapWindowPoints, ReleaseDC, SelectObject, SetBkMode,
    BITMAPINFO, BITMAPINFOHEADER, BI_RGB, COLOR_BTNFACE, DEFAULT_GUI_FONT, DIB_RGB_COLORS, HBITMAP,
    HDC, HFONT, LOGFONTW, TEXTMETRICW, TRANSPARENT,
};
use windows_sys::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::SystemServices::{
    SS_BITMAP, SS_LEFTNOWORDWRAP, SS_NOPREFIX, SS_REALSIZECONTROL,
};
use windows_sys::Win32::UI::Controls::{
    InitCommonControlsEx, SetScrollInfo, BCM_GETIDEALSIZE, BST_CHECKED, BST_UNCHECKED,
    EM_SETCUEBANNER, ICC_BAR_CLASSES, ICC_PROGRESS_CLASS, ICC_STANDARD_CLASSES,
    INITCOMMONCONTROLSEX, PBM_SETPOS, PBM_SETRANGE32, PROGRESS_CLASSW, TBM_SETLINESIZE,
    TBM_SETPAGESIZE, TBM_SETPOS, TBM_SETRANGEMAX, TBM_SETRANGEMIN, TBS_NOTICKS, TRACKBAR_CLASSW,
    WC_BUTTONW, WC_COMBOBOXW, WC_EDITW, WC_STATICW,
};
use windows_sys::Win32::UI::HiDpi::{
    AdjustWindowRectExForDpi, GetDpiForSystem, GetDpiForWindow, GetSystemMetricsForDpi,
    SetProcessDpiAwarenessContext, SystemParametersInfoForDpi,
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetFocus, VK_RETURN};
use windows_sys::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect, GetMessageW,
    GetParent, GetScrollInfo, GetWindow, GetWindowLongPtrW, GetWindowRect, GetWindowTextLengthW,
    GetWindowTextW, IsDialogMessageW, LoadCursorW, PostMessageW, PostQuitMessage, RegisterClassExW,
    SendMessageW, SetWindowLongPtrW, SetWindowPos, SetWindowTextW, ShowWindow, TranslateMessage,
    BM_GETCHECK, BM_SETCHECK, BN_CLICKED, BS_AUTOCHECKBOX, BS_PUSHBUTTON, BS_TYPEMASK,
    CBN_SELCHANGE, CBS_DROPDOWNLIST, CB_ADDSTRING, CB_ERR, CB_GETCURSEL, CB_RESETCONTENT,
    CB_SETCURSEL, CW_USEDEFAULT, DLGC_WANTALLKEYS, EN_CHANGE, ES_AUTOHSCROLL, GWLP_USERDATA,
    GWL_STYLE, GW_CHILD, GW_HWNDNEXT, HWND_BOTTOM, HWND_TOP, IDC_ARROW, IMAGE_BITMAP, MINMAXINFO,
    MSG, NONCLIENTMETRICSW, SB_BOTTOM, SB_LINEDOWN, SB_LINEUP, SB_PAGEDOWN, SB_PAGEUP,
    SB_THUMBTRACK, SB_TOP, SB_VERT, SCROLLINFO, SIF_ALL, SIF_DISABLENOSCROLL, SIF_PAGE, SIF_POS,
    SIF_RANGE, SIZE_MINIMIZED, SM_CXMENUCHECK, SM_CXVSCROLL, SPI_GETNONCLIENTMETRICS, STM_SETIMAGE,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SW_SHOW, WHEEL_DELTA, WM_APP, WM_CHAR,
    WM_CLOSE, WM_COMMAND, WM_CTLCOLORBTN, WM_CTLCOLORSTATIC, WM_DESTROY, WM_DPICHANGED,
    WM_GETDLGCODE, WM_GETMINMAXINFO, WM_HSCROLL, WM_KEYDOWN, WM_MOUSEWHEEL, WM_NCCREATE,
    WM_SETFONT, WM_SETTINGCHANGE, WM_SIZE, WM_THEMECHANGED, WM_USER, WM_VSCROLL, WNDCLASSEXW,
    WS_CHILD, WS_CLIPCHILDREN, WS_CLIPSIBLINGS, WS_EX_CLIENTEDGE, WS_EX_CONTROLPARENT,
    WS_OVERLAPPEDWINDOW, WS_TABSTOP, WS_VISIBLE, WS_VSCROLL,
};

/// Posted to the top-level window to run one batch.
const WM_FLUSH: u32 = WM_APP + 1;
/// commctrl.h: `TBM_GETPOS` is `WM_USER`.
const TBM_GETPOS: u32 = WM_USER;
const WINDOW_CLASS: PCWSTR = w!("DoneWindow");
const HOST_CLASS: PCWSTR = w!("DoneHost");
const SCROLL_CLASS: PCWSTR = w!("DoneScroll");
/// A drop-down's list opens below its selection field; logical height.
const LIST_HEIGHT: f32 = 200.0;
/// Lines scrolled per wheel notch, and a line's logical height.
const WHEEL_LINES: i32 = 3;
const LINE: f32 = 20.0;

type Dispatch = Box<dyn FnMut(Vec<Event>)>;

/// State window procedures need; everything here is UI-thread-only.
#[derive(Default)]
struct Shared {
    pending: RefCell<Vec<Event>>,
    scheduled: Cell<bool>,
    /// Set while the backend changes a control, so the change is not an edit.
    quiet: Cell<bool>,
    window: Cell<usize>,
    /// The layout's minimum client size, logical units.
    minimum: Cell<(f32, f32)>,
    sliders: RefCell<HashMap<usize, SliderRange>>,
    /// Scroll viewport content extents, physical pixels.
    extents: RefCell<HashMap<usize, (i32, i32)>>,
    dispatch: RefCell<Option<Dispatch>>,
}

thread_local! {
    static SHARED: Shared = Shared::default();
}

fn push(event: Event) {
    SHARED.with(|shared| {
        shared.pending.borrow_mut().push(event);
        if !shared.scheduled.replace(true) {
            // SAFETY: posting to our own top-level window; before it exists
            // the message goes to this thread's queue, which the loop drains.
            unsafe { PostMessageW(shared.window.get() as HWND, WM_FLUSH, 0, 0) };
        }
    });
}

fn flush() {
    let events = SHARED.with(|shared| {
        shared.scheduled.set(false);
        mem::take(&mut *shared.pending.borrow_mut())
    });
    if events.is_empty() {
        return;
    }
    // Taken out while it runs: native calls inside may process messages.
    let dispatch = SHARED.with(|shared| shared.dispatch.borrow_mut().take());
    match dispatch {
        Some(mut dispatch) => {
            dispatch(events);
            SHARED.with(|shared| {
                let mut slot = shared.dispatch.borrow_mut();
                if slot.is_none() {
                    *slot = Some(dispatch);
                }
            });
        }
        // Arrived while a batch was running: keep them, in order, for the next.
        None => SHARED.with(|shared| {
            let mut pending = shared.pending.borrow_mut();
            let later = mem::replace(&mut *pending, events);
            pending.extend(later);
        }),
    }
    SHARED.with(|shared| {
        if !shared.pending.borrow().is_empty() && !shared.scheduled.replace(true) {
            // SAFETY: as in `push`.
            unsafe { PostMessageW(shared.window.get() as HWND, WM_FLUSH, 0, 0) };
        }
    });
}

fn quietly<T>(change: impl FnOnce() -> T) -> T {
    SHARED.with(|shared| shared.quiet.set(true));
    let result = change();
    SHARED.with(|shared| shared.quiet.set(false));
    result
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

fn low(value: usize) -> u32 {
    (value & 0xffff) as u32
}

fn high(value: usize) -> u32 {
    ((value >> 16) & 0xffff) as u32
}

fn last_error(context: &str) -> Error {
    format!("{context}: {}", std::io::Error::last_os_error()).into()
}

fn window_text(hwnd: HWND) -> String {
    // SAFETY: `hwnd` is a live control; the buffer has room for the text and
    // its terminator.
    unsafe {
        let length = GetWindowTextLengthW(hwnd).max(0) as usize;
        let mut buffer = vec![0u16; length + 1];
        let copied = GetWindowTextW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32).max(0);
        String::from_utf16_lossy(&buffer[..copied as usize])
    }
}

fn widget_id(hwnd: HWND) -> WidgetId {
    // SAFETY: every control this backend creates stores its ID here.
    WidgetId(unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as u64)
}

fn dpi_scale(hwnd: HWND) -> f32 {
    // SAFETY: querying a live window's DPI.
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    if dpi == 0 {
        1.0
    } else {
        dpi as f32 / 96.0
    }
}

/// Notifications that controls send to their parent host.
fn notification(message: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
    match message {
        WM_COMMAND if lparam != 0 => {
            if SHARED.with(|shared| shared.quiet.get()) {
                return Some(0);
            }
            let control = lparam as HWND;
            let id = widget_id(control);
            match high(wparam) {
                BN_CLICKED => {
                    // SAFETY: `control` is the live button that sent the message.
                    let style = unsafe { GetWindowLongPtrW(control, GWL_STYLE) } as u32;
                    if style & BS_TYPEMASK as u32 == BS_AUTOCHECKBOX as u32 {
                        // SAFETY: as above.
                        let state = unsafe { SendMessageW(control, BM_GETCHECK, 0, 0) };
                        push(Event::Toggle(id, state as u32 == BST_CHECKED));
                    } else {
                        push(Event::Click(id));
                    }
                }
                EN_CHANGE => push(Event::Input(id, window_text(control))),
                CBN_SELCHANGE => {
                    // SAFETY: `control` is the live combo box that sent the message.
                    let index = unsafe { SendMessageW(control, CB_GETCURSEL, 0, 0) };
                    if index >= 0 {
                        push(Event::Select(id, index as usize));
                    }
                }
                _ => return None,
            }
            Some(0)
        }
        WM_HSCROLL if lparam != 0 => {
            let control = lparam as HWND;
            let range =
                SHARED.with(|shared| shared.sliders.borrow().get(&(control as usize)).copied())?;
            // SAFETY: `control` is the live trackbar that sent the message.
            let position = unsafe { SendMessageW(control, TBM_GETPOS, 0, 0) } as f64;
            let value = (range.min + position * range.step).min(range.max);
            push(Event::Slide(widget_id(control), value));
            Some(0)
        }
        WM_CTLCOLORSTATIC | WM_CTLCOLORBTN => {
            // Labels and check boxes draw over the host's own background.
            // SAFETY: `wparam` is the control's device context for this paint.
            unsafe { SetBkMode(wparam as HDC, TRANSPARENT as i32) };
            // SAFETY: system color brushes are owned by the system.
            Some(unsafe { GetSysColorBrush(COLOR_BTNFACE) } as LRESULT)
        }
        _ => None,
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_NCCREATE => SHARED.with(|shared| shared.window.set(hwnd as usize)),
        WM_FLUSH => {
            flush();
            return 0;
        }
        WM_CLOSE => {
            push(Event::Close);
            return 0;
        }
        WM_DESTROY => {
            // SAFETY: ends this thread's message loop.
            unsafe { PostQuitMessage(0) };
            return 0;
        }
        WM_SIZE if wparam != SIZE_MINIMIZED as usize => {
            let scale = dpi_scale(hwnd);
            push(Event::Resized(Size {
                width: low(lparam as usize) as f32 / scale,
                height: high(lparam as usize) as f32 / scale,
            }));
            return 0;
        }
        WM_GETMINMAXINFO => {
            let (width, height) = SHARED.with(|shared| shared.minimum.get());
            if width > 0.0 || height > 0.0 {
                // SAFETY: querying a live window.
                let dpi = unsafe { GetDpiForWindow(hwnd) };
                let scale = dpi as f32 / 96.0;
                let mut rect = RECT {
                    left: 0,
                    top: 0,
                    right: (width * scale).ceil() as i32,
                    bottom: (height * scale).ceil() as i32,
                };
                // SAFETY: valid rectangle; styles are the window's own.
                unsafe {
                    AdjustWindowRectExForDpi(
                        &mut rect,
                        GetWindowLongPtrW(hwnd, GWL_STYLE) as u32,
                        0,
                        WS_EX_CONTROLPARENT,
                        dpi,
                    )
                };
                // SAFETY: for this message `lparam` points to a MINMAXINFO.
                let info = unsafe { &mut *(lparam as *mut MINMAXINFO) };
                info.ptMinTrackSize = POINT {
                    x: rect.right - rect.left,
                    y: rect.bottom - rect.top,
                };
                return 0;
            }
        }
        WM_DPICHANGED => {
            // SAFETY: for this message `lparam` points to the suggested rectangle.
            let suggested = unsafe { &*(lparam as *const RECT) };
            // SAFETY: resizing our own window to the system's suggestion.
            unsafe {
                SetWindowPos(
                    hwnd,
                    ptr::null_mut(),
                    suggested.left,
                    suggested.top,
                    suggested.right - suggested.left,
                    suggested.bottom - suggested.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                )
            };
            push(Event::AppearanceChanged);
            return 0;
        }
        WM_SETTINGCHANGE | WM_THEMECHANGED => push(Event::AppearanceChanged),
        _ => {
            if let Some(result) = notification(message, wparam, lparam) {
                return result;
            }
        }
    }
    // SAFETY: default handling for our own window class.
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}

unsafe extern "system" fn host_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match notification(message, wparam, lparam) {
        Some(result) => result,
        // SAFETY: default handling for our own window class.
        None => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

/// Scroll the viewport so `position` (physical pixels) is its top edge.
fn scroll_to(viewport: HWND, position: i32) {
    let mut info = SCROLLINFO {
        cbSize: mem::size_of::<SCROLLINFO>() as u32,
        fMask: SIF_POS,
        nMin: 0,
        nMax: 0,
        nPage: 0,
        nPos: position,
        nTrackPos: 0,
    };
    // SAFETY: `viewport` is a live scroll window and `info` is initialized;
    // Windows clamps the position to the range.
    unsafe {
        SetScrollInfo(viewport, SB_VERT, &info, 1);
        info.fMask = SIF_POS;
        GetScrollInfo(viewport, SB_VERT, &mut info);
        let content = GetWindow(viewport, GW_CHILD);
        if !content.is_null() {
            SetWindowPos(
                content,
                ptr::null_mut(),
                0,
                -info.nPos,
                0,
                0,
                SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    }
}

fn scroll_position(viewport: HWND) -> SCROLLINFO {
    let mut info = SCROLLINFO {
        cbSize: mem::size_of::<SCROLLINFO>() as u32,
        fMask: SIF_ALL,
        nMin: 0,
        nMax: 0,
        nPage: 0,
        nPos: 0,
        nTrackPos: 0,
    };
    // SAFETY: `viewport` is a live scroll window and `info` is initialized.
    unsafe { GetScrollInfo(viewport, SB_VERT, &mut info) };
    info
}

/// Size the content to its extent and the scroll bar to the viewport.
fn update_scroll(viewport: HWND) {
    let (width, height) = SHARED.with(|shared| {
        shared
            .extents
            .borrow()
            .get(&(viewport as usize))
            .copied()
            .unwrap_or_default()
    });
    let mut client = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    // SAFETY: `viewport` is a live scroll window.
    unsafe { GetClientRect(viewport, &mut client) };
    let info = SCROLLINFO {
        cbSize: mem::size_of::<SCROLLINFO>() as u32,
        fMask: SIF_RANGE | SIF_PAGE | SIF_DISABLENOSCROLL,
        nMin: 0,
        nMax: (height - 1).max(0),
        nPage: client.bottom.max(0) as u32,
        nPos: 0,
        nTrackPos: 0,
    };
    // SAFETY: as above; the scroll bar stays visible so the gutter is stable.
    unsafe {
        SetScrollInfo(viewport, SB_VERT, &info, 1);
        let content = GetWindow(viewport, GW_CHILD);
        if !content.is_null() {
            SetWindowPos(
                content,
                ptr::null_mut(),
                0,
                0,
                width.max(client.right),
                height.max(client.bottom),
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    }
    let position = scroll_position(viewport).nPos;
    scroll_to(viewport, position);
}

unsafe extern "system" fn scroll_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let line = (LINE * dpi_scale(hwnd)).round() as i32;
    match message {
        WM_VSCROLL if lparam == 0 => {
            let info = scroll_position(hwnd);
            let page = info.nPage as i32;
            let position = match low(wparam) as i32 {
                SB_LINEUP => info.nPos - line,
                SB_LINEDOWN => info.nPos + line,
                SB_PAGEUP => info.nPos - page,
                SB_PAGEDOWN => info.nPos + page,
                SB_THUMBTRACK => info.nTrackPos,
                SB_TOP => 0,
                SB_BOTTOM => info.nMax,
                _ => info.nPos,
            };
            scroll_to(hwnd, position);
            0
        }
        WM_MOUSEWHEEL => {
            let notches = high(wparam) as u16 as i16 as i32 / WHEEL_DELTA as i32;
            scroll_to(
                hwnd,
                scroll_position(hwnd).nPos - notches * WHEEL_LINES * line,
            );
            0
        }
        WM_SIZE => {
            update_scroll(hwnd);
            0
        }
        // SAFETY: default handling for our own window class.
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

/// Enter in a single-line edit submits instead of activating a default button.
unsafe extern "system" fn edit_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _: usize,
    id: usize,
) -> LRESULT {
    match message {
        WM_GETDLGCODE if wparam == VK_RETURN as usize => {
            // SAFETY: default subclass handling for this control.
            DLGC_WANTALLKEYS as LRESULT | unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
        }
        WM_KEYDOWN if wparam == VK_RETURN as usize => {
            push(Event::Submit(WidgetId(id as u64)));
            0
        }
        // Swallow the carriage return so the edit does not beep.
        WM_CHAR if wparam == '\r' as usize => 0,
        // SAFETY: default subclass handling for this control.
        _ => unsafe { DefSubclassProc(hwnd, message, wparam, lparam) },
    }
}

/// Keep a focused control inside any scroll viewports that contain it.
fn reveal(control: HWND) {
    let mut child = control;
    loop {
        // SAFETY: walking the parent chain of live windows.
        let parent = unsafe { GetParent(child) };
        if parent.is_null() {
            return;
        }
        let viewport = parent;
        let is_scroll =
            SHARED.with(|shared| shared.extents.borrow().contains_key(&(viewport as usize)));
        if is_scroll {
            let mut rect = RECT {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            };
            let mut client = rect;
            // SAFETY: both windows are live; MapWindowPoints converts the
            // control's screen rectangle into viewport client coordinates.
            unsafe {
                GetWindowRect(control, &mut rect);
                MapWindowPoints(
                    ptr::null_mut(),
                    viewport,
                    &mut rect as *mut RECT as *mut POINT,
                    2,
                );
                GetClientRect(viewport, &mut client);
            }
            let position = scroll_position(viewport).nPos;
            if rect.top < 0 {
                scroll_to(viewport, position + rect.top);
            } else if rect.bottom > client.bottom {
                scroll_to(
                    viewport,
                    position + (rect.bottom - client.bottom).min(rect.top),
                );
            }
        }
        child = parent;
    }
}

struct Native {
    hwnd: HWND,
    kind: Kind,
    /// The window that hosts this widget's children.
    host: Option<HWND>,
    parent: Option<WidgetId>,
    /// An explicit text size and the font made for it.
    font: Option<(f32, HFONT)>,
    bitmap: HBITMAP,
    image_size: (u32, u32),
    /// Children overlap, so later ones must be above earlier ones.
    overlap: bool,
    /// Scroll viewports: frames of direct children, physical pixels.
    frames: HashMap<WidgetId, RECT>,
}

pub struct Win32Backend {
    widgets: HashMap<WidgetId, Native>,
    instance: HINSTANCE,
    window: HWND,
    dpi: u32,
    message_font: LOGFONTW,
    font: HFONT,
}

fn create_font(logfont: &LOGFONTW) -> HFONT {
    // SAFETY: `logfont` is a fully initialized LOGFONTW.
    let font = unsafe { CreateFontIndirectW(logfont) };
    if font.is_null() {
        // SAFETY: stock objects are owned by the system.
        unsafe { GetStockObject(DEFAULT_GUI_FONT) }
    } else {
        font
    }
}

fn message_logfont(dpi: u32) -> LOGFONTW {
    // SAFETY: an all-zero NONCLIENTMETRICSW is a valid value to fill in.
    let mut metrics: NONCLIENTMETRICSW = unsafe { mem::zeroed() };
    metrics.cbSize = mem::size_of::<NONCLIENTMETRICSW>() as u32;
    // SAFETY: `metrics` is a writable NONCLIENTMETRICSW of the declared size.
    unsafe {
        SystemParametersInfoForDpi(
            SPI_GETNONCLIENTMETRICS,
            metrics.cbSize,
            &mut metrics as *mut NONCLIENTMETRICSW as *mut c_void,
            0,
            dpi,
        )
    };
    metrics.lfMessageFont
}

impl Win32Backend {
    fn native(&self, id: WidgetId) -> Result<&Native> {
        self.widgets
            .get(&id)
            .ok_or_else(|| format!("unknown widget {}", id.0).into())
    }

    fn scale(&self) -> f32 {
        self.dpi as f32 / 96.0
    }

    fn px(&self, logical: f32) -> i32 {
        (logical * self.scale()).round() as i32
    }

    fn sized_font(&self, size: f32) -> HFONT {
        let mut logfont = self.message_font;
        logfont.lfHeight = -self.px(size);
        create_font(&logfont)
    }

    /// Follow the window to a new monitor DPI: new fonts for every control.
    fn sync_dpi(&mut self) {
        if self.window.is_null() {
            return;
        }
        // SAFETY: querying our live top-level window.
        let dpi = unsafe { GetDpiForWindow(self.window) };
        if dpi == 0 || dpi == self.dpi {
            return;
        }
        self.dpi = dpi;
        self.message_font = message_logfont(dpi);
        let old = mem::replace(&mut self.font, create_font(&self.message_font));
        let sizes: Vec<_> = self
            .widgets
            .iter()
            .filter_map(|(id, native)| native.font.map(|(size, _)| (*id, size)))
            .collect();
        for (id, size) in sizes {
            let font = self.sized_font(size);
            if let Some(native) = self.widgets.get_mut(&id) {
                if let Some((_, old)) = native.font.replace((size, font)) {
                    // SAFETY: the old font is no longer selected into a control.
                    unsafe { DeleteObject(old) };
                }
            }
        }
        for native in self.widgets.values() {
            let font = native.font.map_or(self.font, |(_, font)| font);
            // SAFETY: live control; fonts outlive the controls using them.
            unsafe { SendMessageW(native.hwnd, WM_SETFONT, font as WPARAM, 1) };
        }
        // SAFETY: no control uses the previous message font any more.
        unsafe { DeleteObject(old) };
    }

    fn create_window(&mut self, options: &WindowOptions) -> Result<HWND> {
        let style = WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN;
        // SAFETY: plain query.
        let dpi = unsafe { GetDpiForSystem() };
        let scale = dpi as f32 / 96.0;
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: (options.size.width * scale).round() as i32,
            bottom: (options.size.height * scale).round() as i32,
        };
        // SAFETY: valid rectangle and styles.
        unsafe { AdjustWindowRectExForDpi(&mut rect, style, 0, WS_EX_CONTROLPARENT, dpi) };
        let title = wide(&options.title);
        // SAFETY: registered class name and NUL-terminated title outlive the call.
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_CONTROLPARENT,
                WINDOW_CLASS,
                title.as_ptr(),
                style,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                rect.right - rect.left,
                rect.bottom - rect.top,
                ptr::null_mut(),
                ptr::null_mut(),
                self.instance,
                ptr::null(),
            )
        };
        if hwnd.is_null() {
            return Err(last_error("creating the window"));
        }
        SHARED.with(|shared| shared.window.set(hwnd as usize));
        self.window = hwnd;
        self.dpi = dpi;
        self.message_font = message_logfont(dpi);
        self.font = create_font(&self.message_font);
        self.sync_dpi();
        Ok(hwnd)
    }

    #[allow(clippy::too_many_arguments)]
    fn control(
        &self,
        class: PCWSTR,
        text: &str,
        style: u32,
        ex_style: u32,
        parent: HWND,
        id: WidgetId,
        font: HFONT,
    ) -> Result<HWND> {
        let text = wide(text);
        // SAFETY: class and text are NUL-terminated and outlive the call; the
        // parent is a live host window.
        let hwnd = unsafe {
            CreateWindowExW(
                ex_style,
                class,
                text.as_ptr(),
                WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS | style,
                0,
                0,
                0,
                0,
                parent,
                ptr::null_mut(),
                self.instance,
                ptr::null(),
            )
        };
        if hwnd.is_null() {
            return Err(last_error("creating a control"));
        }
        // SAFETY: a standard control's user data belongs to the application;
        // the font outlives the control.
        unsafe {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, id.0 as isize);
            SendMessageW(hwnd, WM_SETFONT, font as WPARAM, 0);
        }
        Ok(hwnd)
    }

    fn host_window(&self, parent: HWND, class: PCWSTR, style: u32) -> Result<HWND> {
        // SAFETY: registered class; the parent is a live window.
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_CONTROLPARENT,
                class,
                w!(""),
                WS_CHILD | WS_VISIBLE | WS_CLIPCHILDREN | WS_CLIPSIBLINGS | style,
                0,
                0,
                0,
                0,
                parent,
                ptr::null_mut(),
                self.instance,
                ptr::null(),
            )
        };
        if hwnd.is_null() {
            Err(last_error("creating a layout host"))
        } else {
            Ok(hwnd)
        }
    }

    fn image(&self, path: &Path) -> Result<(HBITMAP, (u32, u32))> {
        let pixels = wic::decode(path)?;
        // SAFETY: plain query of a system color.
        let background = unsafe { windows_sys::Win32::Graphics::Gdi::GetSysColor(COLOR_BTNFACE) };
        let (red, green, blue) = (
            background & 0xff,
            (background >> 8) & 0xff,
            (background >> 16) & 0xff,
        );
        // Static bitmaps draw without alpha: composite over the host colour.
        let mut bgra = pixels.bgra;
        for pixel in bgra.as_chunks_mut::<4>().0 {
            let clear = 255 - pixel[3] as u32;
            pixel[0] = (pixel[0] as u32 + blue * clear / 255) as u8;
            pixel[1] = (pixel[1] as u32 + green * clear / 255) as u8;
            pixel[2] = (pixel[2] as u32 + red * clear / 255) as u8;
            pixel[3] = 255;
        }
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: pixels.width as i32,
                // Negative: rows run top down, as WIC copied them.
                biHeight: -(pixels.height as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                biSizeImage: 0,
                biXPelsPerMeter: 0,
                biYPelsPerMeter: 0,
                biClrUsed: 0,
                biClrImportant: 0,
            },
            bmiColors: [Default::default()],
        };
        let mut bits = ptr::null_mut();
        // SAFETY: `info` describes a 32-bit top-down DIB; Windows allocates
        // the section and returns its pixel pointer.
        let bitmap = unsafe {
            CreateDIBSection(
                ptr::null_mut(),
                &info,
                DIB_RGB_COLORS,
                &mut bits,
                ptr::null_mut(),
                0,
            )
        };
        if bitmap.is_null() || bits.is_null() {
            return Err(last_error(&format!(
                "{}: creating a bitmap",
                path.display()
            )));
        }
        // SAFETY: the section holds exactly width * height 32-bit pixels.
        unsafe { ptr::copy_nonoverlapping(bgra.as_ptr(), bits as *mut u8, bgra.len()) };
        Ok((bitmap, (pixels.width, pixels.height)))
    }

    /// Text extent of a control's current text in its font, physical pixels.
    fn text_extent(&self, hwnd: HWND, font: HFONT, text: &str) -> SIZE {
        let text: Vec<u16> = text.encode_utf16().collect();
        let mut size = SIZE { cx: 0, cy: 0 };
        // SAFETY: a device context for a live window, released below; the
        // previous font is restored before release.
        unsafe {
            let dc = GetDC(hwnd);
            let previous = SelectObject(dc, font);
            GetTextExtentPoint32W(dc, text.as_ptr(), text.len() as i32, &mut size);
            let mut metrics: TEXTMETRICW = mem::zeroed();
            GetTextMetricsW(dc, &mut metrics);
            size.cy = size.cy.max(metrics.tmHeight);
            SelectObject(dc, previous);
            ReleaseDC(hwnd, dc);
        }
        size
    }

    fn average_char_width(&self, hwnd: HWND, font: HFONT) -> i32 {
        // SAFETY: as in `text_extent`.
        unsafe {
            let dc = GetDC(hwnd);
            let previous = SelectObject(dc, font);
            let mut metrics: TEXTMETRICW = mem::zeroed();
            GetTextMetricsW(dc, &mut metrics);
            SelectObject(dc, previous);
            ReleaseDC(hwnd, dc);
            metrics.tmAveCharWidth.max(1)
        }
    }

    /// Place `order` in tab order; overlapping children are drawn the other
    /// way round, so later ones are on top.
    fn arrange(&self, overlap: bool, order: &[HWND]) {
        let mut previous = HWND_TOP;
        let windows: Vec<HWND> = if overlap {
            order.iter().rev().copied().collect()
        } else {
            order.to_vec()
        };
        for hwnd in windows {
            // SAFETY: live sibling windows under the same host.
            unsafe {
                SetWindowPos(
                    hwnd,
                    previous,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                )
            };
            previous = hwnd;
        }
    }

    fn set_slider_range(&self, hwnd: HWND, range: SliderRange) {
        let steps = ((range.max - range.min) / range.step).round().max(1.0) as isize;
        SHARED.with(|shared| shared.sliders.borrow_mut().insert(hwnd as usize, range));
        // SAFETY: live trackbar.
        unsafe {
            SendMessageW(hwnd, TBM_SETRANGEMIN, 0, 0);
            SendMessageW(hwnd, TBM_SETRANGEMAX, 1, steps);
            SendMessageW(hwnd, TBM_SETLINESIZE, 0, 1);
            SendMessageW(hwnd, TBM_SETPAGESIZE, 0, (steps / 10).max(1));
        }
    }

    fn set_slider_value(&self, hwnd: HWND, value: f64) {
        let Some(range) =
            SHARED.with(|shared| shared.sliders.borrow().get(&(hwnd as usize)).copied())
        else {
            return;
        };
        let position = ((value - range.min) / range.step).round() as isize;
        // SAFETY: live trackbar; TBM_SETPOS sends no notification.
        unsafe { SendMessageW(hwnd, TBM_SETPOS, 1, position) };
    }

    fn set_options(&self, hwnd: HWND, options: &[Cow<'static, str>], selected: Option<usize>) {
        quietly(|| {
            // SAFETY: live combo box; each string is NUL-terminated and copied.
            unsafe {
                SendMessageW(hwnd, CB_RESETCONTENT, 0, 0);
                for option in options {
                    let option = wide(option);
                    SendMessageW(hwnd, CB_ADDSTRING, 0, option.as_ptr() as LPARAM);
                }
            }
        });
        self.select(hwnd, selected);
    }

    fn select(&self, hwnd: HWND, selected: Option<usize>) {
        let index = selected.map_or(-1, |index| index as isize);
        // SAFETY: live combo box; CB_SETCURSEL sends no notification.
        unsafe { SendMessageW(hwnd, CB_SETCURSEL, index as usize, 0) };
    }
}

impl Backend for Win32Backend {
    fn create(
        &mut self,
        id: WidgetId,
        parent: Option<WidgetId>,
        properties: Properties<'_>,
    ) -> Result<()> {
        if let Properties::Window(options) = properties {
            let hwnd = self.create_window(options)?;
            self.widgets.insert(
                id,
                Native {
                    hwnd,
                    kind: Kind::Window,
                    host: Some(hwnd),
                    parent: None,
                    font: None,
                    bitmap: ptr::null_mut(),
                    image_size: (0, 0),
                    overlap: false,
                    frames: HashMap::new(),
                },
            );
            return Ok(());
        }
        self.sync_dpi();
        let parent_id = parent.ok_or("a control needs a parent")?;
        let parent_native = self.native(parent_id)?;
        let host = parent_native
            .host
            .ok_or_else(|| format!("widget {} cannot have children", parent_id.0))?;
        let overlap = parent_native.overlap;
        let mut native = Native {
            hwnd: ptr::null_mut(),
            kind: properties.kind(),
            host: None,
            parent,
            font: None,
            bitmap: ptr::null_mut(),
            image_size: (0, 0),
            overlap: false,
            frames: HashMap::new(),
        };
        let tab = WS_TABSTOP;
        native.hwnd = match properties {
            Properties::Window(_) => unreachable!("handled above"),
            Properties::Group { overlap } => {
                let hwnd = self.host_window(host, HOST_CLASS, 0)?;
                native.host = Some(hwnd);
                native.overlap = overlap;
                hwnd
            }
            Properties::Scroll => {
                let viewport = self.host_window(host, SCROLL_CLASS, WS_VSCROLL)?;
                let content = self.host_window(viewport, HOST_CLASS, 0)?;
                SHARED.with(|shared| {
                    shared
                        .extents
                        .borrow_mut()
                        .insert(viewport as usize, (0, 0))
                });
                native.host = Some(content);
                update_scroll(viewport);
                viewport
            }
            Properties::Text { value, size } => {
                let font = match size {
                    Some(size) => {
                        let font = self.sized_font(size);
                        native.font = Some((size, font));
                        font
                    }
                    None => self.font,
                };
                let style = SS_LEFTNOWORDWRAP | SS_NOPREFIX;
                self.control(WC_STATICW, value, style, 0, host, id, font)?
            }
            Properties::Button(label) => {
                let style = BS_PUSHBUTTON as u32 | tab;
                self.control(WC_BUTTONW, label, style, 0, host, id, self.font)?
            }
            Properties::TextInput {
                value, placeholder, ..
            } => {
                let style = ES_AUTOHSCROLL as u32 | tab;
                let hwnd = quietly(|| {
                    self.control(
                        WC_EDITW,
                        value,
                        style,
                        WS_EX_CLIENTEDGE,
                        host,
                        id,
                        self.font,
                    )
                })?;
                let placeholder = wide(placeholder);
                // SAFETY: live edit control; the cue text is copied; the
                // subclass procedure lives for the program's lifetime.
                unsafe {
                    SendMessageW(hwnd, EM_SETCUEBANNER, 0, placeholder.as_ptr() as LPARAM);
                    SetWindowSubclass(hwnd, Some(edit_proc), 1, id.0 as usize);
                }
                hwnd
            }
            Properties::Checkbox { label, checked } => {
                let style = BS_AUTOCHECKBOX as u32 | tab;
                let hwnd = self.control(WC_BUTTONW, label, style, 0, host, id, self.font)?;
                let state = if checked { BST_CHECKED } else { BST_UNCHECKED };
                // SAFETY: live check box; BM_SETCHECK sends no notification.
                unsafe { SendMessageW(hwnd, BM_SETCHECK, state as usize, 0) };
                hwnd
            }
            Properties::Slider { value, range, .. } => {
                let style = TBS_NOTICKS | tab;
                let hwnd = self.control(TRACKBAR_CLASSW, "", style, 0, host, id, self.font)?;
                self.set_slider_range(hwnd, range);
                self.set_slider_value(hwnd, value);
                hwnd
            }
            Properties::Progress { fraction, .. } => {
                let hwnd = self.control(PROGRESS_CLASSW, "", 0, 0, host, id, self.font)?;
                // SAFETY: live progress bar.
                unsafe {
                    SendMessageW(hwnd, PBM_SETRANGE32, 0, 1000);
                    SendMessageW(hwnd, PBM_SETPOS, (fraction * 1000.0).round() as usize, 0);
                }
                hwnd
            }
            Properties::Dropdown {
                options, selected, ..
            } => {
                let style = CBS_DROPDOWNLIST as u32 | WS_VSCROLL | tab;
                let hwnd = self.control(WC_COMBOBOXW, "", style, 0, host, id, self.font)?;
                self.set_options(hwnd, options, selected);
                hwnd
            }
            Properties::Image { path, .. } => {
                let (bitmap, size) = self.image(path)?;
                let style = SS_BITMAP | SS_REALSIZECONTROL;
                let hwnd = self.control(WC_STATICW, "", style, 0, host, id, self.font)?;
                // SAFETY: live static control; the bitmap is owned by this
                // backend and deleted after the control.
                unsafe {
                    SendMessageW(hwnd, STM_SETIMAGE, IMAGE_BITMAP as usize, bitmap as LPARAM)
                };
                native.bitmap = bitmap;
                native.image_size = size;
                hwnd
            }
        };
        // New children go last in tab order, or on top of overlapping siblings.
        let position = if overlap { HWND_TOP } else { HWND_BOTTOM };
        // SAFETY: the new window is a live child of `host`.
        unsafe {
            SetWindowPos(
                native.hwnd,
                position,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            )
        };
        self.widgets.insert(id, native);
        Ok(())
    }

    fn update(&mut self, id: WidgetId, change: Change<'_>) -> Result<()> {
        self.sync_dpi();
        let hwnd = self.native(id)?.hwnd;
        let set_text = |text: &str| {
            let text = wide(text);
            // SAFETY: live window; the text is NUL-terminated and copied.
            unsafe { SetWindowTextW(hwnd, text.as_ptr()) };
        };
        match change {
            Change::Text(value) | Change::Label(value) => set_text(value),
            Change::Value(value) => quietly(|| set_text(value)),
            Change::TextSize(size) => {
                let font = size.map(|size| (size, self.sized_font(size)));
                let native = self.widgets.get_mut(&id).ok_or("unknown widget")?;
                let old = mem::replace(&mut native.font, font);
                let current = native.font.map_or(self.font, |(_, font)| font);
                // SAFETY: live control; the old font is deleted only after
                // the control stops using it.
                unsafe {
                    SendMessageW(hwnd, WM_SETFONT, current as WPARAM, 1);
                    if let Some((_, old)) = old {
                        DeleteObject(old);
                    }
                }
            }
            // Win32 names inputs after the label preceding them; done's
            // accessible-only labels are not mapped yet.
            Change::AccessibleLabel(_) => {}
            Change::Placeholder(value) => {
                let value = wide(value);
                // SAFETY: live edit control; the cue text is copied.
                unsafe { SendMessageW(hwnd, EM_SETCUEBANNER, 0, value.as_ptr() as LPARAM) };
            }
            Change::Checked(checked) => {
                let state = if checked { BST_CHECKED } else { BST_UNCHECKED };
                // SAFETY: live check box; BM_SETCHECK sends no notification.
                unsafe { SendMessageW(hwnd, BM_SETCHECK, state as usize, 0) };
            }
            Change::SliderValue(value) => self.set_slider_value(hwnd, value),
            Change::SliderRange(range) => self.set_slider_range(hwnd, range),
            Change::Fraction(fraction) => {
                // SAFETY: live progress bar.
                unsafe { SendMessageW(hwnd, PBM_SETPOS, (fraction * 1000.0).round() as usize, 0) };
            }
            Change::Options(options) => {
                // SAFETY: live combo box.
                let current = unsafe { SendMessageW(hwnd, CB_GETCURSEL, 0, 0) };
                let selected = (current != CB_ERR as isize).then_some(current as usize);
                self.set_options(hwnd, options, selected);
            }
            Change::Selected(selected) => self.select(hwnd, selected),
            Change::Image(path) => {
                let (bitmap, size) = self.image(path)?;
                let native = self.widgets.get_mut(&id).ok_or("unknown widget")?;
                // SAFETY: live static control; the previous bitmap is
                // returned by the control and deleted once replaced.
                unsafe {
                    SendMessageW(hwnd, STM_SETIMAGE, IMAGE_BITMAP as usize, bitmap as LPARAM);
                    if !native.bitmap.is_null() {
                        DeleteObject(native.bitmap);
                    }
                }
                native.bitmap = bitmap;
                native.image_size = size;
            }
            Change::Order(order) => {
                let native = self.native(id)?;
                let windows = order
                    .iter()
                    .map(|child| Ok(self.native(*child)?.hwnd))
                    .collect::<Result<Vec<_>>>()?;
                self.arrange(native.overlap, &windows);
            }
            Change::MinimumSize(size) => {
                SHARED.with(|shared| shared.minimum.set((size.width, size.height)));
                let mut client = RECT {
                    left: 0,
                    top: 0,
                    right: 0,
                    bottom: 0,
                };
                // SAFETY: live top-level window.
                unsafe { GetClientRect(hwnd, &mut client) };
                let (width, height) = (self.px(size.width), self.px(size.height));
                if client.right < width || client.bottom < height {
                    let mut rect = RECT {
                        left: 0,
                        top: 0,
                        right: client.right.max(width),
                        bottom: client.bottom.max(height),
                    };
                    // SAFETY: live window; growing it to the layout's minimum.
                    unsafe {
                        AdjustWindowRectExForDpi(
                            &mut rect,
                            GetWindowLongPtrW(hwnd, GWL_STYLE) as u32,
                            0,
                            WS_EX_CONTROLPARENT,
                            self.dpi,
                        );
                        SetWindowPos(
                            hwnd,
                            ptr::null_mut(),
                            0,
                            0,
                            rect.right - rect.left,
                            rect.bottom - rect.top,
                            SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
                        );
                    }
                }
            }
            Change::Overlap(overlap) => {
                let native = self.widgets.get_mut(&id).ok_or("unknown widget")?;
                native.overlap = overlap;
                let host = native.host.ok_or("not a group")?;
                let mut children = Vec::new();
                // SAFETY: walking the live children of our own host window.
                let mut child = unsafe { GetWindow(host, GW_CHILD) };
                while !child.is_null() {
                    children.push(child);
                    // SAFETY: as above.
                    child = unsafe { GetWindow(child, GW_HWNDNEXT) };
                }
                // Reverse the stacking: the order was the other way round.
                children.reverse();
                self.arrange(false, &children);
            }
        }
        Ok(())
    }

    fn destroy(&mut self, id: WidgetId) {
        let Some(native) = self.widgets.remove(&id) else {
            return;
        };
        if let Some(parent) = native
            .parent
            .and_then(|parent| self.widgets.get_mut(&parent))
        {
            if parent.kind == Kind::Scroll && parent.frames.remove(&id).is_some() {
                let extent = parent
                    .frames
                    .values()
                    .fold((0, 0), |(width, height), rect| {
                        (width.max(rect.right), height.max(rect.bottom))
                    });
                let viewport = parent.hwnd;
                SHARED.with(|shared| {
                    shared
                        .extents
                        .borrow_mut()
                        .insert(viewport as usize, extent)
                });
                update_scroll(viewport);
            }
        }
        SHARED.with(|shared| {
            shared.sliders.borrow_mut().remove(&(native.hwnd as usize));
            shared.extents.borrow_mut().remove(&(native.hwnd as usize));
        });
        if native.kind == Kind::Window {
            self.window = ptr::null_mut();
        }
        // SAFETY: the window is ours and destroyed once; GDI objects are
        // deleted after the control that used them.
        unsafe {
            DestroyWindow(native.hwnd);
            if let Some((_, font)) = native.font {
                DeleteObject(font);
            }
            if !native.bitmap.is_null() {
                DeleteObject(native.bitmap);
            }
        }
    }

    fn set_frame(&mut self, id: WidgetId, rect: Rect) -> Result<()> {
        self.sync_dpi();
        let scale = self.scale();
        let native = self.native(id)?;
        // Round edges, not sizes, so siblings never gap or overlap.
        let left = (rect.x * scale).round() as i32;
        let top = (rect.y * scale).round() as i32;
        let right = ((rect.x + rect.width) * scale).round() as i32;
        let bottom = ((rect.y + rect.height) * scale).round() as i32;
        let extra = if native.kind == Kind::Dropdown {
            self.px(LIST_HEIGHT)
        } else {
            0
        };
        // SAFETY: live child window.
        unsafe {
            SetWindowPos(
                native.hwnd,
                ptr::null_mut(),
                left,
                top,
                right - left,
                bottom - top + extra,
                SWP_NOZORDER | SWP_NOACTIVATE,
            )
        };
        if let Some(parent) = native
            .parent
            .and_then(|parent| self.widgets.get_mut(&parent))
        {
            if parent.kind == Kind::Scroll {
                parent.frames.insert(
                    id,
                    RECT {
                        left,
                        top,
                        right,
                        bottom,
                    },
                );
                let extent = parent
                    .frames
                    .values()
                    .fold((0, 0), |(width, height), rect| {
                        (width.max(rect.right), height.max(rect.bottom))
                    });
                let viewport = parent.hwnd;
                SHARED.with(|shared| {
                    shared
                        .extents
                        .borrow_mut()
                        .insert(viewport as usize, extent)
                });
                update_scroll(viewport);
            }
        }
        Ok(())
    }

    fn measure(&mut self, id: WidgetId, _: Constraints) -> Result<Measurement> {
        self.sync_dpi();
        let scale = self.scale();
        let native = self.native(id)?;
        let hwnd = native.hwnd;
        let font = native.font.map_or(self.font, |(_, font)| font);
        let (minimum, preferred) = match native.kind {
            Kind::Text => {
                let size = self.text_extent(hwnd, font, &window_text(hwnd));
                (size, size)
            }
            Kind::Button => {
                let mut ideal = SIZE { cx: 0, cy: 0 };
                // SAFETY: live button; BCM_GETIDEALSIZE fills `ideal` when
                // common controls v6 are active.
                let ok = unsafe {
                    SendMessageW(hwnd, BCM_GETIDEALSIZE, 0, &mut ideal as *mut SIZE as LPARAM)
                };
                if ok == 0 || ideal.cx == 0 {
                    let text = self.text_extent(hwnd, font, &window_text(hwnd));
                    ideal = SIZE {
                        cx: text.cx + self.px(20.0),
                        cy: text.cy + self.px(8.0),
                    };
                }
                ideal.cy = ideal.cy.max(self.px(23.0));
                (ideal, ideal)
            }
            Kind::Checkbox => {
                let text = self.text_extent(hwnd, font, &window_text(hwnd));
                // SAFETY: plain metric query.
                let glyph = unsafe { GetSystemMetricsForDpi(SM_CXMENUCHECK, self.dpi) };
                let size = SIZE {
                    cx: glyph + self.px(6.0) + text.cx,
                    cy: glyph.max(text.cy) + self.px(2.0),
                };
                (size, size)
            }
            Kind::TextInput => {
                let average = self.average_char_width(hwnd, font);
                let height =
                    (self.text_extent(hwnd, font, "").cy + self.px(9.0)).max(self.px(23.0));
                (
                    SIZE {
                        cx: average * 8,
                        cy: height,
                    },
                    SIZE {
                        cx: average * 24,
                        cy: height,
                    },
                )
            }
            Kind::Slider => (
                SIZE {
                    cx: self.px(80.0),
                    cy: self.px(30.0),
                },
                SIZE {
                    cx: self.px(160.0),
                    cy: self.px(30.0),
                },
            ),
            Kind::Progress => (
                SIZE {
                    cx: self.px(40.0),
                    cy: self.px(15.0),
                },
                SIZE {
                    cx: self.px(160.0),
                    cy: self.px(15.0),
                },
            ),
            Kind::Dropdown => {
                let mut field = RECT {
                    left: 0,
                    top: 0,
                    right: 0,
                    bottom: 0,
                };
                // SAFETY: live combo box; its window rectangle is the
                // selection field, the list being a separate popup.
                unsafe { GetWindowRect(hwnd, &mut field) };
                let count = {
                    // SAFETY: live combo box.
                    unsafe {
                        SendMessageW(
                            hwnd,
                            windows_sys::Win32::UI::WindowsAndMessaging::CB_GETCOUNT,
                            0,
                            0,
                        )
                    }
                    .max(0)
                };
                let mut widest = 0;
                for index in 0..count {
                    let mut buffer = [0u16; 256];
                    // SAFETY: live combo box; the buffer is large enough for
                    // 255 characters, longer items are measured truncated.
                    let length = unsafe {
                        let length = SendMessageW(
                            hwnd,
                            windows_sys::Win32::UI::WindowsAndMessaging::CB_GETLBTEXTLEN,
                            index as usize,
                            0,
                        );
                        if length < 0 || length >= buffer.len() as isize {
                            0
                        } else {
                            SendMessageW(
                                hwnd,
                                windows_sys::Win32::UI::WindowsAndMessaging::CB_GETLBTEXT,
                                index as usize,
                                buffer.as_mut_ptr() as LPARAM,
                            )
                            .max(0) as usize
                        }
                    };
                    let item = String::from_utf16_lossy(&buffer[..length]);
                    widest = widest.max(self.text_extent(hwnd, font, &item).cx);
                }
                // SAFETY: plain metric query.
                let arrow = unsafe { GetSystemMetricsForDpi(SM_CXVSCROLL, self.dpi) };
                let height = (field.bottom - field.top)
                    .max(self.text_extent(hwnd, font, "").cy + self.px(8.0));
                let size = SIZE {
                    cx: widest + arrow + self.px(12.0),
                    cy: height,
                };
                (size, size)
            }
            Kind::Image => {
                // One image pixel per logical unit, scaled with the display.
                let (width, height) = native.image_size;
                let size = SIZE {
                    cx: self.px(width as f32),
                    cy: self.px(height as f32),
                };
                (size, size)
            }
            Kind::Window | Kind::Group | Kind::Scroll => {
                (SIZE { cx: 0, cy: 0 }, SIZE { cx: 0, cy: 0 })
            }
        };
        let logical = |size: SIZE| Size {
            width: size.cx as f32 / scale,
            height: size.cy as f32 / scale,
        };
        Ok(Measurement {
            minimum: logical(minimum),
            preferred: logical(preferred),
        })
    }

    fn scrollbar_gutter(&self) -> f32 {
        // SAFETY: plain metric query.
        let width = unsafe { GetSystemMetricsForDpi(SM_CXVSCROLL, self.dpi.max(96)) };
        width as f32 / self.scale().max(1.0 / 96.0)
    }

    fn post(&self, event: Event) {
        push(event);
    }

    fn run_loop(
        mut dispatch: impl FnMut(&mut Self, &[Event]) -> Result<()> + 'static,
    ) -> Result<()> {
        // SAFETY: called before any window exists; failure means awareness
        // was already set, e.g. by an application manifest.
        unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
        // SAFETY: COM for WIC on this, the UI, thread; balanced below.
        let com = unsafe { CoInitializeEx(ptr::null(), COINIT_APARTMENTTHREADED as _) };
        let controls = INITCOMMONCONTROLSEX {
            dwSize: mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_STANDARD_CLASSES | ICC_BAR_CLASSES | ICC_PROGRESS_CLASS,
        };
        // SAFETY: initialized structure of the declared size.
        unsafe { InitCommonControlsEx(&controls) };
        // SAFETY: the module handle of this executable.
        let instance = unsafe { GetModuleHandleW(ptr::null()) };
        for (class, procedure) in [
            (
                WINDOW_CLASS,
                window_proc as unsafe extern "system" fn(_, _, _, _) -> _,
            ),
            (HOST_CLASS, host_proc),
            (SCROLL_CLASS, scroll_proc),
        ] {
            let class = WNDCLASSEXW {
                cbSize: mem::size_of::<WNDCLASSEXW>() as u32,
                style: 0,
                lpfnWndProc: Some(procedure),
                cbClsExtra: 0,
                cbWndExtra: 0,
                hInstance: instance,
                hIcon: ptr::null_mut(),
                // SAFETY: a system cursor.
                hCursor: unsafe { LoadCursorW(ptr::null_mut(), IDC_ARROW) },
                // SAFETY: system color brushes are owned by the system.
                hbrBackground: unsafe { GetSysColorBrush(COLOR_BTNFACE) },
                lpszMenuName: ptr::null(),
                lpszClassName: class,
                hIconSm: ptr::null_mut(),
            };
            // SAFETY: initialized class description with static strings;
            // re-registration after a previous run fails harmlessly.
            unsafe { RegisterClassExW(&class) };
        }
        let mut backend = Self {
            widgets: HashMap::new(),
            instance,
            window: ptr::null_mut(),
            dpi: 96,
            // SAFETY: an all-zero LOGFONTW is valid; replaced on window creation.
            message_font: unsafe { mem::zeroed() },
            font: ptr::null_mut(),
        };
        let reset = |com: i32| {
            SHARED.with(|shared| {
                shared.dispatch.borrow_mut().take();
                shared.pending.borrow_mut().clear();
                shared.scheduled.set(false);
                shared.window.set(0);
            });
            if com >= 0 {
                // SAFETY: balances the successful CoInitializeEx above.
                unsafe { CoUninitialize() };
            }
        };
        let started = dispatch(&mut backend, &[Event::Started]);
        let window = backend.window;
        if let Err(error) = started {
            drop(backend);
            reset(com);
            return Err(error);
        }
        if window.is_null() {
            drop(backend);
            reset(com);
            return Err("app did not create a window".into());
        }
        let failure = Rc::new(RefCell::new(None));
        let pair = Rc::new(RefCell::new((backend, dispatch)));
        let weak = Rc::downgrade(&pair);
        let observed = failure.clone();
        SHARED.with(|shared| {
            *shared.dispatch.borrow_mut() = Some(Box::new(move |events: Vec<Event>| {
                if let Some(pair) = weak.upgrade() {
                    let mut pair = pair.borrow_mut();
                    let (backend, dispatch) = &mut *pair;
                    if let Err(error) = dispatch(backend, &events) {
                        *observed.borrow_mut() = Some(error);
                        // SAFETY: ends this thread's message loop.
                        unsafe { PostQuitMessage(0) };
                    }
                }
            }))
        });
        // SAFETY: our live top-level window.
        unsafe { ShowWindow(window, SW_SHOW) };
        let mut focus = ptr::null_mut();
        loop {
            // SAFETY: an all-zero MSG is a valid value for GetMessageW to fill.
            let mut message: MSG = unsafe { mem::zeroed() };
            // SAFETY: standard message retrieval for this thread.
            let status = unsafe { GetMessageW(&mut message, ptr::null_mut(), 0, 0) };
            if status == 0 {
                break;
            }
            if status == -1 {
                return Err(last_error("reading window messages"));
            }
            if message.hwnd.is_null() && message.message == WM_FLUSH {
                flush();
                continue;
            }
            let window = SHARED.with(|shared| shared.window.get()) as HWND;
            // SAFETY: standard dialog-style keyboard navigation and dispatch.
            unsafe {
                if window.is_null() || IsDialogMessageW(window, &message) == 0 {
                    TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
            // SAFETY: plain query.
            let current = unsafe { GetFocus() };
            if current != focus {
                focus = current;
                if !focus.is_null() {
                    reveal(focus);
                }
            }
        }
        drop(pair);
        reset(com);
        let result = match failure.borrow_mut().take() {
            Some(error) => Err(error),
            None => Ok(()),
        };
        result
    }
}

impl Drop for Win32Backend {
    fn drop(&mut self) {
        // SAFETY: destroying our own windows (children first by Windows) and
        // deleting GDI objects no window still uses.
        unsafe {
            if !self.window.is_null() {
                DestroyWindow(self.window);
            }
            for native in self.widgets.values() {
                if let Some((_, font)) = native.font {
                    DeleteObject(font);
                }
                if !native.bitmap.is_null() {
                    DeleteObject(native.bitmap);
                }
            }
            if !self.font.is_null() {
                DeleteObject(self.font);
            }
        }
        self.widgets.clear();
    }
}
