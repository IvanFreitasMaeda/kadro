//! Puts a file (a recording) on the clipboard, the same way Finder or Explorer do
//! when you copy a file, so it can be pasted into a chat, an email or a folder.

use std::path::Path;

#[cfg(target_os = "macos")]
pub fn copy_file(path: &Path) -> Result<(), String> {
    use objc2::{class, msg_send, runtime::AnyObject};
    use objc2_foundation::{NSArray, NSString, NSURL};

    let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
    let urls = NSArray::from_retained_slice(&[url]);
    unsafe {
        let board: *mut AnyObject = msg_send![class!(NSPasteboard), generalPasteboard];
        let board = board.as_ref().ok_or("área de transferência indisponível")?;
        let _: isize = msg_send![board, clearContents];
        let ok: bool = msg_send![board, writeObjects: &*urls];
        if !ok {
            return Err("não foi possível copiar o arquivo".into());
        }
    }
    Ok(())
}

#[cfg(target_os = "windows")]
pub fn copy_file(path: &Path) -> Result<(), String> {
    use std::{mem::size_of, os::windows::ffi::OsStrExt};
    use windows::Win32::{
        Foundation::{GlobalFree, HANDLE, HGLOBAL},
        System::{
            DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData},
            Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE, GMEM_ZEROINIT},
            Ole::CF_HDROP,
        },
        UI::Shell::DROPFILES,
    };

    // DROPFILES header followed by the path list: each path NUL-terminated, plus a
    // final NUL.
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0, 0]).collect();
    let header = size_of::<DROPFILES>();
    let size = header + wide.len() * 2;
    let err = |e: windows::core::Error| format!("não foi possível copiar o arquivo: {e}");

    unsafe {
        let memory: HGLOBAL = GlobalAlloc(GMEM_MOVEABLE | GMEM_ZEROINIT, size).map_err(err)?;
        let ptr = GlobalLock(memory) as *mut u8;
        if ptr.is_null() {
            let _ = GlobalFree(Some(memory));
            return Err("não foi possível copiar o arquivo".into());
        }
        let files = ptr as *mut DROPFILES;
        (*files).pFiles = header as u32;
        (*files).fWide = true.into();
        std::ptr::copy_nonoverlapping(wide.as_ptr() as *const u8, ptr.add(header), wide.len() * 2);
        let _ = GlobalUnlock(memory);

        // Another app may be holding the clipboard for a moment.
        let mut opened = OpenClipboard(None);
        for _ in 0..10 {
            if opened.is_ok() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(30));
            opened = OpenClipboard(None);
        }
        if let Err(e) = opened {
            let _ = GlobalFree(Some(memory));
            return Err(err(e));
        }
        let result = EmptyClipboard()
            .and_then(|_| SetClipboardData(CF_HDROP.0 as u32, Some(HANDLE(memory.0))).map(|_| ()));
        let _ = CloseClipboard();
        if let Err(e) = result {
            let _ = GlobalFree(Some(memory));
            return Err(err(e));
        }
    }
    Ok(())
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn copy_file(_path: &Path) -> Result<(), String> {
    Err("Copiar arquivos não é suportado neste sistema.".into())
}
