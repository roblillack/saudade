//! The freedesktop settings portal's `color-scheme`: whether the desktop is in
//! light or dark mode, on X11 and Wayland.
//!
//! GNOME, KDE and most other desktops publish their light / dark switch as the
//! `org.freedesktop.appearance` / `color-scheme` setting of
//! `org.freedesktop.portal.Settings`, a D-Bus service on the session bus. Rather
//! than pull in a D-Bus library for one method call and one signal, this speaks
//! the little of the wire protocol that takes: the `EXTERNAL` handshake, method
//! calls whose arguments are all strings, and decoding the reply and the
//! signals that come back — little-endian or big, the variant around the value
//! single or double, as portals old and new send it.
//!
//! It all runs on a thread of its own, which [`watch`] waits on for a moment
//! only, so a slow or missing portal costs startup no more than that.
//!
//! Built on every Unix, though only used where neither winit nor the platform
//! has the answer — not on macOS, where it is compiled solely so its tests can
//! run against a `dbus-daemon` there too.
#![cfg_attr(target_os = "macos", allow(dead_code))]

use std::ffi::OsStr;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use crate::appearance::Appearance;

/// How long [`watch`] holds startup up for the first answer. A portal that is
/// running answers in a millisecond or two; one the bus has to start first
/// can take a good deal longer, and its answer then arrives as a change.
const FIRST_ANSWER: Duration = Duration::from_millis(250);

/// How long the handshake and the calls that set the watch up may take before
/// the bus is given up on. Waiting for a change, later, takes as long as it
/// takes.
const SETUP_TIMEOUT: Duration = Duration::from_secs(5);

const BUS: &str = "org.freedesktop.DBus";
const BUS_PATH: &str = "/org/freedesktop/DBus";
const PORTAL: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const SETTINGS: &str = "org.freedesktop.portal.Settings";
const NAMESPACE: &str = "org.freedesktop.appearance";
const KEY: &str = "color-scheme";

/// Ask the portal for the desktop's appearance, and keep listening for changes
/// on a thread of its own.
///
/// Returns the appearance if the portal answered within [`FIRST_ANSWER`]. Every
/// change after that — and a first answer that came too late to be returned —
/// goes to `notify`, until it returns `false`. No portal, no session bus, or a
/// portal without the setting: `None`, and nothing ever notified.
pub(crate) fn watch(
    mut notify: impl FnMut(Appearance) -> bool + Send + 'static,
) -> Option<Appearance> {
    let address = session_bus_address()?;
    let first = Arc::new(FirstAnswer::default());
    let offered = first.clone();
    let spawned = std::thread::Builder::new()
        .name("saudade-portal".into())
        .spawn(move || {
            // An error just ends the watch: whatever went wrong, the app
            // paints in the theme it already has.
            let _ = listen(&address, |appearance| {
                offered.offer(appearance) || notify(appearance)
            });
            offered.give_up();
        });
    if spawned.is_err() {
        return None;
    }
    first.wait(FIRST_ANSWER)
}

/// The hand-off of the portal's first answer from its thread to the one
/// waiting in [`watch`], race-free whichever of them gets there first.
#[derive(Default)]
struct FirstAnswer {
    state: Mutex<First>,
    changed: Condvar,
}

#[derive(Default)]
enum First {
    /// The waiter is waiting.
    #[default]
    Pending,
    /// It has its answer, or will on waking.
    Answered(Appearance),
    /// The watch ended without one.
    Failed,
    /// It stopped waiting: anything from now on is a change.
    Gone,
}

impl FirstAnswer {
    /// Hand `appearance` to the waiter. `true` if it takes it; `false` if it
    /// already has one or is no longer waiting, which makes this a change to
    /// report the usual way.
    fn offer(&self, appearance: Appearance) -> bool {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if !matches!(*state, First::Pending) {
            return false;
        }
        *state = First::Answered(appearance);
        self.changed.notify_all();
        true
    }

    /// The watch ended: stop the waiter waiting for an answer that won't come.
    fn give_up(&self) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if matches!(*state, First::Pending) {
            *state = First::Failed;
            self.changed.notify_all();
        }
    }

    /// Wait up to `timeout` for the first answer.
    fn wait(&self, timeout: Duration) -> Option<Appearance> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let (mut state, _) = self
            .changed
            .wait_timeout_while(state, timeout, |s| matches!(s, First::Pending))
            .unwrap_or_else(|e| e.into_inner());
        match *state {
            First::Answered(appearance) => Some(appearance),
            _ => {
                *state = First::Gone;
                None
            }
        }
    }
}

/// Connect to the bus at `address`, read the color scheme and report it, then
/// report every change to it — while `report` keeps returning `true`.
fn listen(address: &str, mut report: impl FnMut(Appearance) -> bool) -> io::Result<()> {
    let mut bus = Bus::connect(address)?;
    // Subscribe before asking, so a change made in between is not missed.
    let rule = format!(
        "type='signal',interface='{SETTINGS}',member='SettingChanged',\
         path='{PORTAL_PATH}',arg0='{NAMESPACE}',arg1='{KEY}'"
    );
    bus.call(BUS, BUS_PATH, BUS, "AddMatch", &[&rule])?;
    // `Read` rather than the newer `ReadOne`, which only portals from 2023 on
    // answer; the value comes back double-wrapped, which `scheme` unwraps.
    let read = bus.call(PORTAL, PORTAL_PATH, SETTINGS, "Read", &[NAMESPACE, KEY])?;
    bus.set_timeout(None)?;
    loop {
        let message = bus.receive()?;
        let appearance = match message.kind {
            MessageKind::Return if message.reply_serial == Some(read) => {
                scheme(&mut message.body_reader())?
            }
            // No portal running, or one without the setting.
            MessageKind::Error if message.reply_serial == Some(read) => {
                return Err(io::Error::other(format!(
                    "the settings portal has no {KEY}: {}",
                    message.error_name.unwrap_or_default(),
                )));
            }
            MessageKind::Signal if message.is_setting_changed() => {
                let mut body = message.body_reader();
                match (body.string(), body.string()) {
                    (Ok(namespace), Ok(key)) if namespace == NAMESPACE && key == KEY => {
                        match scheme(&mut body) {
                            Ok(appearance) => appearance,
                            Err(_) => continue,
                        }
                    }
                    _ => continue,
                }
            }
            _ => continue,
        };
        if !report(appearance) {
            return Ok(());
        }
    }
}

/// Decode a `color-scheme` value: 1 asks for dark; 0 (no preference) and 2
/// (light) for light.
fn scheme(body: &mut Reader) -> io::Result<Appearance> {
    match body.value(b"v")? {
        Value::U32(1) => Ok(Appearance::Dark),
        Value::U32(_) => Ok(Appearance::Light),
        _ => Err(malformed("color-scheme is not a number")),
    }
}

/// Where the session bus listens: `DBUS_SESSION_BUS_ADDRESS`, or else the
/// socket a systemd user session keeps at `$XDG_RUNTIME_DIR/bus`.
fn session_bus_address() -> Option<String> {
    if let Some(address) = std::env::var("DBUS_SESSION_BUS_ADDRESS")
        .ok()
        .filter(|a| !a.is_empty())
    {
        return Some(address);
    }
    let dir = std::env::var_os("XDG_RUNTIME_DIR")?;
    let socket = PathBuf::from(dir).join("bus");
    socket
        .exists()
        .then(|| format!("unix:path={}", socket.display()))
}

/// Connect to the first of the `;`-separated addresses that takes the
/// connection — `unix:path=…`, or on Linux `unix:abstract=…`.
fn connect_stream(addresses: &str) -> io::Result<UnixStream> {
    let mut last = io::Error::new(io::ErrorKind::NotFound, "no usable D-Bus address");
    for address in addresses.split(';') {
        let Some(params) = address.strip_prefix("unix:") else {
            continue;
        };
        for param in params.split(',') {
            let Some((key, value)) = param.split_once('=') else {
                continue;
            };
            let Some(value) = unescape(value) else {
                continue;
            };
            let attempt = match key {
                "path" => UnixStream::connect(OsStr::from_bytes(&value)),
                #[cfg(target_os = "linux")]
                "abstract" => {
                    use std::os::linux::net::SocketAddrExt;
                    std::os::unix::net::SocketAddr::from_abstract_name(&value)
                        .and_then(|addr| UnixStream::connect_addr(&addr))
                }
                _ => continue,
            };
            match attempt {
                Ok(stream) => return Ok(stream),
                Err(e) => last = e,
            }
        }
    }
    Err(last)
}

/// Undo the `%xx` escaping a D-Bus address value may carry.
fn unescape(value: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(value.len());
    let mut bytes = value.bytes();
    while let Some(b) = bytes.next() {
        if b == b'%' {
            let hex = [bytes.next()?, bytes.next()?];
            out.push(u8::from_str_radix(std::str::from_utf8(&hex).ok()?, 16).ok()?);
        } else {
            out.push(b);
        }
    }
    Some(out)
}

fn malformed(what: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, format!("D-Bus: {what}"))
}

/// A connection to a message bus, past the handshake.
struct Bus {
    stream: BufReader<UnixStream>,
    /// The serial the next message goes out with. Never zero.
    serial: u32,
}

impl Bus {
    /// Connect, authenticate as the user this process runs as, and say hello
    /// — after which the bus routes messages to and from this connection.
    fn connect(address: &str) -> io::Result<Self> {
        let stream = connect_stream(address)?;
        stream.set_read_timeout(Some(SETUP_TIMEOUT))?;
        stream.set_write_timeout(Some(SETUP_TIMEOUT))?;
        let mut bus = Bus {
            stream: BufReader::new(stream),
            serial: 1,
        };
        // SAFETY: getuid() has no preconditions and cannot fail.
        let uid = unsafe { libc::getuid() };
        let hex: String = uid
            .to_string()
            .bytes()
            .map(|b| format!("{b:02x}"))
            .collect();
        bus.write(format!("\0AUTH EXTERNAL {hex}\r\n").as_bytes())?;
        let mut line = Vec::new();
        bus.stream.read_until(b'\n', &mut line)?;
        if !line.starts_with(b"OK ") {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!(
                    "D-Bus refused us: {}",
                    String::from_utf8_lossy(&line).trim()
                ),
            ));
        }
        bus.write(b"BEGIN\r\n")?;
        bus.call(BUS, BUS_PATH, BUS, "Hello", &[])?;
        Ok(bus)
    }

    fn set_timeout(&mut self, timeout: Option<Duration>) -> io::Result<()> {
        self.stream.get_ref().set_read_timeout(timeout)
    }

    fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.stream.get_mut().write_all(bytes)
    }

    fn next_serial(&mut self) -> u32 {
        let serial = self.serial;
        self.serial = self.serial.checked_add(1).unwrap_or(1);
        serial
    }

    /// Send a method call taking only string arguments; returns its serial,
    /// which the reply will name. The reply itself turns up in [`Self::receive`].
    fn call(
        &mut self,
        destination: &str,
        path: &str,
        interface: &str,
        member: &str,
        args: &[&str],
    ) -> io::Result<u32> {
        let serial = self.next_serial();
        let mut body = Writer::default();
        for arg in args {
            body.string(arg);
        }
        let signature = "s".repeat(args.len());
        let fields = [
            Field::Path(path),
            Field::Destination(destination),
            Field::Interface(interface),
            Field::Member(member),
            Field::Signature(&signature),
        ];
        let message = encode(MessageKind::Call, serial, &fields, &body.0);
        self.write(&message)?;
        Ok(serial)
    }

    /// Read the next message off the bus, whatever it is.
    fn receive(&mut self) -> io::Result<Message> {
        let mut fixed = [0u8; 16];
        self.stream.read_exact(&mut fixed)?;
        let little = match fixed[0] {
            b'l' => true,
            b'B' => false,
            _ => return Err(malformed("unknown byte order")),
        };
        let word = |at: usize| {
            let bytes = [fixed[at], fixed[at + 1], fixed[at + 2], fixed[at + 3]];
            if little {
                u32::from_le_bytes(bytes)
            } else {
                u32::from_be_bytes(bytes)
            }
        };
        // The protocol's own ceiling on a message, which also keeps a
        // corrupt length from allocating the moon.
        if 16 + word(12) as u64 + 7 + word(4) as u64 > 1 << 27 {
            return Err(malformed("message too large"));
        }
        let (body_len, fields_len) = (word(4) as usize, word(12) as usize);
        let padded = fields_len.next_multiple_of(8);
        let mut rest = vec![0u8; padded + body_len];
        self.stream.read_exact(&mut rest)?;

        let mut header = fixed.to_vec();
        header.extend_from_slice(&rest[..fields_len]);
        let mut message = Message {
            kind: MessageKind::from(fixed[1]),
            little,
            serial: word(8),
            reply_serial: None,
            interface: None,
            member: None,
            error_name: None,
            sender: None,
            body: rest.split_off(padded),
        };
        let mut fields = Reader::new(&header, little);
        fields.pos = 16;
        while fields.pos < header.len() {
            fields.align(8)?;
            let code = fields.byte()?;
            let signature = fields.signature()?;
            let value = fields.value(signature.as_bytes())?;
            match (code, value) {
                (2, Value::Str(s)) => message.interface = Some(s),
                (3, Value::Str(s)) => message.member = Some(s),
                (4, Value::Str(s)) => message.error_name = Some(s),
                (5, Value::U32(n)) => message.reply_serial = Some(n),
                (7, Value::Str(s)) => message.sender = Some(s),
                _ => {}
            }
        }
        Ok(message)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MessageKind {
    Call,
    Return,
    Error,
    Signal,
    Unknown,
}

impl From<u8> for MessageKind {
    fn from(code: u8) -> Self {
        match code {
            1 => Self::Call,
            2 => Self::Return,
            3 => Self::Error,
            4 => Self::Signal,
            _ => Self::Unknown,
        }
    }
}

impl MessageKind {
    fn code(self) -> u8 {
        match self {
            Self::Call => 1,
            Self::Return => 2,
            Self::Error => 3,
            Self::Signal => 4,
            Self::Unknown => 0,
        }
    }
}

/// A message read off the bus: the header fields that matter here, and the
/// body still marshalled.
struct Message {
    kind: MessageKind,
    little: bool,
    /// What a reply to this message names. Only the tests' stand-in portal
    /// replies to anything.
    #[cfg_attr(not(test), allow(dead_code))]
    serial: u32,
    reply_serial: Option<u32>,
    interface: Option<String>,
    member: Option<String>,
    error_name: Option<String>,
    /// Who sent it — where a reply goes, so again only for the stand-in.
    #[cfg_attr(not(test), allow(dead_code))]
    sender: Option<String>,
    body: Vec<u8>,
}

impl Message {
    fn is_setting_changed(&self) -> bool {
        self.interface.as_deref() == Some(SETTINGS)
            && self.member.as_deref() == Some("SettingChanged")
    }

    fn body_reader(&self) -> Reader<'_> {
        Reader::new(&self.body, self.little)
    }
}

/// A header field, by the code the protocol gives it.
enum Field<'a> {
    Path(&'a str),
    Interface(&'a str),
    Member(&'a str),
    #[cfg(test)]
    ReplySerial(u32),
    Destination(&'a str),
    Signature(&'a str),
}

/// Marshal a whole message, little-endian: the fixed header, the header
/// fields, padding to eight, and `body` — already marshalled from offset 0,
/// which is 8-aligned in the message, so its alignment holds.
fn encode(kind: MessageKind, serial: u32, fields: &[Field], body: &[u8]) -> Vec<u8> {
    let mut m = Writer::default();
    m.byte(b'l');
    m.byte(kind.code());
    m.byte(0); // flags
    m.byte(1); // protocol version
    m.u32(body.len() as u32);
    m.u32(serial);
    let length_at = m.0.len();
    m.u32(0); // the fields' length, patched in below
    let start = m.0.len();
    for field in fields {
        // An empty signature is the same as none: leave it out, padding and
        // all — the fields' length must end where the last one does.
        if let Field::Signature("") = field {
            continue;
        }
        m.align(8);
        match *field {
            Field::Path(s) => m.field(1, "o", |m| m.string(s)),
            Field::Interface(s) => m.field(2, "s", |m| m.string(s)),
            Field::Member(s) => m.field(3, "s", |m| m.string(s)),
            #[cfg(test)]
            Field::ReplySerial(n) => m.field(5, "u", |m| m.u32(n)),
            Field::Destination(s) => m.field(6, "s", |m| m.string(s)),
            Field::Signature(s) => m.field(8, "g", |m| m.signature(s)),
        }
    }
    let fields_len = (m.0.len() - start) as u32;
    m.0[length_at..length_at + 4].copy_from_slice(&fields_len.to_le_bytes());
    m.align(8);
    m.0.extend_from_slice(body);
    m.0
}

/// Little-endian marshalling into a buffer whose offset 0 is 8-aligned.
#[derive(Default)]
struct Writer(Vec<u8>);

impl Writer {
    fn align(&mut self, to: usize) {
        let len = self.0.len().next_multiple_of(to);
        self.0.resize(len, 0);
    }

    fn byte(&mut self, b: u8) {
        self.0.push(b);
    }

    fn u32(&mut self, n: u32) {
        self.align(4);
        self.0.extend_from_slice(&n.to_le_bytes());
    }

    fn string(&mut self, s: &str) {
        self.u32(s.len() as u32);
        self.0.extend_from_slice(s.as_bytes());
        self.0.push(0);
    }

    fn signature(&mut self, s: &str) {
        self.byte(s.len() as u8);
        self.0.extend_from_slice(s.as_bytes());
        self.0.push(0);
    }

    /// A header field: its code, and its value as a variant of type `sig`.
    fn field(&mut self, code: u8, sig: &str, value: impl FnOnce(&mut Self)) {
        self.byte(code);
        self.signature(sig);
        value(self);
    }
}

/// The values this client has a use for; anything else is an error.
#[derive(Debug, PartialEq)]
enum Value {
    U32(u32),
    Str(String),
}

/// Unmarshalling from a buffer whose offset 0 is 8-aligned in the message.
struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
    little: bool,
}

impl<'a> Reader<'a> {
    fn new(buf: &'a [u8], little: bool) -> Self {
        Self {
            buf,
            pos: 0,
            little,
        }
    }

    fn take(&mut self, n: usize) -> io::Result<&'a [u8]> {
        let end = self
            .pos
            .checked_add(n)
            .filter(|&end| end <= self.buf.len())
            .ok_or_else(|| malformed("truncated message"))?;
        let bytes = &self.buf[self.pos..end];
        self.pos = end;
        Ok(bytes)
    }

    fn align(&mut self, to: usize) -> io::Result<()> {
        let pad = self.pos.next_multiple_of(to) - self.pos;
        self.take(pad).map(drop)
    }

    fn byte(&mut self) -> io::Result<u8> {
        Ok(self.take(1)?[0])
    }

    fn u32(&mut self) -> io::Result<u32> {
        self.align(4)?;
        let b = self.take(4)?;
        let bytes = [b[0], b[1], b[2], b[3]];
        Ok(if self.little {
            u32::from_le_bytes(bytes)
        } else {
            u32::from_be_bytes(bytes)
        })
    }

    /// The text of `len` bytes and the nul after them.
    fn text(&mut self, len: usize) -> io::Result<String> {
        let bytes = self.take(len + 1)?;
        String::from_utf8(bytes[..len].to_vec()).map_err(|_| malformed("string is not UTF-8"))
    }

    fn string(&mut self) -> io::Result<String> {
        let len = self.u32()? as usize;
        self.text(len)
    }

    fn signature(&mut self) -> io::Result<String> {
        let len = self.byte()? as usize;
        self.text(len)
    }

    /// One value of the single complete type `sig`. A variant is looked
    /// through, however deep, to the value inside.
    fn value(&mut self, sig: &[u8]) -> io::Result<Value> {
        match sig {
            // `i` too: a portal has no business sending a color scheme signed,
            // but there is no harm in reading one.
            b"u" | b"i" => self.u32().map(Value::U32),
            b"s" | b"o" => self.string().map(Value::Str),
            b"g" => self.signature().map(Value::Str),
            b"v" => {
                let inner = self.signature()?;
                self.value(inner.as_bytes())
            }
            _ => Err(malformed("unexpected type")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Child, Command, Stdio};
    use std::sync::mpsc;

    /// A method call to the bus, byte for byte as the D-Bus specification lays
    /// it out — the reference every other test leans on.
    #[test]
    fn a_method_call_is_marshalled_to_the_letter() {
        let fields = [
            Field::Path("/a"),
            Field::Destination("d.e"),
            Field::Interface("i.f"),
            Field::Member("M"),
            Field::Signature("s"),
        ];
        let mut body = Writer::default();
        body.string("xy");
        let bytes = encode(MessageKind::Call, 7, &fields, &body.0);
        #[rustfmt::skip]
        let expected: &[u8] = &[
            b'l', 1, 0, 1,  7, 0, 0, 0,  7, 0, 0, 0,  71, 0, 0, 0,
            // PATH: code, signature "o", then the string 4-aligned.
            1, 1, b'o', 0,  2, 0, 0, 0,  b'/', b'a', 0, 0, 0, 0, 0, 0,
            // DESTINATION, 8-aligned like every field.
            6, 1, b's', 0,  3, 0, 0, 0,  b'd', b'.', b'e', 0, 0, 0, 0, 0,
            2, 1, b's', 0,  3, 0, 0, 0,  b'i', b'.', b'f', 0, 0, 0, 0, 0,
            3, 1, b's', 0,  1, 0, 0, 0,  b'M', 0, 0, 0, 0, 0, 0, 0,
            // SIGNATURE ends the fields 71 bytes in — the padding between
            // them counts, the padding after the last doesn't. The body starts
            // 8-aligned.
            8, 1, b'g', 0,  1, b's', 0, 0,
            2, 0, 0, 0,  b'x', b'y', 0,
        ];
        assert_eq!(bytes, expected);
    }

    /// `Hello` takes no arguments, so no signature field: the fields end with
    /// the member's name, and the length says so — padding after it would
    /// read as another field, and the bus hangs up on the garbage.
    #[test]
    fn a_call_without_arguments_ends_its_fields_at_the_last_one() {
        let fields = [
            Field::Path(BUS_PATH),
            Field::Destination(BUS),
            Field::Interface(BUS),
            Field::Member("Hello"),
            Field::Signature(""),
        ];
        let bytes = encode(MessageKind::Call, 1, &fields, &[]);
        let fields_len = u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]) as usize;
        assert!(bytes[..16 + fields_len].ends_with(b"Hello\0"));
        assert_eq!(bytes.len(), (16 + fields_len).next_multiple_of(8));
    }

    #[test]
    fn a_double_wrapped_value_is_unwrapped() {
        // v(v(u 1)), as `Read` answers — big-endian for good measure.
        let body: &[u8] = &[1, b'v', 0, 1, b'u', 0, 0, 0, 0, 0, 0, 1];
        assert_eq!(
            scheme(&mut Reader::new(body, false)).unwrap(),
            Appearance::Dark
        );
        // v(u 2), as `ReadOne` would.
        let body: &[u8] = &[1, b'u', 0, 0, 2, 0, 0, 0];
        assert_eq!(
            scheme(&mut Reader::new(body, true)).unwrap(),
            Appearance::Light
        );
        // 0 is "no preference", which is light.
        let body: &[u8] = &[1, b'u', 0, 0, 0, 0, 0, 0];
        assert_eq!(
            scheme(&mut Reader::new(body, true)).unwrap(),
            Appearance::Light
        );
    }

    #[test]
    fn a_truncated_or_foreign_value_is_an_error() {
        assert!(scheme(&mut Reader::new(&[1, b'u', 0, 0, 1], true)).is_err());
        assert!(
            scheme(&mut Reader::new(
                &[1, b's', 0, 0, 1, 0, 0, 0, b'x', 0],
                true
            ))
            .is_err()
        );
        assert!(
            scheme(&mut Reader::new(
                &[5, b'a', b'{', b's', b'v', b'}', 0],
                true
            ))
            .is_err()
        );
    }

    #[test]
    fn addresses_are_unescaped() {
        assert_eq!(
            unescape("/run/user/1000/bus").unwrap(),
            b"/run/user/1000/bus"
        );
        assert_eq!(unescape("/tmp/a%2cb%3D").unwrap(), b"/tmp/a,b=");
        assert_eq!(unescape("/tmp/%2"), None);
        assert_eq!(unescape("/tmp/%zz"), None);
    }

    /// The first address that connects wins; ones of other transports, or
    /// ones that refuse, are passed over.
    #[test]
    fn the_first_address_that_answers_is_used() {
        let dir = TempDir::new("saudade-addr");
        let socket = dir.0.join("s");
        let _listener = std::os::unix::net::UnixListener::bind(&socket).expect("bind");
        let missing = dir.0.join("missing");
        let addresses = format!(
            "tcp:host=localhost,port=1;unix:path={};unix:path={},guid=0",
            missing.display(),
            socket.display(),
        );
        assert!(connect_stream(&addresses).is_ok());
        assert!(connect_stream(&format!("unix:path={}", missing.display())).is_err());
    }

    /// Session buses started outside systemd often listen on an abstract
    /// socket instead of a path.
    #[cfg(target_os = "linux")]
    #[test]
    fn an_abstract_address_connects() {
        use std::os::linux::net::SocketAddrExt;
        use std::os::unix::net::{SocketAddr, UnixListener};
        let name = format!("saudade-test-{}", std::process::id());
        let addr = SocketAddr::from_abstract_name(name.as_bytes()).expect("abstract name");
        let _listener = UnixListener::bind_addr(&addr).expect("bind");
        assert!(connect_stream(&format!("unix:abstract={name},guid=0")).is_ok());
    }

    #[test]
    fn the_first_answer_goes_to_the_waiter_and_later_ones_are_changes() {
        let first = FirstAnswer::default();
        assert!(first.offer(Appearance::Dark));
        assert_eq!(first.wait(Duration::ZERO), Some(Appearance::Dark));
        assert!(!first.offer(Appearance::Light));
    }

    #[test]
    fn an_answer_after_the_waiter_left_is_a_change() {
        let first = FirstAnswer::default();
        assert_eq!(first.wait(Duration::from_millis(1)), None);
        assert!(!first.offer(Appearance::Dark));
    }

    #[test]
    fn a_failed_watch_releases_the_waiter_at_once() {
        let first = Arc::new(FirstAnswer::default());
        let failing = first.clone();
        std::thread::spawn(move || failing.give_up());
        let started = std::time::Instant::now();
        assert_eq!(first.wait(Duration::from_secs(10)), None);
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    /// A private `dbus-daemon`, for as long as the value lives. `None` where
    /// none is installed — the end-to-end tests below then have nothing to
    /// talk to and pass vacuously.
    struct Daemon {
        child: Child,
        address: String,
        _dir: TempDir,
    }

    impl Daemon {
        fn start() -> Option<Self> {
            let dir = TempDir::new("saudade-dbus");
            let socket = dir.0.join("bus");
            let config = dir.0.join("bus.conf");
            std::fs::write(
                &config,
                format!(
                    r#"<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>session</type>
  <listen>unix:path={}</listen>
  <auth>EXTERNAL</auth>
  <policy context="default">
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow own="*"/>
  </policy>
</busconfig>"#,
                    socket.display()
                ),
            )
            .ok()?;
            let mut child = Command::new("dbus-daemon")
                .arg(format!("--config-file={}", config.display()))
                .args(["--nofork", "--print-address"])
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .ok()?;
            // It prints the address once it is listening.
            let mut line = String::new();
            BufReader::new(child.stdout.take()?)
                .read_line(&mut line)
                .ok()?;
            let address = line.trim().to_string();
            if address.is_empty() {
                let _ = child.kill();
                return None;
            }
            Some(Self {
                child,
                address,
                _dir: dir,
            })
        }
    }

    impl Drop for Daemon {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(prefix: &str) -> Self {
            use std::sync::atomic::{AtomicU32, Ordering};
            static N: AtomicU32 = AtomicU32::new(0);
            let n = N.fetch_add(1, Ordering::Relaxed);
            // Short, since a socket path has to fit in sun_path.
            let dir = std::env::temp_dir().join(format!("{prefix}-{}-{n}", std::process::id()));
            std::fs::create_dir_all(&dir).expect("temp dir");
            Self(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A stand-in for the portal on the test bus: owns its name, answers
    /// `Read` with `scheme` the way real portals do — double-wrapped — and
    /// emits `SettingChanged` for every scheme it is sent afterwards.
    fn fake_portal(
        address: String,
        scheme: u32,
        changes: mpsc::Receiver<u32>,
    ) -> mpsc::Receiver<()> {
        let (ready_tx, ready) = mpsc::channel();
        std::thread::spawn(move || {
            let mut bus = Bus::connect(&address).expect("portal connects");
            // RequestName takes (su), which `Bus::call` cannot marshal: the
            // flags are a number. So by hand.
            let serial = bus.next_serial();
            let mut body = Writer::default();
            body.string(PORTAL);
            body.u32(0);
            let fields = [
                Field::Path(BUS_PATH),
                Field::Destination(BUS),
                Field::Interface(BUS),
                Field::Member("RequestName"),
                Field::Signature("su"),
            ];
            bus.write(&encode(MessageKind::Call, serial, &fields, &body.0))
                .expect("send RequestName");
            loop {
                let m = bus.receive().expect("portal receives");
                if m.reply_serial == Some(serial) {
                    assert_eq!(m.kind, MessageKind::Return, "the portal owns its name");
                    break;
                }
            }
            ready_tx.send(()).expect("test is waiting");
            bus.set_timeout(Some(Duration::from_millis(20)))
                .expect("timeout");

            loop {
                if let Ok(scheme) = changes.try_recv() {
                    let mut body = Writer::default();
                    body.string(NAMESPACE);
                    body.string(KEY);
                    body.signature("u");
                    body.u32(scheme);
                    let serial = bus.next_serial();
                    let fields = [
                        Field::Path(PORTAL_PATH),
                        Field::Interface(SETTINGS),
                        Field::Member("SettingChanged"),
                        Field::Signature("ssv"),
                    ];
                    bus.write(&encode(MessageKind::Signal, serial, &fields, &body.0))
                        .expect("emit SettingChanged");
                }
                let m = match bus.receive() {
                    Ok(m) => m,
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => continue,
                    Err(e) if e.kind() == io::ErrorKind::TimedOut => continue,
                    Err(_) => return,
                };
                if m.kind != MessageKind::Call || m.member.as_deref() != Some("Read") {
                    continue;
                }
                let mut args = m.body_reader();
                assert_eq!(args.string().unwrap(), NAMESPACE);
                assert_eq!(args.string().unwrap(), KEY);
                let mut body = Writer::default();
                body.signature("v");
                body.signature("u");
                body.u32(scheme);
                let serial = bus.next_serial();
                let sender = m.sender.clone().expect("the bus names the caller");
                let fields = [
                    Field::ReplySerial(m.serial),
                    Field::Destination(&sender),
                    Field::Signature("v"),
                ];
                bus.write(&encode(MessageKind::Return, serial, &fields, &body.0))
                    .expect("reply to Read");
            }
        });
        ready
    }

    #[test]
    fn the_scheme_and_its_changes_come_through_a_real_bus() {
        let Some(daemon) = Daemon::start() else {
            eprintln!("no dbus-daemon to test against; skipping");
            return;
        };
        let (change_tx, change_rx) = mpsc::channel();
        let ready = fake_portal(daemon.address.clone(), 1, change_rx);
        ready
            .recv_timeout(Duration::from_secs(5))
            .expect("the portal comes up");

        let (seen_tx, seen) = mpsc::channel();
        let address = daemon.address.clone();
        std::thread::spawn(move || {
            let _ = listen(&address, |appearance| seen_tx.send(appearance).is_ok());
        });
        let wait = || {
            seen.recv_timeout(Duration::from_secs(5))
                .expect("an answer")
        };
        assert_eq!(wait(), Appearance::Dark, "Read says 1");

        change_tx.send(2).unwrap();
        assert_eq!(wait(), Appearance::Light, "a change to 2");
        change_tx.send(1).unwrap();
        assert_eq!(wait(), Appearance::Dark, "and back to 1");
    }

    #[test]
    fn no_portal_on_the_bus_ends_the_watch() {
        let Some(daemon) = Daemon::start() else {
            eprintln!("no dbus-daemon to test against; skipping");
            return;
        };
        let result = listen(&daemon.address, |_| panic!("nothing to report"));
        let error = result.expect_err("no portal answers");
        assert!(error.to_string().contains("ServiceUnknown"), "{error}");
    }
}
