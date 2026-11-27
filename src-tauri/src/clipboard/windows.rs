//! Windows side of the clipboard history: the clipboard sequence number, items to
//! skip, and the window that was in front.

use std::sync::atomic::{AtomicIsize, Ordering};

use windows::{
    core::w,
    Win32::{
        Foundation::{HGLOBAL, HWND},
        System::{
            DataExchange::{
                CloseClipboard, GetClipboardData, GetClipboardSequenceNumber,
                IsClipboardFormatAvailable, OpenClipboard, RegisterClipboardFormatW,
            },
            Memory::{GlobalLock, GlobalUnlock},
        },
        UI::WindowsAndMessaging::{GetForegroundWindow, IsWindow, SetForegroundWindow},
    },
};

/// The window that was in front when the popup opened.
static PREVIOUS: AtomicIsize = AtomicIsize::new(0);

pub fn change_count() -> i64 {
    unsafe { GetClipboardSequenceNumber() as i64 }
}

/// Password managers mark what they copy with these formats (the same ones Windows'
/// own clipboard history respects).
pub fn should_skip() -> bool {
    unsafe {
        for name in [
            w!("ExcludeClipboardContentFromMonitorProcessing"),
            w!("Clipboard Viewer Ignore"),
        ] {
            let format = RegisterClipboardFormatW(name);
            if format != 0 && IsClipboardFormatAvailable(format).is_ok() {
                return true;
            }
        }
        // CanIncludeInClipboardHistory = 0 means "keep out of the history".
        let format = RegisterClipboardFormatW(w!("CanIncludeInClipboardHistory"));
        if format == 0 || IsClipboardFormatAvailable(format).is_err() {
            return false;
        }
        if OpenClipboard(None).is_err() {
            return false;
        }
        let mut skip = false;
        if let Ok(handle) = GetClipboardData(format) {
            let memory = HGLOBAL(handle.0);
            let ptr = GlobalLock(memory) as *const u32;
            if !ptr.is_null() {
                skip = ptr.read_unaligned() == 0;
                let _ = GlobalUnlock(memory);
            }
        }
        let _ = CloseClipboard();
        skip
    }
}

pub fn remember_foreground() {
    let hwnd = unsafe { GetForegroundWindow() };
    if !hwnd.is_invalid() {
        PREVIOUS.store(hwnd.0 as isize, Ordering::SeqCst);
    }
}

/// Brings back the window that was in front when the popup opened.
pub fn activate_previous() {
    let hwnd = HWND(PREVIOUS.load(Ordering::SeqCst) as *mut _);
    unsafe {
        if !hwnd.is_invalid() && IsWindow(Some(hwnd)).as_bool() {
            let _ = SetForegroundWindow(hwnd);
        }
    }
}
