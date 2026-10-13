//! An [`Editor`] implementation for egui.

use crate::egui::Vec2;
use crate::egui::ViewportCommand;
use crate::EguiState;
use baseview::gl::GlConfig;
use baseview::PhySize;
use baseview::{Size, WindowHandle, WindowOpenOptions, WindowScalePolicy};
use crossbeam::atomic::AtomicCell;
use egui_baseview::egui::Context;
use egui_baseview::EguiWindow;
use nih_plug::prelude::{Editor, GuiContext, ParamSetter, ParentWindowHandle};
use parking_lot::RwLock;
use raw_window_handle::{HasRawWindowHandle, RawWindowHandle};
use std::sync::atomic::Ordering;
use std::sync::Arc;


#[cfg(target_os = "windows")]
use std::sync::atomic::{AtomicPtr, Ordering as AtomicOrdering};
#[cfg(target_os = "windows")]
pub static PLUGIN_HWND: AtomicPtr<std::ffi::c_void> = AtomicPtr::new(std::ptr::null_mut());

#[cfg(target_os = "windows")]
pub mod win_keyboard {
    //! Windows keyboard input routing for VST plugins.
    //!
    //! Studio One, REAPER, Ableton Live etc. swallow `WM_KEYDOWN`/`WM_CHAR` destined for
    //! plugin child windows via `TranslateAccelerator` and `IsDialogMessage`. The fix used
    //! by `plugin-things` (ilmai) and adapted here:
    //!
    //! 1. A separate child window (`message window`) is registered with its own class.
    //!    Because it lives outside the host's dialog tree, neither
    //!    `TranslateAccelerator` nor `IsDialogMessage` consume messages for it.
    //! 2. When egui needs keyboard focus, `set_keyboard_focus(true)` redirects Win32
    //!    focus to the message window. Otherwise focus stays on the baseview window
    //!    (which forwards naturally to its parent's accelerator loop).
    //! 3. The message window's `WndProc` intercepts `WM_KEYDOWN`/`WM_KEYUP`/`WM_CHAR`
    //!    (and SYS variants) and re-posts them to the baseview HWND with `WM_APP+N` IDs.
    //!    Custom messages in the `WM_APP` range are never filtered by host accelerator
    //!    handling.
    //! 4. The subclassed `WndProc` on the baseview HWND translates `WM_APP+N` back to the
    //!    original `WM_KEY*`/`WM_CHAR` IDs before forwarding to baseview's original
    //!    `WndProc`, so baseview's keyboard handling sees the events as if they came in
    //!    normally.

    use std::ffi::c_void;
    use std::ptr::{null, null_mut};
    use std::sync::atomic::{AtomicPtr, AtomicU16, AtomicUsize, Ordering};
    use std::sync::OnceLock;

    type WndProcFn = unsafe extern "system" fn(*mut c_void, u32, usize, isize) -> isize;

    /// Temporary diagnostic log for the Windows keyboard workaround.
    /// Writes to %TEMP%\flash_drum_kbd.log. Remove once keyboard input is stable.
    /// Keyboard-interception trace, **debug builds only**.
    ///
    /// Kept rather than deleted because swallowing plugin key events is a
    /// per-host problem: Studio One, REAPER, Ableton and Cubase each filter
    /// differently, so this trace is what makes the next host debuggable. But it
    /// must not run in a shipped build: it used to open, append to and close a
    /// file in `%TEMP%` **on the UI thread** at every keyboard-focus change, and
    /// the file grew without bound (471 KB after a single testing session).
    ///
    /// The `format!` at the call sites still runs in release — an ordinary
    /// UI-thread allocation on a focus change, which is not worth turning nine
    /// call sites into a macro for.
    /// Diagnostic trace of the keyboard workaround.
    ///
    /// Always on in a debug build; in release it is opt-in through
    /// `FLASH_DRUM_KBD_LOG=1`, because the interception this module fights is a
    /// HOST behaviour and only reproduces inside a real DAW, where a debug
    /// build is not what anyone is running.
    fn kbd_log(msg: &str) {
        static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        let enabled = *ENABLED.get_or_init(|| {
            cfg!(debug_assertions) || std::env::var_os("FLASH_DRUM_KBD_LOG").is_some()
        });
        if !enabled {
            let _ = msg;
            return;
        }
        use std::fs::OpenOptions;
        use std::io::Write;
        let path = std::env::temp_dir().join("flash_drum_kbd.log");
        let _ = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .and_then(|mut f| writeln!(f, "{}", msg));
    }

    #[repr(C)]
    struct WNDCLASSW {
        style: u32,
        lpfn_wnd_proc: Option<WndProcFn>,
        cb_cls_extra: i32,
        cb_wnd_extra: i32,
        h_instance: *mut c_void,
        h_icon: *mut c_void,
        h_cursor: *mut c_void,
        hbr_background: *mut c_void,
        lpsz_menu_name: *const u16,
        lpsz_class_name: *const u16,
    }

    #[repr(C)]
    struct MSG {
        hwnd: *mut c_void,
        message: u32,
        wparam: usize,
        lparam: isize,
        time: u32,
        pt_x: i32,
        pt_y: i32,
        private: u32,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct POINT {
        x: i32,
        y: i32,
    }

    const PM_NOREMOVE: u32 = 0x0000;
    const SCAN_MASK: isize = 0x01FF_0000;

    #[link(name = "user32")]
    extern "system" {
        fn SetWindowLongPtrW(hwnd: *mut c_void, n_index: i32, new_long: isize) -> isize;
        fn CallWindowProcW(
            prev: WndProcFn,
            hwnd: *mut c_void,
            msg: u32,
            wparam: usize,
            lparam: isize,
        ) -> isize;
        fn SetPropW(hwnd: *mut c_void, lp_string: *const u16, h_data: *mut c_void) -> i32;
        fn GetPropW(hwnd: *mut c_void, lp_string: *const u16) -> *mut c_void;
        fn DefWindowProcW(hwnd: *mut c_void, msg: u32, wparam: usize, lparam: isize) -> isize;
        fn RegisterClassW(wc: *const WNDCLASSW) -> u16;
        fn CreateWindowExW(
            ex_style: u32,
            class_name: *const u16,
            window_name: *const u16,
            style: u32,
            x: i32,
            y: i32,
            width: i32,
            height: i32,
            parent: *mut c_void,
            menu: *mut c_void,
            instance: *mut c_void,
            param: *mut c_void,
        ) -> *mut c_void;
        fn PostMessageW(hwnd: *mut c_void, msg: u32, wparam: usize, lparam: isize) -> i32;
        fn PeekMessageW(
            msg: *mut MSG,
            hwnd: *mut c_void,
            msg_filter_min: u32,
            msg_filter_max: u32,
            remove_msg: u32,
        ) -> i32;
        fn GetParent(hwnd: *mut c_void) -> *mut c_void;
        fn IsWindow(hwnd: *mut c_void) -> i32;
        fn SetFocus(hwnd: *mut c_void) -> *mut c_void;
        fn GetFocus() -> *mut c_void;
        fn GetForegroundWindow() -> *mut c_void;
        #[allow(dead_code)]
        fn GetCursorPos(point: *mut POINT) -> i32;
        #[allow(dead_code)]
        fn WindowFromPoint(point: POINT) -> *mut c_void;
        fn GetWindowThreadProcessId(hwnd: *mut c_void, process_id: *mut u32) -> u32;
        fn GetCurrentProcessId() -> u32;
        fn GetAncestor(hwnd: *mut c_void, flags: u32) -> *mut c_void;
        fn GetClassNameW(hwnd: *mut c_void, class_name: *mut u16, max_count: i32) -> i32;
        fn GetCurrentThreadId() -> u32;
        fn AttachThreadInput(id_attach: u32, id_attach_to: u32, f_attach: i32) -> i32;
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GetModuleHandleW(lp_module_name: *const u16) -> *mut c_void;
        fn GetModuleHandleExW(
            flags: u32,
            module_name: *const u16,
            module: *mut *mut c_void,
        ) -> i32;
        fn DestroyWindow(hwnd: *mut c_void) -> i32;
        fn RemovePropW(hwnd: *mut c_void, lp_string: *const u16) -> *mut c_void;
        fn UnregisterClassW(class_name: *const u16, h_instance: *mut c_void) -> i32;
    }

    const GWLP_WNDPROC: i32 = -4;
    const WM_GETDLGCODE: u32 = 0x0087;
    const DLGC_WANTARROWS: isize = 0x0001;
    /// What a standard `Edit` control answers: hosts use it to recognise a
    /// text field and route typing (the space bar included) to it instead of
    /// running their own shortcuts.
    const DLGC_HASSETSEL: isize = 0x0008;
    const DLGC_WANTTAB: isize = 0x0002;
    const DLGC_WANTALLKEYS: isize = 0x0004;
    const DLGC_WANTCHARS: isize = 0x0080;

    const WM_KEYDOWN: u32 = 0x0100;
    const WM_KEYUP: u32 = 0x0101;
    const WM_CHAR: u32 = 0x0102;
    const WM_SYSKEYDOWN: u32 = 0x0104;
    const WM_SYSKEYUP: u32 = 0x0105;
    const WM_SYSCHAR: u32 = 0x0106;

    const WM_APP: u32 = 0x8000;
    const WM_APP_KEY_DOWN: u32 = WM_APP + 1;
    const WM_APP_KEY_UP: u32 = WM_APP + 2;
    const WM_APP_CHAR: u32 = WM_APP + 3;
    const WM_APP_SYSKEY_DOWN: u32 = WM_APP + 4;
    const WM_APP_SYSKEY_UP: u32 = WM_APP + 5;
    const WM_APP_SYSCHAR: u32 = WM_APP + 6;

    const WS_CHILD: u32 = 0x40000000;

    // UTF-16 null-terminated "NihPlugEguiOrigWndProc".
    const PROP_NAME: &[u16] = &[
        0x004E, 0x0069, 0x0068, 0x0050, 0x006C, 0x0075, 0x0067, 0x0045, 0x0067, 0x0075, 0x0069,
        0x004F, 0x0072, 0x0069, 0x0067, 0x0057, 0x006E, 0x0064, 0x0050, 0x0072, 0x006F, 0x0063,
        0x0000,
    ];

    const GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT: u32 = 0x0000_0002;
    const GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS: u32 = 0x0000_0004;

    /// Live editors that installed the bridge. The class may only be unregistered
    /// when the last one goes away, or a second instance's message window would be
    /// left pointing at a dead class.
    static INSTALL_COUNT: AtomicUsize = AtomicUsize::new(0);

    /// **This DLL's** module handle — NOT the host executable's.
    ///
    /// The window class must be owned by the module whose code its `WndProc` lives
    /// in. Registering it under the host EXE's `HINSTANCE` (which is what this code
    /// used to do) made the registration outlive our DLL, leaving a class whose
    /// `WndProc` pointed into unmapped memory.
    fn our_hinstance() -> *mut c_void {
        static CACHED: AtomicPtr<c_void> = AtomicPtr::new(null_mut());
        let cached = CACHED.load(Ordering::Acquire);
        if !cached.is_null() {
            return cached;
        }
        unsafe {
            let mut module: *mut c_void = null_mut();
            let ok = GetModuleHandleExW(
                GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS
                    | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
                msg_wnd_proc as *const () as *const u16,
                &mut module,
            );
            if ok != 0 && !module.is_null() {
                CACHED.store(module, Ordering::Release);
                module
            } else {
                // Should not happen; the host EXE is still better than nothing.
                GetModuleHandleW(null())
            }
        }
    }

    /// Class name for THIS DLL load: a fixed prefix plus our module base.
    ///
    /// The name used to be the constant `"NihPlugEguiKbdMsg"`, which is what made
    /// the host crash. Sequence: load the DLL (class registered) -> close the
    /// session (DLL unloaded, class survives with a dangling `WndProc`) -> load the
    /// DLL again at a different base. `RegisterClassW` then failed because the name
    /// was taken, the old code treated that as success, and `CreateWindowExW` built
    /// a window on the STALE class. The first message dispatched jumped into
    /// unmapped memory: `0xC0000005` with an execute-violation flag, sometimes
    /// landing inside whatever unrelated DLL had since been loaded there.
    ///
    /// A per-load name makes that reuse impossible by construction, even if a crash
    /// prevented `uninstall` from running.
    fn class_name() -> *const u16 {
        static NAME: OnceLock<Vec<u16>> = OnceLock::new();
        NAME.get_or_init(|| {
            let base = our_hinstance() as usize;
            format!("NihPlugEguiKbdMsg_{base:X}")
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect()
        })
        .as_ptr()
    }

    /// Message window's HWND, accessed by `set_keyboard_focus`. Single global since the
    /// plugin is expected to host at most one editor window at a time per process.
    static MESSAGE_HWND: AtomicPtr<c_void> = AtomicPtr::new(null_mut());

    /// Atom returned by `RegisterClassW`. Registered once per DLL load.
    static MSG_CLASS_ATOM: AtomicU16 = AtomicU16::new(0);

    /// WndProc installed on the message window. Forwards keyboard messages back to the
    /// parent (baseview) HWND via custom `WM_APP+N` IDs that hosts don't filter.
    /// Returns true if a `WM_CHAR`/`WM_SYSCHAR` with a matching scan code is queued at
    /// `hwnd`. Mirrors baseview's `is_last_message` so we drop the `WM_KEYDOWN` when a
    /// `WM_CHAR` is coming next: otherwise baseview would produce two events (one from
    /// each), leading to duplicated text.
    unsafe fn has_pending_char(hwnd: *mut c_void, msg: u32, lparam: isize) -> bool {
        let expected = match msg {
            WM_KEYDOWN | WM_CHAR => WM_CHAR,
            WM_SYSKEYDOWN | WM_SYSCHAR => WM_SYSCHAR,
            _ => return false,
        };
        let mut next: MSG = std::mem::zeroed();
        let avail = PeekMessageW(&mut next, hwnd, expected, expected, PM_NOREMOVE);
        avail != 0 && (next.lparam & SCAN_MASK) == (lparam & SCAN_MASK)
    }

    unsafe extern "system" fn msg_wnd_proc(
        hwnd: *mut c_void,
        msg: u32,
        wparam: usize,
        lparam: isize,
    ) -> isize {
        let log_keyboard = msg == WM_KEYDOWN
            || msg == WM_KEYUP
            || msg == WM_CHAR
            || msg == WM_SYSKEYDOWN
            || msg == WM_SYSKEYUP
            || msg == WM_SYSCHAR;
        if log_keyboard {
            kbd_log(&format!(
                "msg_wnd_proc: hwnd={:p} msg={:04x} wparam={} lparam={:016x}",
                hwnd, msg, wparam, lparam
            ));
        }
        // Claim every key and character for THIS window too, not only for
        // baseview's HWND (see `subclass_proc`).
        //
        // REAPER hosts the editor in a `#32770` dialog and runs `IsDialogMessage`
        // on its message loop. That routine asks the FOCUSED window — which is
        // this one while typing — whether it wants the characters; with no
        // answer it treats WM_CHAR as dialog mnemonics and swallows them. The
        // trace showed exactly that: the key-down peek saw a WM_CHAR queued,
        // dropped the key-down so the char would carry the event, and the char
        // never arrived. Two letters got through before the dialog started
        // filtering, then nothing.
        if msg == WM_GETDLGCODE {
            // `DLGC_HASSETSEL` is what a real `Edit` control answers. A host
            // that decides between "text field: deliver the key" and "run my
            // shortcut" by asking the focused window this way then treats us
            // as a text field — the space bar types a space instead of
            // starting the transport.
            return DLGC_WANTALLKEYS | DLGC_WANTCHARS | DLGC_WANTARROWS | DLGC_WANTTAB | DLGC_HASSETSEL;
        }
        // Drop the keydown if a matching char is queued: baseview's keyboard logic
        // merges them, but only if it can see the WM_CHAR via PeekMessageW. Since we
        // forward as WM_APP+N, baseview's peek won't find it and would emit two events.
        // Dropping the keydown here makes the single WM_CHAR carry the final event.
        if (msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN) && has_pending_char(hwnd, msg, lparam) {
            return 0;
        }

        let forward_as = match msg {
            WM_KEYDOWN => Some(WM_APP_KEY_DOWN),
            WM_KEYUP => Some(WM_APP_KEY_UP),
            WM_CHAR => Some(WM_APP_CHAR),
            WM_SYSKEYDOWN => Some(WM_APP_SYSKEY_DOWN),
            WM_SYSKEYUP => Some(WM_APP_SYSKEY_UP),
            WM_SYSCHAR => Some(WM_APP_SYSCHAR),
            _ => None,
        };
        if let Some(app_msg) = forward_as {
            let parent = GetParent(hwnd);
            if !parent.is_null() {
                PostMessageW(parent, app_msg, wparam, lparam);
            }
            return 0;
        }
        DefWindowProcW(hwnd, msg, wparam, lparam)
    }

    /// WndProc subclass installed on the baseview HWND. Two jobs:
    /// - On `WM_GETDLGCODE`, claim all keyboard input so that hosts using
    ///   `IsDialogMessage` (some older / less common code paths) don't filter keys.
    /// - On `WM_APP+N` messages posted by `msg_wnd_proc`, translate them back to their
    ///   real `WM_KEY*`/`WM_CHAR` IDs and dispatch to baseview's original WndProc.
    unsafe extern "system" fn subclass_proc(
        hwnd: *mut c_void,
        msg: u32,
        wparam: usize,
        lparam: isize,
    ) -> isize {
        if msg == WM_GETDLGCODE {
            return DLGC_WANTALLKEYS | DLGC_WANTCHARS | DLGC_WANTARROWS | DLGC_WANTTAB;
        }
        let translated_msg = match msg {
            WM_APP_KEY_DOWN => WM_KEYDOWN,
            WM_APP_KEY_UP => WM_KEYUP,
            WM_APP_CHAR => WM_CHAR,
            WM_APP_SYSKEY_DOWN => WM_SYSKEYDOWN,
            WM_APP_SYSKEY_UP => WM_SYSKEYUP,
            WM_APP_SYSCHAR => WM_SYSCHAR,
            other => other,
        };
        if translated_msg != msg {
            kbd_log(&format!(
                "subclass_proc: hwnd={:p} app_msg={:04x} -> real_msg={:04x} wparam={} lparam={:016x}",
                hwnd, msg, translated_msg, wparam, lparam
            ));
        }
        let original = GetPropW(hwnd, PROP_NAME.as_ptr());
        if original.is_null() {
            return DefWindowProcW(hwnd, translated_msg, wparam, lparam);
        }
        let original_fn: WndProcFn = std::mem::transmute(original);
        CallWindowProcW(original_fn, hwnd, translated_msg, wparam, lparam)
    }

    fn install_subclass(hwnd: *mut c_void) {
        if hwnd.is_null() {
            return;
        }
        unsafe {
            if !GetPropW(hwnd, PROP_NAME.as_ptr()).is_null() {
                return;
            }
            let original =
                SetWindowLongPtrW(hwnd, GWLP_WNDPROC, subclass_proc as *const () as isize);
            if original != 0 {
                SetPropW(hwnd, PROP_NAME.as_ptr(), original as *mut c_void);
            }
        }
    }

    fn ensure_msg_class() -> u16 {
        let existing = MSG_CLASS_ATOM.load(Ordering::Acquire);
        if existing != 0 {
            return existing;
        }
        unsafe {
            let h_inst = our_hinstance();
            let wc = WNDCLASSW {
                style: 0,
                lpfn_wnd_proc: Some(msg_wnd_proc),
                cb_cls_extra: 0,
                cb_wnd_extra: 0,
                h_instance: h_inst,
                h_icon: null_mut(),
                h_cursor: null_mut(),
                hbr_background: null_mut(),
                lpsz_menu_name: null(),
                lpsz_class_name: class_name(),
            };
            let atom = RegisterClassW(&wc);
            // The name is unique per DLL load, so a 0 here means a genuine failure
            // rather than a leftover registration from a previous load. Reusing such
            // a leftover is exactly what crashed the host (see `class_name`).
            let marker = if atom == 0 { 1 } else { atom };
            MSG_CLASS_ATOM.store(marker, Ordering::Release);
            marker
        }
    }

    fn create_message_window(parent: *mut c_void) -> *mut c_void {
        let _ = ensure_msg_class();
        unsafe {
            let h_inst = our_hinstance();
            CreateWindowExW(
                0, // WS_EX_NOACTIVATE removed: child window needs to be able to receive focus
                class_name(),
                null(),
                WS_CHILD,
                0,
                0,
                1,
                1,
                parent,
                null_mut(),
                h_inst,
                null_mut(),
            )
        }
    }

    /// Install the subclass on the plugin HWND and create the auxiliary message window.
    /// Called once per editor `spawn`.
    pub fn install(plugin_hwnd: *mut c_void) -> *mut c_void {
        if plugin_hwnd.is_null() {
            return null_mut();
        }
        install_subclass(plugin_hwnd);
        let msg_hwnd = create_message_window(plugin_hwnd);
        MESSAGE_HWND.store(msg_hwnd, Ordering::Release);
        INSTALL_COUNT.fetch_add(1, Ordering::AcqRel);
        msg_hwnd
    }

    /// Undo everything [`install`] did, in the order Windows requires.
    ///
    /// Must run while the windows still exist, i.e. **before** the editor's
    /// baseview window is closed. Skipping this is what left the process holding
    /// function pointers into a DLL that was about to be unloaded:
    ///
    /// 1. the host's own window kept OUR `subclass_proc` as its `WndProc`;
    /// 2. the message window outlived its class;
    /// 3. the class outlived the DLL.
    pub fn uninstall(plugin_hwnd: *mut c_void, msg_hwnd: *mut c_void) {
        unsafe {
            // 1. Give the host window its original WndProc back.
            if !plugin_hwnd.is_null() && IsWindow(plugin_hwnd) != 0 {
                let original = GetPropW(plugin_hwnd, PROP_NAME.as_ptr());
                if !original.is_null() {
                    SetWindowLongPtrW(plugin_hwnd, GWLP_WNDPROC, original as isize);
                    RemovePropW(plugin_hwnd, PROP_NAME.as_ptr());
                }
            }

            // 2. Destroy our message window. UnregisterClassW refuses to run while
            //    a window of the class is alive, so this has to come first.
            if !msg_hwnd.is_null() && IsWindow(msg_hwnd) != 0 {
                DestroyWindow(msg_hwnd);
            }
            if MESSAGE_HWND.load(Ordering::Acquire) == msg_hwnd {
                MESSAGE_HWND.store(null_mut(), Ordering::Release);
            }

            // 3. The last editor out unregisters the class. Several instances of the
            //    plugin share one DLL load, so unregistering on the first close
            //    would strand the others.
            if INSTALL_COUNT.load(Ordering::Acquire) > 0
                && INSTALL_COUNT.fetch_sub(1, Ordering::AcqRel) == 1
            {
                UnregisterClassW(class_name(), our_hinstance());
                MSG_CLASS_ATOM.store(0, Ordering::Release);
            }
        }
    }

    #[allow(dead_code)] // remplace par le test de fenetre active, garde comme trace
    unsafe fn is_window_or_descendant(root: *mut c_void, child: *mut c_void) -> bool {
        if root.is_null() || child.is_null() {
            return false;
        }

        let mut current = child;
        while !current.is_null() {
            if current == root {
                return true;
            }
            current = GetParent(current);
        }
        false
    }

    #[allow(dead_code)] // remplace par le test de fenetre active, garde comme trace
    unsafe fn cursor_over_window_or_descendant(root: *mut c_void) -> bool {
        let mut point = POINT { x: 0, y: 0 };
        if GetCursorPos(&mut point) == 0 {
            return false;
        }
        is_window_or_descendant(root, WindowFromPoint(point))
    }

    /// Window class name, for the diagnostic log only.
    fn hwnd_class(hwnd: *mut c_void) -> String {
        if hwnd.is_null() {
            return String::from("null");
        }
        let mut buf = [0u16; 128];
        let n = unsafe { GetClassNameW(hwnd, buf.as_mut_ptr(), buf.len() as i32) };
        if n <= 0 {
            return String::from("?");
        }
        String::from_utf16_lossy(&buf[..n as usize])
    }

    /// A key the host offered through `IPlugView::onKeyDown/Up`, delivered to
    /// baseview over the same `WM_APP` channel the message window uses, so it
    /// goes through `subclass_proc` and reaches egui like any other key.
    ///
    /// A printable character becomes ONE `WM_CHAR` (no key-down alongside it:
    /// baseview would otherwise emit the text twice, the peek-based merge only
    /// sees real `WM_CHAR`s). A named key (backspace, arrows, enter, delete…)
    /// becomes a `WM_KEYDOWN` / `WM_KEYUP` with the Windows virtual key that
    /// matches the VST3 code. Returns whether anything was delivered — that is
    /// what the view reports to the host as "handled".
    pub fn host_key(key: u16, code: i16, _modifiers: i16, down: bool) -> bool {
        let plugin = super::PLUGIN_HWND.load(Ordering::Acquire);
        if plugin.is_null() {
            return false;
        }
        // VST3 `VirtualKeyCodes` (keycodes.h) -> Windows virtual keys. Only the
        // keys a text field needs; anything else falls back to the host.
        let vk: Option<usize> = match code {
            1 => Some(0x08),  // KEY_BACK      -> VK_BACK
            2 => Some(0x09),  // KEY_TAB       -> VK_TAB
            4 | 19 => Some(0x0D), // KEY_RETURN / KEY_ENTER -> VK_RETURN
            6 => Some(0x1B),  // KEY_ESCAPE    -> VK_ESCAPE
            7 => Some(0x20),  // KEY_SPACE     -> VK_SPACE
            9 => Some(0x23),  // KEY_END       -> VK_END
            10 => Some(0x24), // KEY_HOME      -> VK_HOME
            11 => Some(0x25), // KEY_LEFT      -> VK_LEFT
            12 => Some(0x26), // KEY_UP        -> VK_UP
            13 => Some(0x27), // KEY_RIGHT     -> VK_RIGHT
            14 => Some(0x28), // KEY_DOWN      -> VK_DOWN
            15 => Some(0x21), // KEY_PAGEUP    -> VK_PRIOR
            16 => Some(0x22), // KEY_PAGEDOWN  -> VK_NEXT
            21 => Some(0x2D), // KEY_INSERT    -> VK_INSERT
            22 => Some(0x2E), // KEY_DELETE    -> VK_DELETE
            _ => None,
        };
        // Repeat count 1, scan code 0: baseview keys off the message id and
        // the virtual key, not the scan code.
        const LPARAM_KEY: isize = 1;
        unsafe {
            if down {
                // The space bar comes with BOTH a code and a character; the
                // character is what the field needs.
                let printable = key >= 0x20 && key != 0x7F;
                if printable {
                    PostMessageW(plugin, WM_APP_CHAR, key as usize, LPARAM_KEY);
                    kbd_log(&format!("host_key: char {key} -> WM_CHAR"));
                    return true;
                }
                if let Some(vk) = vk {
                    PostMessageW(plugin, WM_APP_KEY_DOWN, vk, LPARAM_KEY);
                    kbd_log(&format!("host_key: code {code} -> WM_KEYDOWN vk={vk:#x}"));
                    return true;
                }
                false
            } else {
                match vk {
                    Some(vk) => {
                        PostMessageW(plugin, WM_APP_KEY_UP, vk, LPARAM_KEY | (1 << 30) | (1 << 31));
                        true
                    }
                    // A character's release has nothing to deliver; still
                    // "handled", it belongs to a key we took.
                    None => key >= 0x20,
                }
            }
        }
    }

    /// Move keyboard focus between the message window (when egui wants input) and the
    /// baseview window (when it doesn't). Never refocuses the plugin unless focus is
    /// already inside the editor, otherwise hosts like Studio One cannot open menus while
    /// the editor is visible. Uses `AttachThreadInput` so `SetFocus` works even when the
    /// calling thread doesn't own the host's input queue.
    pub fn set_keyboard_focus(focused: bool) {
        // Published for the VST3 view: while true it answers the host's
        // on_key_down with "handled", which is what stops REAPER from running
        // its space-bar Play action over a character being typed.
        nih_plug::editor::EDITOR_WANTS_KEYBOARD.store(focused, Ordering::Relaxed);
        let plugin = super::PLUGIN_HWND.load(Ordering::Acquire);
        let msg = MESSAGE_HWND.load(Ordering::Acquire);
        if plugin.is_null() {
            return;
        }
        unsafe {
            if !msg.is_null() && IsWindow(msg) == 0 {
                MESSAGE_HWND.store(null_mut(), Ordering::Release);
                return;
            }
            let target = if focused && !msg.is_null() { msg } else { plugin };
            let current_focus = GetFocus();
            if current_focus == target {
                return;
            }

            let fg = GetForegroundWindow();
            if fg.is_null() {
                kbd_log("  abort: no foreground window");
                return;
            }

            if focused {
                kbd_log(&format!(
                    "set_keyboard_focus(true): plugin={:p} msg={:p} current_focus={:p} target={:p}",
                    plugin, msg, current_focus, target
                ));
                // The editor may take the keyboard only while its OWN top-level
                // window is the active one.
                //
                // This replaces a focus-or-cursor test that could not work here:
                // `GetFocus()` reports the calling thread's queue and returns
                // null from the audio-plugin thread, so the whole decision fell
                // on "is the mouse over the plugin" — which fails the moment you
                // type without hovering it. Measured in REAPER: **2091 aborts
                // out of 2105 attempts**, all on that test.
                //
                // `GA_ROOT` is what makes it work in both layouts: the plugin's
                // root is the host's main window when the editor is docked
                // (Studio One) and the floating FX window when it is not
                // (REAPER). Either way, the guard's intent holds — we never
                // steal the keyboard from another window of the host, nor from
                // another application.
                const GA_ROOT: u32 = 2;
                const GA_ROOTOWNER: u32 = 3;
                let root = GetAncestor(plugin, GA_ROOT);
                let root_owner = GetAncestor(plugin, GA_ROOTOWNER);
                let fg_root_owner = GetAncestor(fg, GA_ROOTOWNER);
                // A floating FX window may be OWNED by the host's main window
                // rather than parented to it, and some hosts keep the main
                // window as the foreground one while you type in the floating
                // editor. Accept any of the ownership relations between the
                // plugin's window and the active one; the process test above
                // already rules out other applications.
                let related = root == fg
                    || root_owner == fg
                    || (!fg_root_owner.is_null()
                        && (fg_root_owner == root || fg_root_owner == root_owner));
                kbd_log(&format!(
                    "  windows: root={:p} [{}] root_owner={:p} fg={:p} [{}] fg_root_owner={:p} related={}",
                    root,
                    hwnd_class(root),
                    root_owner,
                    fg,
                    hwnd_class(fg),
                    fg_root_owner,
                    related
                ));
                if !related {
                    kbd_log("  abort: the plugin's window is unrelated to the active one");
                    return;
                }
            } else if !msg.is_null() && current_focus != msg {
                return;
            }
            // Only set focus while the HOST is the foreground application. If
            // the user has switched to another one (browser, explorer...), do
            // NOT steal focus back.
            //
            // The test is on the owning PROCESS, not on the window chain. The
            // chain walk this replaces climbed `GetParent` from the plugin
            // looking for the foreground window, which holds in a docked editor
            // but never in a FLOATING plugin window: REAPER's FX window is a
            // top-level window of its own, outside the plugin's parent chain,
            // so the walk ran to null and aborted on every single frame. The
            // log showed exactly that — "plugin not in foreground chain",
            // hundreds of times, with no focus ever set, which is why typing
            // did nothing in REAPER.
            let mut fg_pid: u32 = 0;
            GetWindowThreadProcessId(fg, &mut fg_pid);
            if fg_pid != 0 && fg_pid != GetCurrentProcessId() {
                kbd_log("  abort: foreground window belongs to another process");
                return;
            }
            let fg_thread = GetWindowThreadProcessId(fg, null_mut());
            let my_thread = GetCurrentThreadId();
            if fg_thread != 0 && fg_thread != my_thread {
                AttachThreadInput(my_thread, fg_thread, 1);
                SetFocus(target);
                AttachThreadInput(my_thread, fg_thread, 0);
                kbd_log("  SetFocus(target) via AttachThreadInput");
                return;
            }
            SetFocus(target);
            kbd_log("  SetFocus(target) direct");

        }
    }
}

/// An [`Editor`] implementation that calls an egui draw loop.
pub(crate) struct EguiEditor<T> {
    pub(crate) egui_state: Arc<EguiState>,
    /// The plugin's state. This is kept in between editor openenings.
    pub(crate) user_state: Arc<RwLock<T>>,

    /// The user's build function. Applied once at the start of the application.
    pub(crate) build: Arc<dyn Fn(&Context, &mut T) + 'static + Send + Sync>,
    /// The user's update function.
    pub(crate) update: Arc<dyn Fn(&Context, &ParamSetter, &mut T) + 'static + Send + Sync>,

    /// The scaling factor reported by the host, if any. On macOS this will never be set and we
    /// should use the system scaling factor instead.
    pub(crate) scaling_factor: AtomicCell<Option<f32>>,
}

/// This version of `baseview` uses a different version of `raw_window_handle than NIH-plug, so we
/// need to adapt it ourselves.
struct ParentWindowHandleAdapter(nih_plug::editor::ParentWindowHandle);

unsafe impl HasRawWindowHandle for ParentWindowHandleAdapter {
    fn raw_window_handle(&self) -> RawWindowHandle {
        match self.0 {
            ParentWindowHandle::X11Window(window) => {
                let mut handle = raw_window_handle::XcbWindowHandle::empty();
                handle.window = window;
                RawWindowHandle::Xcb(handle)
            }
            ParentWindowHandle::AppKitNsView(ns_view) => {
                let mut handle = raw_window_handle::AppKitWindowHandle::empty();
                handle.ns_view = ns_view;
                RawWindowHandle::AppKit(handle)
            }
            ParentWindowHandle::Win32Hwnd(hwnd) => {
                let mut handle = raw_window_handle::Win32WindowHandle::empty();
                handle.hwnd = hwnd;
                RawWindowHandle::Win32(handle)
            }
        }
    }
}

impl<T> Editor for EguiEditor<T>
where
    T: 'static + Send + Sync,
{
    fn spawn(
        &self,
        parent: ParentWindowHandle,
        context: Arc<dyn GuiContext>,
    ) -> Box<dyn std::any::Any + Send> {
        let build = self.build.clone();
        let update = self.update.clone();
        let state = self.user_state.clone();
        let egui_state = self.egui_state.clone();

        let (unscaled_width, unscaled_height) = self.egui_state.size();
        let scaling_factor = self.scaling_factor.load();
        let window = EguiWindow::open_parented(
            &ParentWindowHandleAdapter(parent),
            WindowOpenOptions {
                title: String::from("egui window"),
                // Baseview should be doing the DPI scaling for us
                size: Size::new(unscaled_width as f64, unscaled_height as f64),
                // NOTE: For some reason passing 1.0 here causes the UI to be scaled on macOS but
                //       not the mouse events.
                scale: scaling_factor
                    .map(|factor| WindowScalePolicy::ScaleFactor(factor as f64))
                    .unwrap_or(WindowScalePolicy::SystemScaleFactor),

                #[cfg(feature = "opengl")]
                gl_config: Some(GlConfig {
                    version: (3, 2),
                    red_bits: 8,
                    blue_bits: 8,
                    green_bits: 8,
                    alpha_bits: 8,
                    depth_bits: 24,
                    stencil_bits: 8,
                    samples: None,
                    srgb: true,
                    double_buffer: true,
                    vsync: true,
                    ..Default::default()
                }),
            },
            Default::default(),
            state,
            move |egui_ctx, _queue, state| build(egui_ctx, &mut state.write()),
            move |egui_ctx, queue, state| {
                let setter = ParamSetter::new(context.as_ref());

                // If the window was requested to resize
                if let Some(new_size) = egui_state.requested_size.swap(None) {
                    // FLASH-DRUM PATCH [282]: store the new size BEFORE asking the host.
                    // On the GUI thread the VST3 wrapper runs the request immediately and
                    // sends the host `size()`, which still returned the old size: the host
                    // kept its window while the content grew.
                    let old_size = egui_state.size.swap(new_size);
                    // Ask the plugin host to resize to self.size()
                    if context.request_resize() {
                        // Resize the content of egui window
                        queue.resize(PhySize::new(new_size.0, new_size.1));
                        egui_ctx.send_viewport_cmd(ViewportCommand::InnerSize(Vec2::new(
                            new_size.0 as f32,
                            new_size.1 as f32,
                        )));
                    } else {
                        egui_state.size.store(old_size);
                    }
                }

                // For now, just always redraw. Most plugin GUIs have meters, and those almost always
                // need a redraw. Later we can try to be a bit more sophisticated about this. Without
                // this we would also have a blank GUI when it gets first opened because most DAWs open
                // their GUI while the window is still unmapped.
                egui_ctx.request_repaint();
                (update)(egui_ctx, &setter, &mut state.write());
            },
        );

        #[cfg(target_os = "windows")]
        let mut plugin_hwnd = std::ptr::null_mut();
        #[cfg(target_os = "windows")]
        let mut msg_hwnd = std::ptr::null_mut();
        #[cfg(target_os = "windows")]
        {
            use raw_window_handle::HasRawWindowHandle;
            if let RawWindowHandle::Win32(handle) = window.raw_window_handle() {
                PLUGIN_HWND.store(handle.hwnd, AtomicOrdering::Release);
                plugin_hwnd = handle.hwnd;
                msg_hwnd = win_keyboard::install(handle.hwnd);
                // Once per process: the VST3 view hands host-offered keys here.
                let _ = nih_plug::editor::HOST_KEY_HANDLER.set(win_keyboard::host_key);
            }
        }

        self.egui_state.open.store(true, Ordering::Release);
        Box::new(EguiEditorHandle {
            egui_state: self.egui_state.clone(),
            window,
            #[cfg(target_os = "windows")]
            plugin_hwnd,
            #[cfg(target_os = "windows")]
            msg_hwnd,
        })
    }

    /// Size of the editor window
    fn size(&self) -> (u32, u32) {
        let new_size = self.egui_state.requested_size.load();
        // This method will be used to ask the host for new size.
        // If the editor is currently being resized and new size hasn't been consumed and set yet, return new requested size.
        if let Some(new_size) = new_size {
            new_size
        } else {
            self.egui_state.size()
        }
    }

    fn set_scale_factor(&self, factor: f32) -> bool {
        // If the editor is currently open then the host must not change the current HiDPI scale as
        // we don't have a way to handle that. Ableton Live does this.
        if self.egui_state.is_open() {
            return false;
        }

        self.scaling_factor.store(Some(factor));
        true
    }

    fn param_value_changed(&self, _id: &str, _normalized_value: f32) {
        // As mentioned above, for now we'll always force a redraw to allow meter widgets to work
        // correctly. In the future we can use an `Arc<AtomicBool>` and only force a redraw when
        // that boolean is set.
    }

    fn param_modulation_changed(&self, _id: &str, _modulation_offset: f32) {}

    fn param_values_changed(&self) {
        // Same
    }
}

/// The window handle used for [`EguiEditor`].
struct EguiEditorHandle {
    egui_state: Arc<EguiState>,
    window: WindowHandle,
    /// Kept so `Drop` can undo the keyboard bridge before the window goes away.
    #[cfg(target_os = "windows")]
    plugin_hwnd: *mut std::ffi::c_void,
    #[cfg(target_os = "windows")]
    msg_hwnd: *mut std::ffi::c_void,
}

/// The window handle enum stored within 'WindowHandle' contains raw pointers. Is there a way around
/// having this requirement?
unsafe impl Send for EguiEditorHandle {}

impl Drop for EguiEditorHandle {
    fn drop(&mut self) {
        // Undo the keyboard bridge BEFORE the window is destroyed, so nothing the
        // host keeps points into this DLL once it is unloaded.
        #[cfg(target_os = "windows")]
        {
            win_keyboard::uninstall(self.plugin_hwnd, self.msg_hwnd);
            if PLUGIN_HWND.load(AtomicOrdering::Acquire) == self.plugin_hwnd {
                PLUGIN_HWND.store(std::ptr::null_mut(), AtomicOrdering::Release);
            }
        }
        self.egui_state.open.store(false, Ordering::Release);
        // XXX: This should automatically happen when the handle gets dropped, but apparently not
        self.window.close();
    }
}
