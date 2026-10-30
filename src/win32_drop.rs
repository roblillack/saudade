//! Windows: the main window's own OLE drop target, in place of winit's.
//!
//! winit 0.30 registers one that reads `CF_HDROP`, the dragged files' paths,
//! and nothing else. A link or an image dragged out of a web browser has no
//! path — it comes as an address and, for an image, as a "virtual file": a
//! name in `FileGroupDescriptorW` and the contents in `FileContents` — so it
//! never reached the app. Nor did winit say where the pointer was. This target
//! reads all three of a drag's forms (see [`DragData`]) along with the pointer
//! position, so the window gets [`Event::DragMove`]s on Windows too.
//!
//! OLE calls a drop target while the window's thread dispatches its messages,
//! outside of any winit callback, so there is no widget tree to hand an event
//! to there. The target queues them instead, and the runtime takes them up in
//! `about_to_wait`, which winit runs once the messages are dispatched. Whether
//! a widget accepted the drag (see [`EventCtx::accept_drop`]) is therefore
//! only known for the position before the current one. The cursor follows that
//! answer, and so does a drop: one released where no widget took the drag
//! ends as an [`Event::DragLeave`] rather than an [`Event::Drop`].
//!
//! [`Event::DragMove`]: crate::Event::DragMove
//! [`Event::DragLeave`]: crate::Event::DragLeave
//! [`Event::Drop`]: crate::Event::Drop
//! [`EventCtx::accept_drop`]: crate::EventCtx::accept_drop

use std::cell::{Cell, RefCell};
use std::ffi::{OsString, c_void};
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;
use std::ptr;
use std::rc::Rc;
use std::sync::atomic::{AtomicU32, Ordering};

use windows_sys::Win32::Foundation::{E_NOINTERFACE, E_POINTER, HWND, POINT, POINTL, S_OK};
use windows_sys::Win32::Graphics::Gdi::ScreenToClient;
use windows_sys::Win32::System::Com::{
    DVASPECT_CONTENT, FORMATETC, STGMEDIUM, TYMED, TYMED_HGLOBAL, TYMED_ISTREAM,
};
use windows_sys::Win32::System::DataExchange::RegisterClipboardFormatW;
use windows_sys::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
use windows_sys::Win32::System::Ole::{
    CF_HDROP, CF_UNICODETEXT, DROPEFFECT_COPY, DROPEFFECT_LINK, DROPEFFECT_NONE, OleInitialize,
    RegisterDragDrop, ReleaseStgMedium,
};
use windows_sys::Win32::UI::Shell::{
    CFSTR_FILECONTENTS, CFSTR_FILEDESCRIPTORW, CFSTR_INETURLW, DragQueryFileW, FD_FILESIZE,
    FILEDESCRIPTORW, HDROP,
};
use windows_sys::core::{GUID, HRESULT, PCWSTR};
use winit::dpi::PhysicalPosition;
use winit::window::Window;

use crate::event::{DragData, DraggedFile};

/// A drag as the drop target saw it, waiting for the runtime to turn it into
/// an [`Event`](crate::Event). Positions are physical pixels in the window's
/// client area, as winit reports the pointer's.
#[derive(Debug)]
pub(crate) enum Drag {
    Enter(PhysicalPosition<f64>),
    Move(PhysicalPosition<f64>),
    Leave,
    Drop(PhysicalPosition<f64>, DragData),
}

/// What the drop target and the runtime share.
#[derive(Default)]
pub(crate) struct Drags {
    queue: RefCell<Vec<Drag>>,
    /// Whether a widget accepted the drag at the last position the runtime
    /// dispatched; `None` until it has dispatched one.
    accepted: Cell<Option<bool>>,
}

impl Drags {
    /// The drags queued since the last call.
    pub(crate) fn take(&self) -> Vec<Drag> {
        std::mem::take(&mut *self.queue.borrow_mut())
    }

    /// The widget tree's answer to the drag at the last position.
    pub(crate) fn set_accepted(&self, accepted: Option<bool>) {
        self.accepted.set(accepted);
    }

    pub(crate) fn accepted(&self) -> Option<bool> {
        self.accepted.get()
    }

    fn push(&self, drag: Drag) {
        self.queue.borrow_mut().push(drag);
    }
}

/// Initialize OLE on this thread, which drag and drop needs. `false` if it
/// can't be — COM was initialized multithreaded already — and winit's own
/// target should be kept, as it is the one that handles that case.
pub(crate) fn init_ole() -> bool {
    // S_FALSE (1): it already was, as a single-threaded apartment.
    unsafe { OleInitialize(ptr::null()) >= 0 }
}

/// Make `win` take drags through a [`DropTarget`]. Its window must have been
/// created with winit's own target turned off, as a window takes one only.
/// winit revokes it as the window is destroyed.
pub(crate) fn register(win: &Window) -> Option<Rc<Drags>> {
    let hwnd = crate::win32_dpi::hwnd(win)?;
    let drags = Rc::new(Drags::default());
    let target = DropTarget::new(hwnd, drags.clone());
    let registered = unsafe { RegisterDragDrop(hwnd, target.cast()) } == S_OK;
    // OLE holds a reference of its own once registered; this one is ours.
    unsafe { DropTarget::release(target) };
    registered.then_some(drags)
}

// ---- the COM object --------------------------------------------------------

const IID_IUNKNOWN: GUID = GUID::from_u128(0x00000000_0000_0000_c000_000000000046);
const IID_IDROPTARGET: GUID = GUID::from_u128(0x00000122_0000_0000_c000_000000000046);

/// `IDropTarget`. `POINTL` is passed by value, as in the C declaration.
#[repr(C)]
struct DropTargetVtbl {
    query_interface:
        unsafe extern "system" fn(*mut DropTarget, *const GUID, *mut *mut c_void) -> HRESULT,
    add_ref: unsafe extern "system" fn(*mut DropTarget) -> u32,
    release: unsafe extern "system" fn(*mut DropTarget) -> u32,
    drag_enter:
        unsafe extern "system" fn(*mut DropTarget, *mut c_void, u32, POINTL, *mut u32) -> HRESULT,
    drag_over: unsafe extern "system" fn(*mut DropTarget, u32, POINTL, *mut u32) -> HRESULT,
    drag_leave: unsafe extern "system" fn(*mut DropTarget) -> HRESULT,
    drop: unsafe extern "system" fn(*mut DropTarget, *mut c_void, u32, POINTL, *mut u32) -> HRESULT,
}

static VTBL: DropTargetVtbl = DropTargetVtbl {
    query_interface: DropTarget::query_interface,
    add_ref: DropTarget::add_ref,
    release: DropTarget::release,
    drag_enter: DropTarget::drag_enter,
    drag_over: DropTarget::drag_over,
    drag_leave: DropTarget::drag_leave,
    drop: DropTarget::drop,
};

#[repr(C)]
struct DropTarget {
    vtbl: *const DropTargetVtbl,
    refs: AtomicU32,
    hwnd: HWND,
    drags: Rc<Drags>,
    /// The drag over the window carries something [`read`] can read.
    readable: Cell<bool>,
    /// Where it was last reported, to report only moves.
    last: Cell<(i32, i32)>,
}

impl DropTarget {
    fn new(hwnd: HWND, drags: Rc<Drags>) -> *mut DropTarget {
        Box::into_raw(Box::new(DropTarget {
            vtbl: &VTBL,
            refs: AtomicU32::new(1),
            hwnd,
            drags,
            readable: Cell::new(false),
            last: Cell::new((i32::MIN, i32::MIN)),
        }))
    }

    unsafe extern "system" fn query_interface(
        this: *mut DropTarget,
        iid: *const GUID,
        out: *mut *mut c_void,
    ) -> HRESULT {
        if out.is_null() || iid.is_null() {
            return E_POINTER;
        }
        let iid = unsafe { &*iid };
        if guid_eq(iid, &IID_IUNKNOWN) || guid_eq(iid, &IID_IDROPTARGET) {
            unsafe {
                Self::add_ref(this);
                *out = this.cast();
            }
            S_OK
        } else {
            unsafe { *out = ptr::null_mut() };
            E_NOINTERFACE
        }
    }

    unsafe extern "system" fn add_ref(this: *mut DropTarget) -> u32 {
        unsafe { (*this).refs.fetch_add(1, Ordering::Relaxed) + 1 }
    }

    unsafe extern "system" fn release(this: *mut DropTarget) -> u32 {
        let left = unsafe { (*this).refs.fetch_sub(1, Ordering::AcqRel) - 1 };
        if left == 0 {
            drop(unsafe { Box::from_raw(this) });
        }
        left
    }

    unsafe extern "system" fn drag_enter(
        this: *mut DropTarget,
        data: *mut c_void,
        _keys: u32,
        pt: POINTL,
        effect: *mut u32,
    ) -> HRESULT {
        let target = unsafe { &*this };
        let readable = !data.is_null() && readable(&DataObject(data));
        target.readable.set(readable);
        if readable {
            let pos = target.client(pt);
            target.last.set((pt.x, pt.y));
            target.drags.set_accepted(None);
            target.drags.push(Drag::Enter(pos));
        }
        unsafe { target.answer(effect) };
        S_OK
    }

    unsafe extern "system" fn drag_over(
        this: *mut DropTarget,
        _keys: u32,
        pt: POINTL,
        effect: *mut u32,
    ) -> HRESULT {
        let target = unsafe { &*this };
        // OLE asks again every so often while the pointer rests.
        if target.readable.get() && target.last.replace((pt.x, pt.y)) != (pt.x, pt.y) {
            target.drags.push(Drag::Move(target.client(pt)));
        }
        unsafe { target.answer(effect) };
        S_OK
    }

    unsafe extern "system" fn drag_leave(this: *mut DropTarget) -> HRESULT {
        let target = unsafe { &*this };
        if target.readable.replace(false) {
            target.drags.push(Drag::Leave);
        }
        S_OK
    }

    unsafe extern "system" fn drop(
        this: *mut DropTarget,
        data: *mut c_void,
        _keys: u32,
        pt: POINTL,
        effect: *mut u32,
    ) -> HRESULT {
        let target = unsafe { &*this };
        unsafe { target.answer(effect) };
        if target.readable.replace(false) {
            let payload = if data.is_null() {
                DragData::default()
            } else {
                read(&DataObject(data))
            };
            target.drags.push(Drag::Drop(target.client(pt), payload));
        }
        S_OK
    }

    /// Set `effect`, which comes in as what the source allows, to what this
    /// drop does: copy where it may, link where it may only link (some
    /// programs offer an address that way), nothing where the drag can't be
    /// read or no widget took it.
    unsafe fn answer(&self, effect: *mut u32) {
        if effect.is_null() {
            return;
        }
        let allowed = unsafe { *effect };
        let wanted = self.readable.get() && self.drags.accepted() != Some(false);
        let answer = if !wanted {
            DROPEFFECT_NONE
        } else if allowed & DROPEFFECT_COPY != 0 {
            DROPEFFECT_COPY
        } else if allowed & DROPEFFECT_LINK != 0 {
            DROPEFFECT_LINK
        } else {
            DROPEFFECT_NONE
        };
        unsafe { *effect = answer };
    }

    /// A screen position in the window's client area.
    fn client(&self, pt: POINTL) -> PhysicalPosition<f64> {
        let mut p = POINT { x: pt.x, y: pt.y };
        unsafe { ScreenToClient(self.hwnd, &mut p) };
        PhysicalPosition::new(p.x as f64, p.y as f64)
    }
}

fn guid_eq(a: &GUID, b: &GUID) -> bool {
    (a.data1, a.data2, a.data3, a.data4) == (b.data1, b.data2, b.data3, b.data4)
}

// ---- reading what is dragged -----------------------------------------------

/// The start of `IDataObject`'s vtable, as far as it is called here.
#[repr(C)]
struct DataObjectVtbl {
    query_interface: usize,
    add_ref: usize,
    release: usize,
    get_data: unsafe extern "system" fn(*mut c_void, *const FORMATETC, *mut STGMEDIUM) -> HRESULT,
    get_data_here: usize,
    query_get_data: unsafe extern "system" fn(*mut c_void, *const FORMATETC) -> HRESULT,
}

/// The start of `IStream`'s: `ISequentialStream::Read`.
#[repr(C)]
struct StreamVtbl {
    query_interface: usize,
    add_ref: usize,
    release: usize,
    read: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, *mut u32) -> HRESULT,
}

/// A borrowed `IDataObject`, valid for as long as the OLE call it came with.
pub(crate) struct DataObject(pub(crate) *mut c_void);

impl DataObject {
    fn vtbl(&self) -> &DataObjectVtbl {
        unsafe { &**(self.0 as *const *const DataObjectVtbl) }
    }

    fn offers(&self, format: u16, tymed: TYMED) -> bool {
        let fmt = format_etc(format, -1, tymed);
        unsafe { (self.vtbl().query_get_data)(self.0, &fmt) == S_OK }
    }

    fn get(&self, format: u16, index: i32, tymed: TYMED) -> Option<Medium> {
        let fmt = format_etc(format, index, tymed);
        let mut medium: STGMEDIUM = unsafe { std::mem::zeroed() };
        let hr = unsafe { (self.vtbl().get_data)(self.0, &fmt, &mut medium) };
        (hr == S_OK).then_some(Medium(medium))
    }

    /// A UTF-16 string, up to its terminating NUL.
    fn string(&self, format: u16) -> Option<String> {
        let bytes = self.get(format, -1, TYMED_HGLOBAL)?.bytes()?;
        let units: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .take_while(|&u| u != 0)
            .collect();
        Some(String::from_utf16_lossy(&units))
    }

    fn paths(&self) -> Vec<PathBuf> {
        let Some(medium) = self.get(CF_HDROP, -1, TYMED_HGLOBAL) else {
            return Vec::new();
        };
        let hdrop = unsafe { medium.0.u.hGlobal } as HDROP;
        let count = unsafe { DragQueryFileW(hdrop, u32::MAX, ptr::null_mut(), 0) };
        (0..count)
            .map(|i| {
                let len = unsafe { DragQueryFileW(hdrop, i, ptr::null_mut(), 0) } as usize;
                let mut buf = vec![0u16; len + 1];
                unsafe { DragQueryFileW(hdrop, i, buf.as_mut_ptr(), buf.len() as u32) };
                PathBuf::from(OsString::from_wide(&buf[..len]))
            })
            .collect()
    }

    /// Virtual files: names from the group descriptor, contents one by one.
    fn files(&self) -> Vec<DraggedFile> {
        let (Some(group), Some(contents)) = (
            clipboard_format(CFSTR_FILEDESCRIPTORW),
            clipboard_format(CFSTR_FILECONTENTS),
        ) else {
            return Vec::new();
        };
        let Some(bytes) = self.get(group, -1, TYMED_HGLOBAL).and_then(|m| m.bytes()) else {
            return Vec::new();
        };
        descriptors(&bytes)
            .into_iter()
            .enumerate()
            .filter_map(|(i, d)| {
                let medium = self.get(contents, i as i32, TYMED_HGLOBAL | TYMED_ISTREAM)?;
                let mut contents = medium.bytes()?;
                // A global block may be bigger than what was put in it.
                if d.dwFlags & FD_FILESIZE as u32 != 0 {
                    let size = (d.nFileSizeHigh as u64) << 32 | d.nFileSizeLow as u64;
                    contents.truncate(usize::try_from(size).unwrap_or(usize::MAX));
                }
                // Copied out: the descriptor is packed.
                let name = { d.cFileName };
                let len = name.iter().position(|&u| u == 0).unwrap_or(name.len());
                let name = String::from_utf16_lossy(&name[..len]);
                Some(DraggedFile { name, contents })
            })
            .collect()
    }

    fn url(&self) -> Option<String> {
        let url = self.string(clipboard_format(CFSTR_INETURLW)?)?;
        let url = url.trim();
        (!url.is_empty()).then(|| url.to_string())
    }

    fn text_url(&self) -> Option<String> {
        let text = self.string(CF_UNICODETEXT)?;
        text_as_url(&text).map(str::to_string)
    }
}

/// Dragged text as an address, when it is nothing but one: a single `http:`,
/// `https:` or `data:` URL, as a program that offers a link only as text drags
/// it. Trimmed.
fn text_as_url(text: &str) -> Option<&str> {
    let text = text.trim_matches(|c: char| c.is_whitespace() || c.is_control());
    let (scheme, _) = text.split_once(':')?;
    let scheme = scheme.to_ascii_lowercase();
    let web = matches!(scheme.as_str(), "http" | "https" | "data");
    (web && !text.contains(char::is_whitespace)).then_some(text)
}

/// Whether `data` carries anything [`read`] reads. Text is read here already,
/// to tell an address from any other text; it never takes long to render.
fn readable(data: &DataObject) -> bool {
    let offers =
        |name: PCWSTR| clipboard_format(name).is_some_and(|f| data.offers(f, TYMED_HGLOBAL));
    data.offers(CF_HDROP, TYMED_HGLOBAL)
        || offers(CFSTR_FILEDESCRIPTORW)
        || offers(CFSTR_INETURLW)
        || data.text_url().is_some()
}

/// Everything a drag carries. Virtual files are read only when there are no
/// paths, which are the same files then, and the cheaper to read.
pub(crate) fn read(data: &DataObject) -> DragData {
    let paths = data.paths();
    let files = if paths.is_empty() {
        data.files()
    } else {
        Vec::new()
    };
    let urls = data.url().or_else(|| data.text_url()).into_iter().collect();
    DragData { paths, files, urls }
}

/// The `FILEDESCRIPTORW`s of a `FILEGROUPDESCRIPTORW`: a count, then the
/// descriptors, packed.
fn descriptors(bytes: &[u8]) -> Vec<FILEDESCRIPTORW> {
    let size = std::mem::size_of::<FILEDESCRIPTORW>();
    let Some(count) = bytes
        .get(..4)
        .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
    else {
        return Vec::new();
    };
    bytes[4..]
        .chunks_exact(size)
        .take(count as usize)
        .map(|chunk| unsafe { ptr::read_unaligned(chunk.as_ptr().cast::<FILEDESCRIPTORW>()) })
        .collect()
}

/// A registered clipboard format's number.
fn clipboard_format(name: PCWSTR) -> Option<u16> {
    u16::try_from(unsafe { RegisterClipboardFormatW(name) })
        .ok()
        .filter(|&f| f != 0)
}

fn format_etc(format: u16, index: i32, tymed: TYMED) -> FORMATETC {
    FORMATETC {
        cfFormat: format,
        ptd: ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT as u32,
        lindex: index,
        tymed: tymed as u32,
    }
}

/// A `STGMEDIUM` the source handed over, released when dropped.
struct Medium(STGMEDIUM);

impl Medium {
    /// All of it, from a global memory block or a stream.
    fn bytes(&self) -> Option<Vec<u8>> {
        if self.0.tymed == TYMED_HGLOBAL as u32 {
            let global = unsafe { self.0.u.hGlobal };
            let size = unsafe { GlobalSize(global) };
            let ptr = unsafe { GlobalLock(global) };
            if ptr.is_null() {
                return None;
            }
            let bytes = unsafe { std::slice::from_raw_parts(ptr.cast::<u8>(), size) }.to_vec();
            unsafe { GlobalUnlock(global) };
            Some(bytes)
        } else if self.0.tymed == TYMED_ISTREAM as u32 {
            let stream = unsafe { self.0.u.pstm };
            if stream.is_null() {
                return None;
            }
            let vtbl = unsafe { &**(stream as *const *const StreamVtbl) };
            let mut out = Vec::new();
            let mut chunk = vec![0u8; 64 * 1024];
            loop {
                let mut read = 0u32;
                let hr = unsafe {
                    (vtbl.read)(
                        stream,
                        chunk.as_mut_ptr().cast(),
                        chunk.len() as u32,
                        &mut read,
                    )
                };
                if hr < 0 {
                    return None;
                }
                out.extend_from_slice(&chunk[..read as usize]);
                if read == 0 || hr != S_OK {
                    return Some(out);
                }
            }
        } else {
            None
        }
    }
}

impl Drop for Medium {
    fn drop(&mut self) {
        unsafe { ReleaseStgMedium(&mut self.0) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::Foundation::HGLOBAL;
    use windows_sys::Win32::System::Com::StructuredStorage::CreateStreamOnHGlobal;
    use windows_sys::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc};
    use windows_sys::Win32::UI::Shell::DROPFILES;

    #[link(name = "shell32")]
    unsafe extern "system" {
        /// The shell's own data object, which takes any format through
        /// `SetData`: what a drag out of another program hands over.
        fn SHCreateDataObject(
            folder: *const c_void,
            count: u32,
            items: *const *const c_void,
            inner: *mut c_void,
            iid: *const GUID,
            out: *mut *mut c_void,
        ) -> HRESULT;
    }

    const IID_IDATAOBJECT: GUID = GUID::from_u128(0x0000010e_0000_0000_c000_000000000046);

    /// `IDataObject` up to `SetData`.
    #[repr(C)]
    struct FullVtbl {
        query_interface: usize,
        add_ref: usize,
        release: unsafe extern "system" fn(*mut c_void) -> u32,
        get_data: usize,
        get_data_here: usize,
        query_get_data: usize,
        get_canonical_format_etc: usize,
        set_data: unsafe extern "system" fn(
            *mut c_void,
            *const FORMATETC,
            *mut STGMEDIUM,
            i32,
        ) -> HRESULT,
    }

    struct Fixture(*mut c_void);

    impl Fixture {
        fn new() -> Self {
            assert!(init_ole());
            let mut obj = ptr::null_mut();
            let hr = unsafe {
                SHCreateDataObject(
                    ptr::null(),
                    0,
                    ptr::null(),
                    ptr::null_mut(),
                    &IID_IDATAOBJECT,
                    &mut obj,
                )
            };
            assert_eq!(hr, S_OK);
            Fixture(obj)
        }

        fn vtbl(&self) -> &FullVtbl {
            unsafe { &**(self.0 as *const *const FullVtbl) }
        }

        /// Hand `medium` over, as a source does.
        fn set(&self, format: u16, index: i32, mut medium: STGMEDIUM) {
            let fmt = format_etc(format, index, medium.tymed as TYMED);
            let hr = unsafe { (self.vtbl().set_data)(self.0, &fmt, &mut medium, 1) };
            assert_eq!(hr, S_OK);
        }

        fn set_bytes(&self, format: u16, index: i32, bytes: &[u8]) {
            self.set(format, index, hglobal_medium(bytes));
        }

        fn set_string(&self, format: u16, s: &str) {
            let units: Vec<u16> = s.encode_utf16().chain([0]).collect();
            self.set_bytes(format, -1, &le_bytes(&units));
        }

        fn data(&self) -> DataObject {
            DataObject(self.0)
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            unsafe { (self.vtbl().release)(self.0) };
        }
    }

    fn le_bytes(units: &[u16]) -> Vec<u8> {
        units.iter().flat_map(|u| u.to_le_bytes()).collect()
    }

    fn raw_bytes<T>(value: &T) -> Vec<u8> {
        let ptr = (value as *const T).cast::<u8>();
        unsafe { std::slice::from_raw_parts(ptr, std::mem::size_of::<T>()) }.to_vec()
    }

    fn hglobal(bytes: &[u8]) -> HGLOBAL {
        unsafe {
            let global = GlobalAlloc(GMEM_MOVEABLE, bytes.len().max(1));
            let ptr = GlobalLock(global);
            ptr::copy_nonoverlapping(bytes.as_ptr(), ptr.cast(), bytes.len());
            GlobalUnlock(global);
            global
        }
    }

    fn hglobal_medium(bytes: &[u8]) -> STGMEDIUM {
        let mut medium: STGMEDIUM = unsafe { std::mem::zeroed() };
        medium.tymed = TYMED_HGLOBAL as u32;
        medium.u.hGlobal = hglobal(bytes);
        medium
    }

    fn stream_medium(bytes: &[u8]) -> STGMEDIUM {
        let mut medium: STGMEDIUM = unsafe { std::mem::zeroed() };
        medium.tymed = TYMED_ISTREAM as u32;
        let mut stream = ptr::null_mut();
        assert_eq!(
            unsafe { CreateStreamOnHGlobal(hglobal(bytes), 1, &mut stream) },
            S_OK
        );
        medium.u.pstm = stream;
        medium
    }

    /// A `FILEGROUPDESCRIPTORW` naming `files`, with their sizes where given.
    fn group(files: &[(&str, Option<u32>)]) -> Vec<u8> {
        let mut out = (files.len() as u32).to_le_bytes().to_vec();
        for (name, size) in files {
            let mut d: FILEDESCRIPTORW = unsafe { std::mem::zeroed() };
            let mut units = [0u16; 260];
            for (slot, u) in units.iter_mut().zip(name.encode_utf16()) {
                *slot = u;
            }
            d.cFileName = units;
            if let Some(size) = size {
                d.dwFlags = FD_FILESIZE as u32;
                d.nFileSizeLow = *size;
            }
            out.extend(raw_bytes(&d));
        }
        out
    }

    fn format(name: PCWSTR) -> u16 {
        clipboard_format(name).unwrap()
    }

    #[test]
    fn reads_an_image_dragged_out_of_a_browser() {
        // Its contents as a virtual file, in a block bigger than they are,
        // and its address.
        let f = Fixture::new();
        let group = group(&[("cat.png", Some(5))]);
        f.set_bytes(format(CFSTR_FILEDESCRIPTORW), -1, &group);
        f.set_bytes(format(CFSTR_FILECONTENTS), 0, b"hello, and more");
        f.set_string(format(CFSTR_INETURLW), "https://example.com/cat.png");
        assert!(readable(&f.data()));
        let data = read(&f.data());
        assert!(data.paths.is_empty());
        assert_eq!(
            data.files,
            [DraggedFile {
                name: "cat.png".into(),
                contents: b"hello".to_vec(),
            }]
        );
        assert_eq!(data.urls, ["https://example.com/cat.png"]);
    }

    #[test]
    fn reads_virtual_files_from_a_stream() {
        // As a mail program hands over an attachment.
        let f = Fixture::new();
        f.set_bytes(
            format(CFSTR_FILEDESCRIPTORW),
            -1,
            &group(&[("a.txt", None)]),
        );
        f.set(format(CFSTR_FILECONTENTS), 0, stream_medium(b"attached"));
        let data = read(&f.data());
        assert_eq!(data.files.len(), 1);
        assert_eq!(data.files[0].name, "a.txt");
        assert_eq!(data.files[0].contents, b"attached");
    }

    #[test]
    fn reads_paths() {
        let f = Fixture::new();
        let header = DROPFILES {
            pFiles: std::mem::size_of::<DROPFILES>() as u32,
            pt: POINT { x: 0, y: 0 },
            fNC: 0,
            fWide: 1,
        };
        let names: Vec<u16> = "C:\\a.png\0D:\\b c.jpg\0\0".encode_utf16().collect();
        let mut bytes = raw_bytes(&header);
        bytes.extend(le_bytes(&names));
        f.set_bytes(CF_HDROP, -1, &bytes);
        assert!(readable(&f.data()));
        let data = read(&f.data());
        assert_eq!(
            data.paths,
            [PathBuf::from("C:\\a.png"), PathBuf::from("D:\\b c.jpg")]
        );
        assert!(data.files.is_empty() && data.urls.is_empty());
    }

    #[test]
    fn takes_text_only_when_it_is_an_address() {
        let f = Fixture::new();
        f.set_string(CF_UNICODETEXT, "  https://example.com/a?b=c\r\n");
        assert!(readable(&f.data()));
        assert_eq!(read(&f.data()).urls, ["https://example.com/a?b=c"]);

        let f = Fixture::new();
        f.set_string(CF_UNICODETEXT, "see https://example.com");
        assert!(!readable(&f.data()));
        assert!(read(&f.data()).is_empty());
    }

    #[test]
    fn queues_drags_where_they_are_and_answers_for_the_widgets() {
        let f = Fixture::new();
        f.set_string(format(CFSTR_INETURLW), "https://example.com/");
        let drags = Rc::new(Drags::default());
        // Without a window, positions stay as they are.
        let target = DropTarget::new(ptr::null_mut(), drags.clone());
        let vtbl = unsafe { &*(*target).vtbl };
        let at = |x, y| POINTL { x, y };
        let mut effect = DROPEFFECT_COPY | DROPEFFECT_LINK;
        unsafe { (vtbl.drag_enter)(target, f.0, 0, at(10, 20), &mut effect) };
        assert_eq!(effect, DROPEFFECT_COPY);

        // The widgets turned it down, and the cursor says so.
        drags.set_accepted(Some(false));
        let mut effect = DROPEFFECT_COPY;
        unsafe { (vtbl.drag_over)(target, 0, at(10, 20), &mut effect) };
        assert_eq!(effect, DROPEFFECT_NONE);
        // Taken again, from a source that only links.
        drags.set_accepted(Some(true));
        let mut effect = DROPEFFECT_LINK;
        unsafe { (vtbl.drag_over)(target, 0, at(11, 20), &mut effect) };
        assert_eq!(effect, DROPEFFECT_LINK);
        let mut effect = DROPEFFECT_COPY;
        unsafe { (vtbl.drop)(target, f.0, 0, at(12, 21), &mut effect) };
        assert_eq!(effect, DROPEFFECT_COPY);

        let queued = drags.take();
        assert_eq!(queued.len(), 3, "{queued:?}");
        assert!(matches!(queued[0], Drag::Enter(p) if (p.x, p.y) == (10.0, 20.0)));
        // Resting at (10, 20) was no move.
        assert!(matches!(queued[1], Drag::Move(p) if (p.x, p.y) == (11.0, 20.0)));
        match &queued[2] {
            Drag::Drop(p, data) => {
                assert_eq!((p.x, p.y), (12.0, 21.0));
                assert_eq!(data.urls, ["https://example.com/"]);
            }
            other => panic!("{other:?}"),
        }

        // A drag of nothing it reads isn't announced at all.
        let empty = Fixture::new();
        let mut effect = DROPEFFECT_COPY;
        unsafe { (vtbl.drag_enter)(target, empty.0, 0, at(0, 0), &mut effect) };
        assert_eq!(effect, DROPEFFECT_NONE);
        unsafe { (vtbl.drag_leave)(target) };
        assert!(drags.take().is_empty());

        unsafe { DropTarget::release(target) };
    }
}
