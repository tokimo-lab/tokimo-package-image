//! libvips FFI — 14 functions, no generated bindings.
//!
//! Unix (Linux / macOS): direct linkage against system libvips.
//! Windows: runtime loading via `windows` crate (no extra deps).
//!
//! All GLib `gsize` / `guint64` types are defined as `u64` directly, avoiding
//! the LLP64 mismatch that makes `c_ulong` 4 bytes on Windows (the root cause
//! of 51 u32/u64 compile errors in the `libvips` crate).
#![allow(unsafe_code)]

use std::ffi::{CStr, c_char, c_int, c_void};

type Gsize = u64;

// ── FFI trace (set TOKIMO_VIPS_TRACE=1 to print every libvips call) ─────────
fn trace_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("TOKIMO_VIPS_TRACE").is_ok_and(|v| v == "1"))
}
macro_rules! vtrace {
    ($($arg:tt)*) => {
        if trace_enabled() {
            let line = format!($($arg)*);
            eprintln!(
                "[VIPS_TRACE tid={:?} t={:?}] {}",
                std::thread::current().id(),
                std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0),
                line
            );
            use std::io::Write;
            let _ = std::io::stderr().flush();
        }
    };
}

#[repr(C)]
struct VipsImage {
    _private: [u8; 0],
}

const VIPS_SIZE_DOWN: c_int = 2;

// ── OutputFormat ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, Default)]
pub enum OutputFormat {
    #[default]
    Webp,
    Jpeg,
    Png,
}

impl OutputFormat {
    pub fn mime_type(&self) -> &'static str {
        match self {
            Self::Webp => "image/webp",
            Self::Jpeg => "image/jpeg",
            Self::Png => "image/png",
        }
    }

    pub fn ext(&self) -> &'static str {
        match self {
            Self::Webp => "webp",
            Self::Jpeg => "jpg",
            Self::Png => "png",
        }
    }

    pub fn from_ext(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "jpeg" | "jpg" => Self::Jpeg,
            "png" => Self::Png,
            _ => Self::Webp,
        }
    }
}

// ── Unix: direct linkage ─────────────────────────────────────────────────────

#[cfg(not(windows))]
mod sys {
    use super::*;

    #[link(name = "vips")]
    #[link(name = "gobject-2.0")]
    unsafe extern "C" {
        pub fn vips_init(argv0: *const c_char) -> c_int;
        #[allow(dead_code)]
        pub fn vips_version_string() -> *const c_char;
        pub fn vips_concurrency_set(concurrency: c_int);
        pub fn vips_error_buffer() -> *const c_char;
        pub fn vips_error_clear();
        pub fn vips_cache_set_max(max: c_int);
        pub fn vips_cache_set_max_mem(max_mem: Gsize);
        pub fn vips_cache_set_max_files(max_files: c_int);
        pub fn vips_thumbnail(filename: *const c_char, out: *mut *mut VipsImage, width: c_int, ...) -> c_int;
        pub fn vips_thumbnail_buffer(
            buf: *const c_void,
            len: Gsize,
            out: *mut *mut VipsImage,
            width: c_int,
            ...
        ) -> c_int;
        pub fn vips_image_write_to_file(image: *mut VipsImage, name: *const c_char, ...) -> c_int;
        pub fn g_object_unref(object: *mut c_void);
    }

    pub unsafe fn load() -> bool {
        true
    }
}

#[cfg(not(windows))]
use sys::*;

// ── Windows: runtime loading ─────────────────────────────────────────────────

#[cfg(windows)]
mod sys {
    use super::*;
    use std::ffi::OsStr;
    use std::mem;
    use std::os::windows::ffi::OsStrExt;
    use std::path::PathBuf;
    use std::sync::OnceLock;
    use windows::Win32::Foundation::HMODULE;
    use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
    use windows::core::{PCSTR, PCWSTR, s};

    /// Locate the bundled `bin/tokimo-lib/current/bin` directory that contains
    /// `libvips-42.dll` and friends. Mirrors the lookup in build.rs.
    fn libvips_dll_dir() -> Option<PathBuf> {
        if let Ok(p) = std::env::var("TOKIMO_DEP_LIBVIPS_DIR") {
            let pb = PathBuf::from(p).join("bin");
            if pb.is_dir() {
                return Some(pb);
            }
        }
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let mut dir = manifest_dir.as_path();
        loop {
            let cand = dir.join("bin").join("tokimo-lib").join("current").join("bin");
            if cand.is_dir() {
                return Some(cand);
            }
            dir = dir.parent()?;
        }
    }

    // --- function pointer storage ---

    static F_VIPS_INIT: OnceLock<unsafe extern "C" fn(*const c_char) -> c_int> = OnceLock::new();
    static F_VIPS_VERSION_STRING: OnceLock<unsafe extern "C" fn() -> *const c_char> = OnceLock::new();
    static F_VIPS_CONCURRENCY_SET: OnceLock<unsafe extern "C" fn(c_int)> = OnceLock::new();
    static F_VIPS_ERROR_BUFFER: OnceLock<unsafe extern "C" fn() -> *const c_char> = OnceLock::new();
    static F_VIPS_ERROR_CLEAR: OnceLock<unsafe extern "C" fn()> = OnceLock::new();
    static F_VIPS_CACHE_SET_MAX: OnceLock<unsafe extern "C" fn(c_int)> = OnceLock::new();
    static F_VIPS_CACHE_SET_MAX_MEM: OnceLock<unsafe extern "C" fn(Gsize)> = OnceLock::new();
    static F_VIPS_CACHE_SET_MAX_FILES: OnceLock<unsafe extern "C" fn(c_int)> = OnceLock::new();
    static F_VIPS_THUMBNAIL: OnceLock<
        unsafe extern "C" fn(
            *const c_char,
            *mut *mut VipsImage,
            c_int,
            *const c_char,
            c_int,
            *const c_char,
            c_int,
            *const c_void,
        ) -> c_int,
    > = OnceLock::new();
    static F_VIPS_THUMBNAIL_SIMPLE: OnceLock<
        unsafe extern "C" fn(*const c_char, *mut *mut VipsImage, c_int, *const c_void) -> c_int,
    > = OnceLock::new();
    static F_VIPS_THUMBNAIL_BUF: OnceLock<
        unsafe extern "C" fn(
            *const c_void,
            Gsize,
            *mut *mut VipsImage,
            c_int,
            *const c_char,
            c_int,
            *const c_char,
            c_int,
            *const c_void,
        ) -> c_int,
    > = OnceLock::new();
    static F_VIPS_THUMBNAIL_BUF_SIMPLE: OnceLock<
        unsafe extern "C" fn(*const c_void, Gsize, *mut *mut VipsImage, c_int, *const c_void) -> c_int,
    > = OnceLock::new();
    static F_VIPS_IMAGE_WRITE: OnceLock<unsafe extern "C" fn(*mut VipsImage, *const c_char, *const c_void) -> c_int> =
        OnceLock::new();
    static F_G_OBJECT_UNREF: OnceLock<unsafe extern "C" fn(*mut c_void)> = OnceLock::new();

    unsafe fn find_and_load(file_name: &str) -> Option<HMODULE> {
        // 1. Prefer absolute path inside bin/tokimo-lib/current/bin (deps.toml-managed,
        //    avoids accidentally loading an unrelated libvips from system PATH).
        if let Some(dir) = libvips_dll_dir() {
            let abs = dir.join(file_name);
            if abs.is_file() {
                let wide: Vec<u16> = abs.as_os_str().encode_wide().chain([0]).collect();
                if let Ok(h) = unsafe { LoadLibraryW(PCWSTR::from_raw(wide.as_ptr())) } {
                    return Some(h);
                }
            }
        }
        // 2. Try by name (searches PATH + standard Windows DLL dirs).
        let wide: Vec<u16> = OsStr::new(file_name).encode_wide().chain([0]).collect();
        if let Ok(h) = unsafe { LoadLibraryW(PCWSTR::from_raw(wide.as_ptr())) } {
            return Some(h);
        }
        // 3. Try next to the executable (Tauri / production-bundle layout).
        if let Ok(exe) = std::env::current_exe()
            && let Some(exe_dir) = exe.parent()
        {
            let abs = exe_dir.join(file_name);
            let full: Vec<u16> = abs.as_os_str().encode_wide().chain([0]).collect();
            if let Ok(h) = unsafe { LoadLibraryW(PCWSTR::from_raw(full.as_ptr())) } {
                return Some(h);
            }
        }
        None
    }

    unsafe fn load_fn<T>(hmod: HMODULE, slot: &OnceLock<T>, name: PCSTR) -> bool {
        match unsafe { GetProcAddress(hmod, name) } {
            Some(f) => {
                let ptr: *const () = f as *const ();
                let _ = slot.set(unsafe { mem::transmute_copy::<*const (), T>(&ptr) });
                true
            }
            None => false,
        }
    }

    pub unsafe fn load() -> bool {
        unsafe {
            let (Some(vips), Some(gobj)) = (find_and_load("libvips-42.dll"), find_and_load("libgobject-2.0-0.dll"))
            else {
                return false;
            };

            if !load_fn(vips, &F_VIPS_INIT, s!("vips_init")) {
                return false;
            }
            if !load_fn(vips, &F_VIPS_VERSION_STRING, s!("vips_version_string")) {
                return false;
            }
            if !load_fn(vips, &F_VIPS_CONCURRENCY_SET, s!("vips_concurrency_set")) {
                return false;
            }
            if !load_fn(vips, &F_VIPS_ERROR_BUFFER, s!("vips_error_buffer")) {
                return false;
            }
            if !load_fn(vips, &F_VIPS_ERROR_CLEAR, s!("vips_error_clear")) {
                return false;
            }
            if !load_fn(vips, &F_VIPS_CACHE_SET_MAX, s!("vips_cache_set_max")) {
                return false;
            }
            if !load_fn(vips, &F_VIPS_CACHE_SET_MAX_MEM, s!("vips_cache_set_max_mem")) {
                return false;
            }
            if !load_fn(vips, &F_VIPS_CACHE_SET_MAX_FILES, s!("vips_cache_set_max_files")) {
                return false;
            }
            if !load_fn(vips, &F_VIPS_THUMBNAIL, s!("vips_thumbnail")) {
                return false;
            }
            if !load_fn(vips, &F_VIPS_THUMBNAIL_SIMPLE, s!("vips_thumbnail")) {
                return false;
            }
            if !load_fn(vips, &F_VIPS_THUMBNAIL_BUF, s!("vips_thumbnail_buffer")) {
                return false;
            }
            if !load_fn(vips, &F_VIPS_THUMBNAIL_BUF_SIMPLE, s!("vips_thumbnail_buffer")) {
                return false;
            }
            if !load_fn(vips, &F_VIPS_IMAGE_WRITE, s!("vips_image_write_to_file")) {
                return false;
            }
            if !load_fn(gobj, &F_G_OBJECT_UNREF, s!("g_object_unref")) {
                return false;
            }
            true
        }
    }

    // --- wrappers ---

    pub unsafe fn vips_init(a: *const c_char) -> c_int {
        unsafe { F_VIPS_INIT.get().unwrap()(a) }
    }
    pub unsafe fn vips_concurrency_set(a: c_int) {
        unsafe { F_VIPS_CONCURRENCY_SET.get().unwrap()(a) }
    }
    pub unsafe fn vips_error_buffer() -> *const c_char {
        unsafe { F_VIPS_ERROR_BUFFER.get().unwrap()() }
    }
    pub unsafe fn vips_error_clear() {
        unsafe { F_VIPS_ERROR_CLEAR.get().unwrap()() }
    }
    pub unsafe fn vips_cache_set_max(a: c_int) {
        unsafe { F_VIPS_CACHE_SET_MAX.get().unwrap()(a) }
    }
    pub unsafe fn vips_cache_set_max_mem(a: Gsize) {
        unsafe { F_VIPS_CACHE_SET_MAX_MEM.get().unwrap()(a) }
    }
    pub unsafe fn vips_cache_set_max_files(a: c_int) {
        unsafe { F_VIPS_CACHE_SET_MAX_FILES.get().unwrap()(a) }
    }
    pub unsafe fn g_object_unref(a: *mut c_void) {
        unsafe { F_G_OBJECT_UNREF.get().unwrap()(a) }
    }

    // Fixed-arg wrappers (variadic → non-variadic)
    pub unsafe fn vips_thumbnail_simple(f: *const c_char, out: *mut *mut VipsImage, w: c_int) -> c_int {
        unsafe { F_VIPS_THUMBNAIL_SIMPLE.get().unwrap()(f, out, w, std::ptr::null()) }
    }
    pub unsafe fn vips_thumbnail_opts(f: *const c_char, out: *mut *mut VipsImage, w: c_int, h: c_int) -> c_int {
        unsafe {
            F_VIPS_THUMBNAIL.get().unwrap()(
                f,
                out,
                w,
                c"height".as_ptr().cast::<c_char>(),
                h,
                c"size".as_ptr().cast::<c_char>(),
                VIPS_SIZE_DOWN,
                std::ptr::null(),
            )
        }
    }
    pub unsafe fn vips_thumbnail_buffer_simple(
        buf: *const c_void,
        len: Gsize,
        out: *mut *mut VipsImage,
        w: c_int,
    ) -> c_int {
        unsafe { F_VIPS_THUMBNAIL_BUF_SIMPLE.get().unwrap()(buf, len, out, w, std::ptr::null()) }
    }
    pub unsafe fn vips_thumbnail_buffer_opts(
        buf: *const c_void,
        len: Gsize,
        out: *mut *mut VipsImage,
        w: c_int,
        h: c_int,
    ) -> c_int {
        unsafe {
            F_VIPS_THUMBNAIL_BUF.get().unwrap()(
                buf,
                len,
                out,
                w,
                c"height".as_ptr().cast::<c_char>(),
                h,
                c"size".as_ptr().cast::<c_char>(),
                VIPS_SIZE_DOWN,
                std::ptr::null(),
            )
        }
    }
    pub unsafe fn vips_image_write_to_file_simple(img: *mut VipsImage, name: *const c_char) -> c_int {
        unsafe { F_VIPS_IMAGE_WRITE.get().unwrap()(img, name, std::ptr::null()) }
    }
}

#[cfg(windows)]
use sys::*;

// ── Initialization ─────────────────────────────────────────────────────────

use std::sync::OnceLock;
// Cached result of one-shot init. Use `OnceLock::get_or_init` (not check-then-set)
// so concurrent callers serialize on the same closure. The earlier
// `match get(); ... set()` pattern allowed two tokio blocking threads to both
// call `vips_init()` concurrently, corrupting GLib's GType registry —
// manifesting as `GLib-GObject-WARNING **: cannot retrieve class for invalid
// (unclassed) type '<invalid>'` followed by SIGSEGV ~3 s later.
static VIPS_LOADED: OnceLock<bool> = OnceLock::new();

fn ensure_init() -> Result<(), String> {
    let ok = *VIPS_LOADED.get_or_init(|| {
        vtrace!("ensure_init: BEGIN sys::load");
        if !unsafe { sys::load() } {
            return false;
        }
        vtrace!("ensure_init: sys::load OK; vips_init BEGIN");

        if unsafe { vips_init(c"tokimo".as_ptr().cast::<c_char>()) } != 0 {
            return false;
        }
        vtrace!("ensure_init: vips_init OK");

        unsafe {
            vips_concurrency_set(1);
            vips_cache_set_max(0);
            vips_cache_set_max_mem(0);
            vips_cache_set_max_files(0);
        }
        true
    });

    if ok {
        Ok(())
    } else {
        Err("libvips not available on this system".into())
    }
}

fn vips_error_detail() -> String {
    unsafe {
        let ptr = vips_error_buffer();
        if ptr.is_null() {
            return "unknown error".into();
        }
        let detail = CStr::from_ptr(ptr)
            .to_str()
            .unwrap_or("<invalid utf8>")
            .trim()
            .to_owned();
        vips_error_clear();
        detail
    }
}

// ── Public API ──────────────────────────────────────────────────────────────

/// Generate a thumbnail from in-memory image bytes.
pub fn thumbnail_to_format(buffer: &[u8], width: u32, height: u32, format: OutputFormat) -> Result<Vec<u8>, String> {
    ensure_init()?;

    let w = if width == 0 { 32767 } else { width as c_int };
    let h = if height == 0 { 32767 } else { height as c_int };
    let mut out: *mut VipsImage = std::ptr::null_mut();

    let rc = if height == 0 {
        vtrace!("vips_thumbnail_buffer BEGIN len={} w={} h=auto", buffer.len(), w);
        let r;
        #[cfg(not(windows))]
        {
            r = unsafe {
                vips_thumbnail_buffer(
                    buffer.as_ptr().cast::<c_void>(),
                    buffer.len() as Gsize,
                    &raw mut out,
                    w,
                    std::ptr::null::<c_void>(),
                )
            };
        }
        #[cfg(windows)]
        {
            r = unsafe {
                vips_thumbnail_buffer_simple(buffer.as_ptr().cast::<c_void>(), buffer.len() as Gsize, &raw mut out, w)
            };
        }
        vtrace!("vips_thumbnail_buffer END rc={}", r);
        r
    } else {
        vtrace!("vips_thumbnail_buffer BEGIN len={} w={} h={}", buffer.len(), w, h);
        let r;
        #[cfg(not(windows))]
        {
            r = unsafe {
                vips_thumbnail_buffer(
                    buffer.as_ptr().cast::<c_void>(),
                    buffer.len() as Gsize,
                    &raw mut out,
                    w,
                    c"height".as_ptr().cast::<c_char>(),
                    h,
                    c"size".as_ptr().cast::<c_char>(),
                    VIPS_SIZE_DOWN,
                    std::ptr::null::<c_void>(),
                )
            };
        }
        #[cfg(windows)]
        {
            r = unsafe {
                vips_thumbnail_buffer_opts(
                    buffer.as_ptr().cast::<c_void>(),
                    buffer.len() as Gsize,
                    &raw mut out,
                    w,
                    h,
                )
            };
        }
        vtrace!("vips_thumbnail_buffer END rc={}", r);
        r
    };

    if rc != 0 || out.is_null() {
        return Err(format!("vips_thumbnail_buffer: {}", vips_error_detail()));
    }
    encode_format(out, format)
}

/// Generate a thumbnail from a file path.
pub fn thumbnail_file_to_format(path: &str, width: u32, height: u32, format: OutputFormat) -> Result<Vec<u8>, String> {
    ensure_init()?;

    let c_path = std::ffi::CString::new(path).map_err(|e| format!("invalid path: {e}"))?;
    let w = if width == 0 { 32767 } else { width as c_int };
    let h = if height == 0 { 32767 } else { height as c_int };
    let mut out: *mut VipsImage = std::ptr::null_mut();

    let rc = if height == 0 {
        vtrace!("vips_thumbnail BEGIN path={:?} w={} h=auto", path, w);
        let r;
        #[cfg(not(windows))]
        {
            r = unsafe { vips_thumbnail(c_path.as_ptr(), &raw mut out, w, std::ptr::null::<c_void>()) };
        }
        #[cfg(windows)]
        {
            r = unsafe { vips_thumbnail_simple(c_path.as_ptr(), &raw mut out, w) };
        }
        vtrace!("vips_thumbnail END rc={}", r);
        r
    } else {
        vtrace!("vips_thumbnail BEGIN path={:?} w={} h={}", path, w, h);
        let r;
        #[cfg(not(windows))]
        {
            r = unsafe {
                vips_thumbnail(
                    c_path.as_ptr(),
                    &raw mut out,
                    w,
                    c"height".as_ptr().cast::<c_char>(),
                    h,
                    c"size".as_ptr().cast::<c_char>(),
                    VIPS_SIZE_DOWN,
                    std::ptr::null::<c_void>(),
                )
            };
        }
        #[cfg(windows)]
        {
            r = unsafe { vips_thumbnail_opts(c_path.as_ptr(), &raw mut out, w, h) };
        }
        vtrace!("vips_thumbnail END rc={}", r);
        r
    };

    if rc != 0 || out.is_null() {
        return Err(format!("vips_thumbnail: {}", vips_error_detail()));
    }
    encode_format(out, format)
}

// ── Encoding ────────────────────────────────────────────────────────────────

fn encode_format(thumb: *mut VipsImage, format: OutputFormat) -> Result<Vec<u8>, String> {
    let tmp_dir = std::env::temp_dir();
    let tmp_path = tmp_dir.join(format!(
        "tokimo_enc_{}_{}.{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos()),
        format.ext()
    ));
    let tmp_str = tmp_path.to_str().ok_or("non-UTF8 temp path")?;

    let suffixed = match format {
        OutputFormat::Webp => format!("{tmp_str}[Q=80,effort=4]"),
        OutputFormat::Jpeg => format!("{tmp_str}[Q=82]"),
        OutputFormat::Png => tmp_str.to_string(),
    };
    let c_suffixed = std::ffi::CString::new(suffixed.as_str()).map_err(|e| format!("invalid path: {e}"))?;

    let rc = {
        vtrace!("vips_image_write_to_file BEGIN suffixed={}", suffixed);
        let r;
        #[cfg(not(windows))]
        {
            r = unsafe { vips_image_write_to_file(thumb, c_suffixed.as_ptr(), std::ptr::null::<c_void>()) };
        }
        #[cfg(windows)]
        {
            r = unsafe { vips_image_write_to_file_simple(thumb, c_suffixed.as_ptr()) };
        }
        vtrace!("vips_image_write_to_file END rc={}", r);
        r
    };

    vtrace!("g_object_unref(thumb={:p}) BEGIN", thumb);
    unsafe { g_object_unref(thumb.cast::<c_void>()) };
    vtrace!("g_object_unref END");

    if rc != 0 {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(format!(
            "vips_image_write_to_file({format:?}) failed: {}",
            vips_error_detail()
        ));
    }

    let bytes = std::fs::read(&tmp_path).map_err(|e| format!("read encode tmp: {e}"))?;
    let _ = std::fs::remove_file(&tmp_path);
    Ok(bytes)
}

// ── Backward-compatible aliases ─────────────────────────────────────────────

#[inline]
pub fn thumbnail_to_webp(buffer: &[u8], width: u32) -> Result<Vec<u8>, String> {
    thumbnail_to_format(buffer, width, 0, OutputFormat::Webp)
}

#[inline]
pub fn thumbnail_file_to_webp(path: &str, width: u32) -> Result<Vec<u8>, String> {
    thumbnail_file_to_format(path, width, 0, OutputFormat::Webp)
}
