//! X11 helpers (also used through XWayland on Wayland): the window in front, and a
//! clipboard change counter fed by XFixes, so the clipboard history never has to
//! read the clipboard to know whether it changed.

use std::{
    sync::{
        atomic::{AtomicI64, AtomicU32, Ordering},
        Once,
    },
    time::{Duration, Instant},
};

use xcb::{x, xfixes, Xid, XidNew};

fn connect() -> Option<(xcb::Connection, x::Window)> {
    let (conn, screen) =
        xcb::Connection::connect_with_extensions(None, &[xcb::Extension::XFixes], &[]).ok()?;
    let root = conn.get_setup().roots().nth(screen as usize)?.root();
    Some((conn, root))
}

fn atom(conn: &xcb::Connection, name: &str) -> Option<x::Atom> {
    let cookie = conn.send_request(&x::InternAtom {
        only_if_exists: false,
        name: name.as_bytes(),
    });
    conn.wait_for_reply(cookie).ok().map(|r| r.atom())
}

// ---------------------------------------------------------------------------
// The window in front

static PREVIOUS: AtomicU32 = AtomicU32::new(0);

fn active_window(conn: &xcb::Connection, root: x::Window) -> Option<x::Window> {
    let net_active = atom(conn, "_NET_ACTIVE_WINDOW")?;
    let cookie = conn.send_request(&x::GetProperty {
        delete: false,
        window: root,
        property: net_active,
        r#type: x::ATOM_WINDOW,
        long_offset: 0,
        long_length: 1,
    });
    let reply = conn.wait_for_reply(cookie).ok()?;
    reply
        .value::<x::Window>()
        .first()
        .copied()
        .filter(|w| !w.is_none())
}

/// Remembers the window in front (the app the user was in).
pub fn remember_foreground() {
    let Some((conn, root)) = connect() else {
        return;
    };
    if let Some(window) = active_window(&conn, root) {
        PREVIOUS.store(window.resource_id(), Ordering::SeqCst);
    }
}

/// Gives the focus back to the window remembered by [`remember_foreground`].
pub fn activate_previous() {
    let id = PREVIOUS.load(Ordering::SeqCst);
    if id == 0 {
        return;
    }
    let Some((conn, root)) = connect() else {
        return;
    };
    let Some(net_active) = atom(&conn, "_NET_ACTIVE_WINDOW") else {
        return;
    };
    let window = x::Window::new(id);
    // Source 2: a pager-like request, which window managers honor.
    let event = x::ClientMessageEvent::new(
        window,
        net_active,
        x::ClientMessageData::Data32([2, x::CURRENT_TIME, 0, 0, 0]),
    );
    conn.send_request(&x::SendEvent {
        propagate: false,
        destination: x::SendEventDest::Window(root),
        event_mask: x::EventMask::SUBSTRUCTURE_REDIRECT | x::EventMask::SUBSTRUCTURE_NOTIFY,
        event: &event,
    });
    let _ = conn.flush();
}

// ---------------------------------------------------------------------------
// Clipboard

static CLIPBOARD_CHANGES: AtomicI64 = AtomicI64::new(0);

/// A number that moves each time something new is copied.
pub fn clipboard_change_count() -> i64 {
    static START: Once = Once::new();
    START.call_once(|| {
        std::thread::Builder::new()
            .name("x11-clipboard-watch".into())
            .spawn(watch_clipboard)
            .ok();
    });
    CLIPBOARD_CHANGES.load(Ordering::SeqCst)
}

fn watch_clipboard() {
    let Some((conn, root)) = connect() else {
        log::warn!("clipboard history: no X11 display");
        return;
    };
    let version = conn.send_request(&xfixes::QueryVersion {
        client_major_version: 5,
        client_minor_version: 0,
    });
    if conn.wait_for_reply(version).is_err() {
        log::warn!("clipboard history: XFixes is missing");
        return;
    }
    let Some(clipboard) = atom(&conn, "CLIPBOARD") else {
        return;
    };
    conn.send_request(&xfixes::SelectSelectionInput {
        window: root,
        selection: clipboard,
        event_mask: xfixes::SelectionEventMask::SET_SELECTION_OWNER
            | xfixes::SelectionEventMask::SELECTION_WINDOW_DESTROY
            | xfixes::SelectionEventMask::SELECTION_CLIENT_CLOSE,
    });
    if conn.flush().is_err() {
        return;
    }
    loop {
        match conn.wait_for_event() {
            Ok(xcb::Event::XFixes(xfixes::Event::SelectionNotify(ev)))
                if ev.subtype() == xfixes::SelectionEvent::SetSelectionOwner =>
            {
                CLIPBOARD_CHANGES.fetch_add(1, Ordering::SeqCst);
            }
            Ok(_) => {}
            Err(xcb::Error::Connection(err)) => {
                log::warn!("clipboard watch stopped: {err}");
                return;
            }
            Err(_) => {}
        }
    }
}

/// Whether what's on the clipboard is marked as secret: KeePassXC, Bitwarden and
/// others add `x-kde-passwordManagerHint` (KDE's own history respects it).
pub fn clipboard_is_secret() -> bool {
    clipboard_targets().is_some_and(|targets| {
        targets
            .iter()
            .any(|t| t == "x-kde-passwordManagerHint" || t == "x-kde-passwordmanagerhint")
    })
}

/// The formats offered by the clipboard's owner.
fn clipboard_targets() -> Option<Vec<String>> {
    let (conn, root) = connect()?;
    let clipboard = atom(&conn, "CLIPBOARD")?;
    let targets = atom(&conn, "TARGETS")?;
    let property = atom(&conn, "KADRO_TARGETS")?;
    let window: x::Window = conn.generate_id();
    conn.send_request(&x::CreateWindow {
        depth: x::COPY_FROM_PARENT as u8,
        wid: window,
        parent: root,
        x: -10,
        y: -10,
        width: 1,
        height: 1,
        border_width: 0,
        class: x::WindowClass::InputOnly,
        visual: x::COPY_FROM_PARENT,
        value_list: &[],
    });
    conn.send_request(&x::ConvertSelection {
        requestor: window,
        selection: clipboard,
        target: targets,
        property,
        time: x::CURRENT_TIME,
    });
    conn.flush().ok()?;

    let deadline = Instant::now() + Duration::from_millis(300);
    let mut got = false;
    while Instant::now() < deadline {
        match conn.poll_for_event() {
            Ok(Some(xcb::Event::X(x::Event::SelectionNotify(ev)))) => {
                got = ev.property() == property;
                break;
            }
            Ok(Some(_)) => {}
            Ok(None) => std::thread::sleep(Duration::from_millis(5)),
            Err(_) => break,
        }
    }
    let names = if got {
        let cookie = conn.send_request(&x::GetProperty {
            delete: true,
            window,
            property,
            r#type: x::ATOM_ATOM,
            long_offset: 0,
            long_length: 1024,
        });
        let atoms: Vec<x::Atom> = conn
            .wait_for_reply(cookie)
            .ok()
            .map(|r| r.value::<x::Atom>().to_vec())
            .unwrap_or_default();
        let cookies: Vec<_> = atoms
            .iter()
            .map(|&a| conn.send_request(&x::GetAtomName { atom: a }))
            .collect();
        cookies
            .into_iter()
            .filter_map(|c| conn.wait_for_reply(c).ok())
            .map(|r| r.name().to_string())
            .collect()
    } else {
        Vec::new()
    };
    conn.send_request(&x::DestroyWindow { window });
    let _ = conn.flush();
    Some(names)
}

/// Whether a compositor runs, which transparent windows need (without one they are
/// black). Always true on Wayland, where the desktop itself composites.
pub fn has_compositor() -> bool {
    if super::is_wayland() {
        return true;
    }
    let Some((conn, _)) = connect() else {
        return false;
    };
    let screen = std::env::var("DISPLAY")
        .ok()
        .and_then(|d| d.rsplit('.').next().and_then(|s| s.parse::<u32>().ok()))
        .unwrap_or(0);
    let Some(selection) = atom(&conn, &format!("_NET_WM_CM_S{screen}")) else {
        return false;
    };
    conn.wait_for_reply(conn.send_request(&x::GetSelectionOwner { selection }))
        .is_ok_and(|r| !r.owner().is_none())
}
