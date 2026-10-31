//! X11: the main window's own XDND drop target, in place of winit's.
//!
//! winit 0.30 reads a drag's `text/uri-list` and nothing else, and throws the
//! whole list away when one address in it isn't a local file's: a link or an
//! image dragged out of a web browser never reached the app. Nor did winit say
//! where the pointer was. This target reads a drag the way the Wayland backend
//! does (see [`crate::mime_drop`]), along with where the pointer is, so the
//! window gets [`Event::DragMove`]s on X11 too.
//!
//! A source sends its XDND messages to the window under the pointer, and the X
//! server delivers a message sent to a window to the client that made it —
//! winit, which keeps them to itself. XDND lets a window name another to take
//! its messages instead, its `XdndProxy`, and [`register`] makes an unmapped
//! window of saudade's own that, on the connection [`crate::x11`] opens. A
//! thread of its own takes the messages up as they come in, queues the drags
//! for the runtime the way `win32_drop`'s target does, and wakes winit's loop
//! to dispatch them in `about_to_wait`. GTK, Qt, Firefox and Chromium look for
//! a proxy; a source that doesn't still talks to winit, and the files it drags
//! arrive as before.
//!
//! A source waits for the answer to one position before it sends the next, so
//! unlike on Windows the answer can wait for the widgets: the runtime gives it
//! once they have seen the position (see [`Drags::set_accepted`]). A drag
//! released where no widget took it is no drop then; the source leaves.
//!
//! [`Event::DragMove`]: crate::Event::DragMove

use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::thread;

use winit::dpi::PhysicalPosition;
use winit::event_loop::EventLoopProxy;
use winit::window::Window;
use x11rb::connection::Connection as _;
use x11rb::protocol::Event as XEvent;
use x11rb::protocol::xproto::{
    Atom, AtomEnum, ClientMessageEvent, ConnectionExt as _, CreateWindowAux, EventMask, PropMode,
    Property, Timestamp, Window as XWindow, WindowClass,
};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

use crate::event::DragData;
use crate::mime_drop::Reading;

/// The XDND version spoken: the last, 5, in which `XdndFinished` says whether
/// the drop was taken.
const XDND_VERSION: u32 = 5;

// `SAUDADE_DROP` is the property on the proxy each form of a drop is read
// into.
x11rb::atom_manager! {
    Atoms: AtomsCookie {
        XdndAware,
        XdndProxy,
        XdndEnter,
        XdndPosition,
        XdndStatus,
        XdndLeave,
        XdndDrop,
        XdndFinished,
        XdndSelection,
        XdndTypeList,
        XdndActionCopy,
        XdndActionLink,
        XdndDirectSave0,
        INCR,
        SAUDADE_DROP,
    }
}

/// A drag as the drop target saw it, waiting for the runtime to turn it into
/// an [`Event`](crate::Event). Positions are physical pixels in the window, as
/// winit reports the pointer's.
#[derive(Debug)]
pub(crate) enum Drag {
    Enter(PhysicalPosition<f64>),
    Move(PhysicalPosition<f64>),
    Leave,
    Drop(PhysicalPosition<f64>, DragData),
}

/// The runtime's side of the drop target.
pub(crate) struct Drags(Arc<Shared>);

/// What the drop target's thread and the runtime share.
struct Shared {
    conn: &'static RustConnection,
    atoms: Atoms,
    queue: Mutex<Vec<Drag>>,
    answer: Mutex<Answer>,
}

#[derive(Default)]
struct Answer {
    /// Whether a widget accepted the drag at the last position the runtime
    /// dispatched; `None` until it has dispatched one.
    accepted: Option<bool>,
    /// The position the source waits on an answer to, which the runtime gives
    /// once it has dispatched it.
    waiting: Option<Status>,
}

/// Where an answer goes, and the action a drag is taken with.
#[derive(Clone, Copy, Debug)]
struct Status {
    source: XWindow,
    /// The window the source's messages are about, which an answer names:
    /// winit's, or the proxy itself, depending on the source.
    target: XWindow,
    action: Atom,
}

impl Drags {
    /// The drags queued since the last call.
    pub(crate) fn take(&self) -> Vec<Drag> {
        std::mem::take(&mut *self.0.queue.lock().unwrap())
    }

    /// The widget tree's answer to the drag at the last position, which goes
    /// to the source waiting on it.
    pub(crate) fn set_accepted(&self, accepted: Option<bool>) {
        let mut answer = self.0.answer.lock().unwrap();
        answer.accepted = accepted;
        if let Some(accepted) = accepted
            && let Some(status) = answer.waiting.take()
        {
            self.0.status(status, accepted);
        }
    }

    pub(crate) fn accepted(&self) -> Option<bool> {
        self.0.answer.lock().unwrap().accepted
    }
}

impl Shared {
    /// Answer a position: whether the drag would be dropped there, and how.
    fn status(&self, to: Status, accepted: bool) {
        // Bit 1: send every position, as the widgets may answer each one
        // differently.
        let flags = 2 | accepted as u32;
        let action = if accepted { to.action } else { x11rb::NONE };
        let data = [to.target, flags, 0, 0, action];
        self.send(to.source, self.atoms.XdndStatus, data);
    }

    /// Tell the source its drop is done with, and whether it was taken.
    fn finished(&self, to: Status, taken: bool) {
        let action = if taken { to.action } else { x11rb::NONE };
        let data = [to.target, taken as u32, action, 0, 0];
        self.send(to.source, self.atoms.XdndFinished, data);
    }

    fn send(&self, to: XWindow, kind: Atom, data: [u32; 5]) {
        let message = ClientMessageEvent::new(32, to, kind, data);
        // A source gone by now has nothing more to be told.
        let _ = self
            .conn
            .send_event(false, to, EventMask::NO_EVENT, message);
        let _ = self.conn.flush();
    }
}

/// Make `win` take drags through a drop target of saudade's, whose thread
/// wakes the event loop through `wake` when it has queued some. `None` where
/// that can't be set up; winit's own target takes the drags then.
pub(crate) fn register(win: &Window, wake: EventLoopProxy<()>) -> Option<Rc<Drags>> {
    let window = crate::x11::xid(win)?;
    let conn = crate::x11::connection()?;
    start(
        conn,
        window,
        Box::new(move || {
            let _ = wake.send_event(());
        }),
    )
}

fn start(
    conn: &'static RustConnection,
    window: XWindow,
    wake: Box<dyn Fn() + Send>,
) -> Option<Rc<Drags>> {
    let atoms = Atoms::new(conn).ok()?.reply().ok()?;
    let root = conn.get_geometry(window).ok()?.reply().ok()?.root;
    let proxy = conn.generate_id().ok()?;
    // Never mapped, and input only: there is nothing of it to see.
    let aux = CreateWindowAux::new().event_mask(EventMask::PROPERTY_CHANGE);
    conn.create_window(
        x11rb::COPY_DEPTH_FROM_PARENT,
        proxy,
        root,
        0,
        0,
        1,
        1,
        0,
        WindowClass::INPUT_ONLY,
        x11rb::COPY_FROM_PARENT,
        &aux,
    )
    .ok()?
    .check()
    .ok()?;
    // A proxy names itself, which tells it from one a crashed client left.
    conn.change_property32(
        PropMode::REPLACE,
        proxy,
        atoms.XdndProxy,
        AtomEnum::WINDOW,
        &[proxy],
    )
    .ok()?;
    conn.change_property32(
        PropMode::REPLACE,
        proxy,
        atoms.XdndAware,
        AtomEnum::ATOM,
        &[XDND_VERSION],
    )
    .ok()?;
    let shared = Arc::new(Shared {
        conn,
        atoms,
        queue: Mutex::default(),
        answer: Mutex::default(),
    });
    let target = Target {
        shared: shared.clone(),
        proxy,
        window,
        root,
        wake,
        drag: None,
        dropped: None,
    };
    thread::Builder::new()
        .name("saudade-xdnd".into())
        .spawn(move || target.run())
        .ok()?;
    // Sources go to the proxy from now on, as there is someone to answer.
    conn.change_property32(
        PropMode::REPLACE,
        window,
        atoms.XdndProxy,
        AtomEnum::WINDOW,
        &[proxy],
    )
    .ok()?
    .check()
    .ok()?;
    Some(Rc::new(Drags(shared)))
}

/// The drop target, on its thread.
struct Target {
    shared: Arc<Shared>,
    /// The window XDND messages come to.
    proxy: XWindow,
    /// winit's, which the drags are over.
    window: XWindow,
    root: XWindow,
    wake: Box<dyn Fn() + Send>,
    drag: Option<Incoming>,
    dropped: Option<Dropped>,
}

/// A drag over the window.
struct Incoming {
    version: u32,
    /// What it offers, by MIME type.
    types: Vec<String>,
    /// It carries something a drop reads (see `mime_drop::first`).
    readable: bool,
    /// Where it was last reported, on the root window and in the window;
    /// `None` until the first position, which is where it enters.
    at: Option<((i16, i16), PhysicalPosition<f64>)>,
    status: Status,
}

impl Incoming {
    /// The runtime was told it entered.
    fn entered(&self) -> bool {
        self.readable && self.at.is_some()
    }
}

/// A drop being read, form by form.
struct Dropped {
    status: Status,
    time: Timestamp,
    pos: PhysicalPosition<f64>,
    reading: Reading,
    /// The form asked for last.
    form: Option<String>,
    /// Its bytes so far, while it comes in parts (`INCR`).
    parts: Option<Vec<u8>>,
}

impl Target {
    fn run(mut self) {
        let conn = self.shared.conn;
        let atoms = self.shared.atoms;
        while let Ok(event) = conn.wait_for_event() {
            match event {
                XEvent::ClientMessage(msg) if msg.format == 32 => self.message(&msg),
                XEvent::SelectionNotify(e) if e.requestor == self.proxy => {
                    self.selection(e.property)
                }
                XEvent::PropertyNotify(e)
                    if e.window == self.proxy
                        && e.atom == atoms.SAUDADE_DROP
                        && e.state == Property::NEW_VALUE =>
                {
                    self.part()
                }
                _ => {}
            }
        }
    }

    fn message(&mut self, msg: &ClientMessageEvent) {
        let atoms = self.shared.atoms;
        let data = msg.data.as_data32();
        if msg.type_ == atoms.XdndEnter {
            self.enter(msg.window, data);
        } else if msg.type_ == atoms.XdndPosition {
            self.position(msg.window, data);
        } else if msg.type_ == atoms.XdndLeave {
            self.leave(data);
        } else if msg.type_ == atoms.XdndDrop {
            self.release(data);
        }
    }

    /// Queue `drag` for the runtime, and wake it to dispatch it.
    fn queue(&self, drag: Drag) {
        self.shared.queue.lock().unwrap().push(drag);
        (self.wake)();
    }

    /// A drag came over the window, offering the types in `data` or, with
    /// more than three, in the source's `XdndTypeList`. Where it is comes with
    /// the first position.
    fn enter(&mut self, target: XWindow, data: [u32; 5]) {
        let source = data[0];
        self.abandon();
        let atoms = if data[1] & 1 != 0 {
            self.type_list(source)
        } else {
            data[2..]
                .iter()
                .copied()
                .filter(|&t| t != x11rb::NONE)
                .collect()
        };
        let types = self.names(&atoms);
        let readable = crate::mime_drop::first(&types).is_some();
        let status = Status {
            source,
            target,
            action: self.shared.atoms.XdndActionCopy,
        };
        self.drag = Some(Incoming {
            version: data[1] >> 24,
            types,
            readable,
            at: None,
            status,
        });
    }

    /// Let go of the drag there was before another one entered: one whose
    /// source never said it left, or a drop that was never read to the end.
    fn abandon(&mut self) {
        let mut left = self.drag.take().is_some_and(|d| d.entered());
        if let Some(dropped) = self.dropped.take() {
            self.shared.finished(dropped.status, false);
            left = true;
        }
        if left {
            self.queue(Drag::Leave);
        }
    }

    /// The drag moved, to where `data[2]` says on the root window.
    fn position(&mut self, target: XWindow, data: [u32; 5]) {
        let atoms = self.shared.atoms;
        let Some(drag) = self.drag.as_mut().filter(|d| d.status.source == data[0]) else {
            return;
        };
        // Never move what's dropped: copy it, or link to it where that's what
        // the user asks for.
        let link = drag.version >= 2 && data[4] == atoms.XdndActionLink;
        drag.status.action = if link {
            atoms.XdndActionLink
        } else {
            atoms.XdndActionCopy
        };
        drag.status.target = target;
        let status = drag.status;
        if !drag.readable {
            return self.shared.status(status, false);
        }
        let root = ((data[2] >> 16) as i16, data[2] as i16);
        if let Some((was, _)) = drag.at
            && was == root
        {
            // Asked again where the pointer rests: answer as before, or let the
            // runtime, if it hasn't yet.
            let mut answer = self.shared.answer.lock().unwrap();
            match answer.waiting {
                Some(_) => answer.waiting = Some(status),
                None => {
                    let accepted = answer.accepted == Some(true);
                    self.shared.status(status, accepted);
                }
            }
            return;
        }
        let conn = self.shared.conn;
        let Some(pos) = conn
            .translate_coordinates(self.root, self.window, root.0, root.1)
            .ok()
            .and_then(|c| c.reply().ok())
            .map(|r| PhysicalPosition::new(r.dst_x as f64, r.dst_y as f64))
        else {
            return self.shared.status(status, false);
        };
        let entering = drag.at.replace((root, pos)).is_none();
        {
            let mut answer = self.shared.answer.lock().unwrap();
            if entering {
                answer.accepted = None;
            }
            answer.waiting = Some(status);
        }
        self.queue(if entering {
            Drag::Enter(pos)
        } else {
            Drag::Move(pos)
        });
    }

    /// The drag left without dropping.
    fn leave(&mut self, data: [u32; 5]) {
        let Some(drag) = self.drag.take_if(|d| d.status.source == data[0]) else {
            return;
        };
        self.shared.answer.lock().unwrap().waiting = None;
        if drag.entered() {
            self.queue(Drag::Leave);
        }
    }

    /// The drag was dropped: read it, unless no widget took it.
    fn release(&mut self, data: [u32; 5]) {
        let Some(drag) = self.drag.take_if(|d| d.status.source == data[0]) else {
            return;
        };
        let accepted = {
            let mut answer = self.shared.answer.lock().unwrap();
            answer.waiting = None;
            answer.accepted
        };
        let taken = drag.readable && accepted != Some(false);
        let Some((_, pos)) = drag.at.filter(|_| taken) else {
            if drag.entered() {
                self.queue(Drag::Leave);
            }
            return self.shared.finished(drag.status, false);
        };
        let suggests = drag.types.iter().any(|t| t == "XdndDirectSave0");
        let mut reading = Reading::new(drag.types);
        if suggests && let Some(name) = self.suggested_name(drag.status.source) {
            reading.suggest_name(&name);
        }
        self.dropped = Some(Dropped {
            status: drag.status,
            time: if drag.version >= 1 {
                data[2]
            } else {
                x11rb::CURRENT_TIME
            },
            pos,
            reading,
            form: None,
            parts: None,
        });
        self.ask();
    }

    /// Ask the source for the next form of the drop, or, with none left, hand
    /// the runtime what it carried.
    fn ask(&mut self) {
        let conn = self.shared.conn;
        let atoms = self.shared.atoms;
        let Some(dropped) = self.dropped.as_mut() else {
            return;
        };
        while let Some(form) = dropped.reading.next() {
            let Some(kind) = conn
                .intern_atom(false, form.as_bytes())
                .ok()
                .and_then(|c| c.reply().ok())
            else {
                continue;
            };
            let asked = conn.convert_selection(
                self.proxy,
                atoms.XdndSelection,
                kind.atom,
                atoms.SAUDADE_DROP,
                dropped.time,
            );
            if asked.is_ok() && conn.flush().is_ok() {
                dropped.form = Some(form);
                return;
            }
        }
        let Some(dropped) = self.dropped.take() else {
            return;
        };
        let data = dropped.reading.finish();
        self.shared.finished(dropped.status, !data.is_empty());
        self.queue(Drag::Drop(dropped.pos, data));
    }

    /// The form asked for came in — or, with `None`, didn't — so on to the
    /// next.
    fn got(&mut self, bytes: Option<Vec<u8>>) {
        if let Some(dropped) = self.dropped.as_mut()
            && let Some(form) = dropped.form.take()
        {
            dropped.parts = None;
            if let Some(bytes) = bytes {
                dropped.reading.take(&form, bytes);
            }
        }
        self.ask();
    }

    /// The source wrote out the form asked for into `property`, or, with none,
    /// said it can't.
    fn selection(&mut self, property: Atom) {
        if self.dropped.as_ref().is_none_or(|d| d.form.is_none()) {
            return;
        }
        if property == x11rb::NONE {
            return self.got(None);
        }
        match self.take_property() {
            // Deleting the property, as reading it did, asks for the first part.
            Some((kind, _)) if kind == self.shared.atoms.INCR => {
                if let Some(dropped) = self.dropped.as_mut() {
                    dropped.parts = Some(Vec::new());
                }
            }
            reply => self.got(reply.map(|(_, bytes)| bytes)),
        }
    }

    /// The next part of a form that comes in parts.
    fn part(&mut self) {
        if self.dropped.as_ref().is_none_or(|d| d.parts.is_none()) {
            return;
        }
        match self.take_property() {
            // An empty part ends it.
            Some((_, bytes)) if bytes.is_empty() => {
                let parts = self.dropped.as_mut().and_then(|d| d.parts.take());
                self.got(parts);
            }
            Some((_, bytes)) => {
                if let Some(parts) = self.dropped.as_mut().and_then(|d| d.parts.as_mut()) {
                    parts.extend(bytes);
                }
            }
            None => self.got(None),
        }
    }

    /// Read and delete the property a form is read into: its type and bytes.
    fn take_property(&self) -> Option<(Atom, Vec<u8>)> {
        let reply = self
            .shared
            .conn
            .get_property(
                true,
                self.proxy,
                self.shared.atoms.SAUDADE_DROP,
                AtomEnum::ANY,
                0,
                u32::MAX,
            )
            .ok()?
            .reply()
            .ok()?;
        Some((reply.type_, reply.value))
    }

    /// The types a source offers in its `XdndTypeList`.
    fn type_list(&self, source: XWindow) -> Vec<Atom> {
        let list = self
            .shared
            .conn
            .get_property(
                false,
                source,
                self.shared.atoms.XdndTypeList,
                AtomEnum::ATOM,
                0,
                u32::MAX,
            )
            .ok()
            .and_then(|c| c.reply().ok());
        list.and_then(|l| Some(l.value32()?.collect()))
            .unwrap_or_default()
    }

    /// The names of `atoms`: MIME types, for the types a drag offers.
    fn names(&self, atoms: &[Atom]) -> Vec<String> {
        let conn = self.shared.conn;
        let cookies: Vec<_> = atoms.iter().map(|&a| conn.get_atom_name(a)).collect();
        cookies
            .into_iter()
            .filter_map(|c| String::from_utf8(c.ok()?.reply().ok()?.name).ok())
            .collect()
    }

    /// The name a source suggests for the file it drags by its contents, on
    /// its window's `XdndDirectSave0`.
    fn suggested_name(&self, source: XWindow) -> Option<String> {
        let reply = self
            .shared
            .conn
            .get_property(
                false,
                source,
                self.shared.atoms.XdndDirectSave0,
                AtomEnum::ANY,
                0,
                1024,
            )
            .ok()?
            .reply()
            .ok()?;
        Some(String::from_utf8_lossy(&reply.value).into_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};
    use x11rb::protocol::xproto::{
        ChangeWindowAttributesAux, SELECTION_NOTIFY_EVENT, SelectionNotifyEvent,
        SelectionRequestEvent,
    };

    /// The first event on `conn` that `wanted` picks, failing after a while.
    fn next<T>(conn: &RustConnection, mut wanted: impl FnMut(XEvent) -> Option<T>) -> T {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match conn.poll_for_event().unwrap() {
                Some(event) => {
                    if let Some(t) = wanted(event) {
                        return t;
                    }
                }
                None => {
                    assert!(Instant::now() < deadline, "no event came");
                    thread::sleep(Duration::from_millis(2));
                }
            }
        }
    }

    /// The window winit would have made, at (10, 20), with saudade's drop
    /// target on it. `None` without an X server to test against.
    fn window() -> Option<(RustConnection, XWindow, Rc<Drags>, mpsc::Receiver<()>)> {
        let (app, screen) = x11rb::connect(None).ok()?;
        let root = app.setup().roots[screen].root;
        let window = app.generate_id().unwrap();
        let aux = CreateWindowAux::new();
        app.create_window(
            0,
            window,
            root,
            10,
            20,
            100,
            100,
            0,
            WindowClass::INPUT_OUTPUT,
            0,
            &aux,
        )
        .unwrap()
        .check()
        .unwrap();
        let (conn, _) = x11rb::connect(None).unwrap();
        let conn: &'static RustConnection = Box::leak(Box::new(conn));
        let (woke, wakes) = mpsc::channel();
        let drags = start(
            conn,
            window,
            Box::new(move || {
                let _ = woke.send(());
            }),
        )
        .unwrap();
        Some((app, window, drags, wakes))
    }

    /// What the runtime has to dispatch, once woken to.
    fn woken(drags: &Drags, wakes: &mpsc::Receiver<()>) -> Vec<Drag> {
        wakes.recv_timeout(Duration::from_secs(5)).unwrap();
        while wakes.try_recv().is_ok() {}
        drags.take()
    }

    /// Another program dragging something onto the window, the way Qt does:
    /// to the proxy the window names, about the window.
    struct Source {
        conn: RustConnection,
        window: XWindow,
        atoms: Atoms,
        proxy: XWindow,
        target: XWindow,
    }

    impl Source {
        fn new(target: XWindow) -> Self {
            let (conn, screen) = x11rb::connect(None).unwrap();
            let root = conn.setup().roots[screen].root;
            let window = conn.generate_id().unwrap();
            let aux = CreateWindowAux::new();
            conn.create_window(
                0,
                window,
                root,
                0,
                0,
                1,
                1,
                0,
                WindowClass::INPUT_ONLY,
                0,
                &aux,
            )
            .unwrap()
            .check()
            .unwrap();
            let atoms = Atoms::new(&conn).unwrap().reply().unwrap();
            let proxy = conn
                .get_property(false, target, atoms.XdndProxy, AtomEnum::WINDOW, 0, 1)
                .unwrap()
                .reply()
                .unwrap()
                .value32()
                .and_then(|mut v| v.next())
                .expect("the window names a proxy");
            let aware = conn
                .get_property(false, proxy, atoms.XdndAware, AtomEnum::ATOM, 0, 1)
                .unwrap()
                .reply()
                .unwrap();
            assert_eq!(aware.value32().unwrap().next(), Some(XDND_VERSION));
            Source {
                conn,
                window,
                atoms,
                proxy,
                target,
            }
        }

        fn atom(&self, name: &str) -> Atom {
            let cookie = self.conn.intern_atom(false, name.as_bytes()).unwrap();
            cookie.reply().unwrap().atom
        }

        fn send(&self, kind: Atom, mut data: [u32; 5]) {
            data[0] = self.window;
            let message = ClientMessageEvent::new(32, self.target, kind, data);
            self.conn
                .send_event(false, self.proxy, EventMask::NO_EVENT, message)
                .unwrap();
            self.conn.flush().unwrap();
        }

        fn enter(&self, types: &[&str]) {
            let mut data = [0, XDND_VERSION << 24, 0, 0, 0];
            for (slot, t) in data[2..].iter_mut().zip(types) {
                *slot = self.atom(t);
            }
            self.send(self.atoms.XdndEnter, data);
        }

        /// Move to (`x`, `y`) on the root window, asking for `action`.
        fn position(&self, x: u32, y: u32, action: Atom) {
            self.send(self.atoms.XdndPosition, [0, 0, x << 16 | y, 0, action]);
        }

        /// The target's answer: whether it would take the drop, and how.
        fn status(&self) -> (bool, Atom) {
            let status = self.atoms.XdndStatus;
            let data = next(&self.conn, |e| match e {
                XEvent::ClientMessage(m) if m.type_ == status => Some(m.data.as_data32()),
                _ => None,
            });
            assert_eq!(data[0], self.target);
            (data[1] & 1 != 0, data[4])
        }

        /// Hand the drop over, `forms` by type and in parts of at most `part`
        /// bytes, until the target says it's finished: whether it took it, and
        /// how.
        fn serve(&self, forms: &[(&str, Vec<u8>)], part: usize) -> (bool, Atom) {
            loop {
                match next(&self.conn, Some) {
                    XEvent::SelectionRequest(r) => self.answer(&r, forms, part),
                    XEvent::ClientMessage(m) if m.type_ == self.atoms.XdndFinished => {
                        let data = m.data.as_data32();
                        assert_eq!(data[0], self.target);
                        return (data[1] & 1 != 0, data[2]);
                    }
                    _ => {}
                }
            }
        }

        fn answer(&self, r: &SelectionRequestEvent, forms: &[(&str, Vec<u8>)], part: usize) {
            let conn = &self.conn;
            assert_eq!(r.selection, self.atoms.XdndSelection);
            let name = conn.get_atom_name(r.target).unwrap().reply().unwrap().name;
            let bytes = forms.iter().find(|(f, _)| f.as_bytes() == name);
            let property = match bytes {
                None => x11rb::NONE,
                Some((_, bytes)) if bytes.len() > part => {
                    let aux =
                        ChangeWindowAttributesAux::new().event_mask(EventMask::PROPERTY_CHANGE);
                    conn.change_window_attributes(r.requestor, &aux).unwrap();
                    let size = [bytes.len() as u32];
                    let incr = self.atoms.INCR;
                    conn.change_property32(PropMode::REPLACE, r.requestor, r.property, incr, &size)
                        .unwrap();
                    self.notify(r, r.property);
                    for chunk in bytes.chunks(part).chain([&[][..]]) {
                        next(conn, |e| match e {
                            XEvent::PropertyNotify(p)
                                if p.window == r.requestor
                                    && p.atom == r.property
                                    && p.state == Property::DELETE =>
                            {
                                Some(())
                            }
                            _ => None,
                        });
                        conn.change_property8(
                            PropMode::REPLACE,
                            r.requestor,
                            r.property,
                            r.target,
                            chunk,
                        )
                        .unwrap();
                        conn.flush().unwrap();
                    }
                    return;
                }
                Some((_, bytes)) => {
                    conn.change_property8(
                        PropMode::REPLACE,
                        r.requestor,
                        r.property,
                        r.target,
                        bytes,
                    )
                    .unwrap();
                    r.property
                }
            };
            self.notify(r, property);
        }

        fn notify(&self, r: &SelectionRequestEvent, property: Atom) {
            let event = SelectionNotifyEvent {
                response_type: SELECTION_NOTIFY_EVENT,
                sequence: 0,
                time: r.time,
                requestor: r.requestor,
                selection: r.selection,
                target: r.target,
                property,
            };
            self.conn
                .send_event(false, r.requestor, EventMask::NO_EVENT, event)
                .unwrap();
            self.conn.flush().unwrap();
        }
    }

    #[test]
    fn reads_an_image_dragged_out_of_a_browser() {
        let Some((_app, window, drags, wakes)) = window() else {
            return;
        };
        let source = Source::new(window);
        let (copy, link) = (source.atoms.XdndActionCopy, source.atoms.XdndActionLink);
        source
            .conn
            .set_selection_owner(
                source.window,
                source.atoms.XdndSelection,
                x11rb::CURRENT_TIME,
            )
            .unwrap();
        source.enter(&["text/uri-list", "image/png"]);

        // Each position waits for the widgets' answer to it.
        source.position(15, 30, copy);
        let queued = woken(&drags, &wakes);
        assert!(matches!(queued[..], [Drag::Enter(p)] if (p.x, p.y) == (5.0, 10.0)));
        drags.set_accepted(Some(true));
        assert_eq!(source.status(), (true, copy));

        // Turned down, even for a move the user asks for.
        source.position(16, 30, source.atom("XdndActionMove"));
        let queued = woken(&drags, &wakes);
        assert!(matches!(queued[..], [Drag::Move(p)] if (p.x, p.y) == (6.0, 10.0)));
        drags.set_accepted(Some(false));
        assert_eq!(source.status(), (false, x11rb::NONE));
        // Asked again where it rests, answered as before, without the widgets.
        source.position(16, 30, copy);
        assert_eq!(source.status(), (false, x11rb::NONE));
        assert!(drags.take().is_empty());

        // Taken again, as a link where the user asks for one.
        source.position(17, 30, link);
        assert_eq!(woken(&drags, &wakes).len(), 1);
        drags.set_accepted(Some(true));
        assert_eq!(source.status(), (true, link));

        // The image comes in parts: bigger than a property takes in one.
        let image: Vec<u8> = (0..100_000u32).map(|i| i as u8).collect();
        source.send(source.atoms.XdndDrop, [0; 5]);
        let forms = [
            (
                "text/uri-list",
                b"https://example.com/img/cat.jpg\r\n".to_vec(),
            ),
            ("image/png", image.clone()),
        ];
        assert_eq!(source.serve(&forms, 30_000), (true, link));
        match &woken(&drags, &wakes)[..] {
            [Drag::Drop(p, data)] => {
                assert_eq!((p.x, p.y), (7.0, 10.0));
                assert_eq!(data.urls, ["https://example.com/img/cat.jpg"]);
                assert_eq!(data.files.len(), 1);
                assert_eq!(data.files[0].name, "cat.png");
                assert!(data.files[0].contents == image);
                assert!(data.paths.is_empty());
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn leaves_what_it_cant_read_or_the_widgets_turn_down() {
        let Some((_app, window, drags, wakes)) = window() else {
            return;
        };
        let source = Source::new(window);
        let copy = source.atoms.XdndActionCopy;

        // Plain text is nothing a drop reads: turned down without the widgets.
        source.enter(&["text/plain", "UTF8_STRING"]);
        source.position(15, 30, copy);
        assert_eq!(source.status(), (false, x11rb::NONE));
        source.send(source.atoms.XdndLeave, [0; 5]);

        // Dropped where no widget takes it: the drag leaves, nothing is read.
        source.enter(&["text/uri-list"]);
        source.position(15, 30, copy);
        let queued = woken(&drags, &wakes);
        assert!(matches!(queued[..], [Drag::Enter(_)]), "{queued:?}");
        drags.set_accepted(Some(false));
        assert_eq!(source.status(), (false, x11rb::NONE));
        source.send(source.atoms.XdndDrop, [0; 5]);
        assert_eq!(source.serve(&[], 1), (false, x11rb::NONE));
        assert!(matches!(woken(&drags, &wakes)[..], [Drag::Leave]));
    }
}
