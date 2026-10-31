//! What a drag carries on Wayland and X11, and how a drop reads it.
//!
//! Both name the forms a drag comes in by MIME type — XDND by atoms of those
//! names — and both read a drop one form at a time, off a source that writes
//! each out when asked. The two backends share which forms a drop reads, in
//! which order, and what [`DragData`] they come to.
//!
//! Addresses come from the `text/uri-list` file managers and web browsers
//! offer, failing that from the `text/x-moz-url` or `_NETSCAPE_URL` a browser
//! offers a link as; the `file:` ones are the drag's paths. A drag with no
//! paths is then read for a file by its contents: an image dragged out of a
//! web browser comes as `image/png` and other image types from Firefox, and as
//! `application/octet-stream` from Chromium on X11. Such a file is called what
//! the source suggests (X11's `XdndDirectSave0`), else what the drag's address
//! ends in, with the extension of the image type it was read as.

use std::path::PathBuf;

use crate::event::{DragData, DraggedFile};

/// The MIME type that carries a newline-separated list of dragged URIs. It's
/// the de-facto standard every file manager (Nautilus, Dolphin, Thunar, …)
/// offers for dragged files, and web browsers for links.
pub(crate) const URI_LIST: &str = "text/uri-list";

/// A link as Firefox and Chromium drag it: UTF-16, the address and its title
/// on two lines.
const MOZ_URL: &str = "text/x-moz-url";

/// A link as Netscape dragged it, and browsers on X11 still do: the address
/// and its title on two lines.
const NETSCAPE_URL: &str = "_NETSCAPE_URL";

/// A file's bytes, of no type in particular: an image Chromium drags on X11.
const OCTET_STREAM: &str = "application/octet-stream";

/// The form a drag offering `types` is read in first, `None` when it carries
/// nothing a drop reads. What a Wayland target accepts the drag with.
pub(crate) fn first(types: &[String]) -> Option<&str> {
    address_form(types).or_else(|| contents_form(types))
}

/// The form a drag's addresses are read in.
fn address_form(types: &[String]) -> Option<&str> {
    [URI_LIST, MOZ_URL, NETSCAPE_URL]
        .into_iter()
        .find_map(|form| offered(types, |t| t == form))
}

/// The form a file the drag carries by its contents is read in: PNG where the
/// source offers it — Firefox offers an image in every type it can convert it
/// to — else another image type, else bytes of no type in particular.
fn contents_form(types: &[String]) -> Option<&str> {
    offered(types, |t| t == "image/png")
        .or_else(|| offered(types, |t| t.starts_with("image/")))
        .or_else(|| offered(types, |t| t == OCTET_STREAM))
}

fn offered(types: &[String], wanted: impl Fn(&str) -> bool) -> Option<&str> {
    types.iter().map(String::as_str).find(|t| wanted(t))
}

/// A drop being read: the forms to ask the source for, one after another
/// ([`Self::next`]), and what they come to ([`Self::finish`]).
pub(crate) struct Reading {
    types: Vec<String>,
    step: Step,
    data: DragData,
    /// The file the drag carries by its contents: the form it was read in, and
    /// its bytes.
    contents: Option<(String, Vec<u8>)>,
    /// The name the source suggests for that file.
    name: Option<String>,
}

#[derive(Clone, Copy)]
enum Step {
    Addresses,
    Contents,
    Done,
}

impl Reading {
    /// A drop of a drag offering `types`.
    pub(crate) fn new(types: Vec<String>) -> Self {
        Self {
            types,
            step: Step::Addresses,
            data: DragData::default(),
            contents: None,
            name: None,
        }
    }

    /// Call a file the drag carries by its contents `name`, as the source
    /// suggests.
    pub(crate) fn suggest_name(&mut self, name: &str) {
        let name = name.trim_matches(|c: char| c == '\0' || c.is_whitespace());
        if !name.is_empty() {
            self.name = Some(name.to_string());
        }
    }

    /// The next form to ask the source for, `None` once there is none left.
    /// Contents are asked for only when the addresses held no paths, which are
    /// the same files then, and the cheaper to read.
    pub(crate) fn next(&mut self) -> Option<String> {
        loop {
            let form = match self.step {
                Step::Addresses => {
                    self.step = Step::Contents;
                    address_form(&self.types)
                }
                Step::Contents => {
                    self.step = Step::Done;
                    contents_form(&self.types).filter(|_| self.data.paths.is_empty())
                }
                Step::Done => return None,
            };
            if let Some(form) = form {
                return Some(form.to_string());
            }
        }
    }

    /// Take in what the source wrote out as `form`.
    pub(crate) fn take(&mut self, form: &str, bytes: Vec<u8>) {
        match form {
            URI_LIST => {
                let list = parse_uri_list(&String::from_utf8_lossy(&bytes));
                self.data.paths.extend(list.paths);
                self.data.urls.extend(list.urls);
            }
            MOZ_URL | NETSCAPE_URL => self.data.urls.extend(link(&bytes)),
            _ => self.contents = Some((form.to_string(), bytes)),
        }
    }

    /// Everything the drop carried.
    pub(crate) fn finish(mut self) -> DragData {
        if let Some((form, contents)) = self.contents.take().filter(|(_, c)| !c.is_empty()) {
            let url = self.data.urls.first().map(String::as_str);
            let name = file_name(self.name.as_deref(), url, &form);
            self.data.files.push(DraggedFile { name, contents });
        }
        self.data
    }
}

/// The address of a link dragged as `text/x-moz-url` or `_NETSCAPE_URL`: the
/// first line. The former is UTF-16, which browsers write in the machine's
/// byte order — little-endian on every machine they run on.
fn link(bytes: &[u8]) -> Option<String> {
    let utf16 = bytes.starts_with(&[0xff, 0xfe]) || bytes.get(1) == Some(&0);
    let text = if utf16 {
        let units: Vec<u16> = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&c| u16::from_le_bytes(c))
            .collect();
        String::from_utf16_lossy(&units)
    } else {
        String::from_utf8_lossy(bytes).into_owned()
    };
    let url = text
        .trim_start_matches('\u{feff}')
        .lines()
        .next()?
        .trim_matches(|c: char| c == '\0' || c.is_whitespace());
    (!url.is_empty()).then(|| url.to_string())
}

/// What a file the drag carries by its contents, read as `form`, is called:
/// what the source suggests, else what the drag's address ends in, else
/// `image` or `file` — with the extension of the image type it was read as.
fn file_name(suggested: Option<&str>, url: Option<&str>, form: &str) -> String {
    let name = suggested
        .map(str::to_string)
        .or_else(|| url.and_then(url_file_name))
        .unwrap_or_else(|| {
            let image = form.starts_with("image/");
            if image { "image" } else { "file" }.to_string()
        });
    match image_extension(form) {
        Some(ext) => with_extension(name, ext),
        None => name,
    }
}

/// The last part of a URL's path, as a file name: `cat.png` for
/// `https://example.com/img/cat.png?w=200`. `None` for an address with no
/// path to it, a `data:` one say.
fn url_file_name(url: &str) -> Option<String> {
    let (_, rest) = url.split_once("://")?;
    let path = rest.split(['?', '#']).next()?;
    let (_, last) = path.rsplit_once('/')?;
    let name = percent_decode(last);
    (!name.is_empty()).then_some(name)
}

/// The extension an image of type `form` goes by: `png` for `image/png`, `jpg`
/// for `image/jpeg`, `svg` for `image/svg+xml`. `None` for what isn't an
/// image.
fn image_extension(form: &str) -> Option<&str> {
    let subtype = form.strip_prefix("image/")?;
    let subtype = subtype.strip_prefix("x-").unwrap_or(subtype);
    let subtype = subtype.split(['+', ';']).next().unwrap_or(subtype);
    Some(match subtype {
        "jpeg" => "jpg",
        "icon" | "vnd.microsoft.icon" => "ico",
        other => other,
    })
}

/// `name` with the extension `ext`, in place of the one it has: an image a
/// browser converted to PNG is no longer the JPEG its address names. One that
/// only spells it differently stays, `.jpeg` for `jpg`.
fn with_extension(name: String, ext: &str) -> String {
    let (stem, current) = match name.rsplit_once('.') {
        Some((stem, current)) if !stem.is_empty() => (stem, current.to_ascii_lowercase()),
        _ => (name.as_str(), String::new()),
    };
    let same = current == ext
        || matches!(
            (ext, current.as_str()),
            ("jpg", "jpeg" | "jpe") | ("tiff", "tif")
        );
    if same {
        return name;
    }
    format!("{stem}.{ext}")
}

/// Parse an RFC 2483 `text/uri-list` payload into the local file paths it
/// references and the other addresses it carries, such as a link dragged out
/// of a web browser. Blank lines and comments (`#…`) are skipped.
pub(crate) fn parse_uri_list(text: &str) -> DragData {
    let mut data = DragData::default();
    let uris = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'));
    for uri in uris {
        match file_uri_to_path(uri) {
            Some(path) => data.paths.push(path),
            None if !uri.starts_with("file:") => data.urls.push(uri.to_string()),
            None => {}
        }
    }
    data
}

/// Turn a single `file://[host]/path` URI into a [`PathBuf`], percent-decoding
/// the path. Returns `None` for URIs that aren't `file:` (we can't open a
/// remote resource as a path).
fn file_uri_to_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    // Drop an optional authority (host) component: the path starts at the
    // first '/'. `file:///path` → empty authority, path "/path";
    // `file://host/path` → authority "host", path "/path".
    let path = &rest[rest.find('/')?..];
    Some(PathBuf::from(percent_decode(path)))
}

/// Decode `%XX` escapes in a URI path into raw bytes, then read the result as
/// UTF-8 (lossily — non-UTF-8 paths are rare and not worth refusing the drop).
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = (bytes[i + 1] as char).to_digit(16);
            let lo = (bytes[i + 2] as char).to_digit(16);
            if let (Some(h), Some(l)) = (hi, lo) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn types(names: &[&str]) -> Vec<String> {
        names.iter().map(|n| n.to_string()).collect()
    }

    /// Read a drop the way a backend does, the source writing out `forms`.
    fn read(offered: &[&str], forms: &[(&str, &[u8])]) -> (Vec<String>, DragData) {
        let mut reading = Reading::new(types(offered));
        let mut asked = Vec::new();
        while let Some(form) = reading.next() {
            if let Some((_, bytes)) = forms.iter().find(|(f, _)| *f == form) {
                reading.take(&form, bytes.to_vec());
            }
            asked.push(form);
        }
        (asked, reading.finish())
    }

    #[test]
    fn parses_a_uri_list_into_paths() {
        // Real file managers terminate lines with CRLF and may include a
        // leading comment line; both must be tolerated.
        let payload = "#comment\r\nfile:///home/rob/a.txt\r\nfile:///tmp/b.png\r\n";
        assert_eq!(
            parse_uri_list(payload).paths,
            vec![
                PathBuf::from("/home/rob/a.txt"),
                PathBuf::from("/tmp/b.png"),
            ]
        );
    }

    #[test]
    fn percent_escapes_are_decoded() {
        assert_eq!(
            file_uri_to_path("file:///tmp/a%20b%2Bc.txt"),
            Some(PathBuf::from("/tmp/a b+c.txt"))
        );
        assert_eq!(percent_decode("%41%42"), "AB");
        // A stray, truncated escape is left verbatim rather than panicking.
        assert_eq!(percent_decode("100%"), "100%");
    }

    #[test]
    fn drops_authority_and_non_file_uris() {
        assert_eq!(
            file_uri_to_path("file://localhost/etc/hosts"),
            Some(PathBuf::from("/etc/hosts"))
        );
        assert_eq!(file_uri_to_path("https://example.com/x"), None);
        assert!(parse_uri_list("https://example.com/x\n\n").paths.is_empty());
    }

    #[test]
    fn keeps_web_addresses_apart_from_files() {
        // A link or an image dragged out of a web browser.
        let data = parse_uri_list("https://example.com/a.png\r\nfile:///tmp/b.png\r\n");
        assert_eq!(data.urls, ["https://example.com/a.png"]);
        assert_eq!(data.paths, [PathBuf::from("/tmp/b.png")]);
        // A `file:` URI that names no path is neither.
        assert!(parse_uri_list("file:relative").is_empty());
    }

    #[test]
    fn takes_only_drags_it_can_read() {
        let files = types(&["x-special/gnome-copied-files", "text/uri-list"]);
        assert_eq!(first(&files), Some("text/uri-list"));
        let image = types(&["text/html", "image/jpeg", "image/png"]);
        assert_eq!(first(&image), Some("image/png"));
        assert_eq!(first(&types(&["text/plain", "UTF8_STRING"])), None);
        assert_eq!(first(&[]), None);
    }

    #[test]
    fn reads_files_by_their_paths_only() {
        // The image types are the same files: not read when they have paths.
        let (asked, data) = read(
            &["text/uri-list", "image/png"],
            &[("text/uri-list", b"file:///tmp/a.png\r\n")],
        );
        assert_eq!(asked, ["text/uri-list"]);
        assert_eq!(data.paths, [PathBuf::from("/tmp/a.png")]);
        assert!(data.files.is_empty() && data.urls.is_empty());
    }

    #[test]
    fn reads_an_image_dragged_out_of_firefox() {
        // Its address, and the image in every type Firefox can convert it to,
        // PNG among them. Named after the address, as the PNG it now is.
        let (asked, data) = read(
            &[
                "text/x-moz-url",
                "text/uri-list",
                "image/jpeg",
                "image/png",
                "image/bmp",
            ],
            &[
                ("text/uri-list", b"https://example.com/img/cat.jpg?w=2\r\n"),
                ("image/png", b"\x89PNG"),
            ],
        );
        assert_eq!(asked, ["text/uri-list", "image/png"]);
        assert_eq!(data.urls, ["https://example.com/img/cat.jpg?w=2"]);
        assert_eq!(
            data.files,
            [DraggedFile {
                name: "cat.png".into(),
                contents: b"\x89PNG".to_vec(),
            }]
        );
    }

    #[test]
    fn reads_an_image_dragged_out_of_chromium_on_x11() {
        // Its bytes as they were, under the name the source suggests.
        let mut reading = Reading::new(types(&[
            "_NETSCAPE_URL",
            "text/x-moz-url",
            "XdndDirectSave0",
            "application/octet-stream",
        ]));
        reading.suggest_name("photo.webp\0");
        assert_eq!(reading.next().as_deref(), Some("text/x-moz-url"));
        let moz: Vec<u8> = "https://example.com/p\nA photo"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        reading.take("text/x-moz-url", moz);
        assert_eq!(reading.next().as_deref(), Some("application/octet-stream"));
        reading.take("application/octet-stream", b"RIFF".to_vec());
        assert_eq!(reading.next(), None);
        let data = reading.finish();
        assert_eq!(data.urls, ["https://example.com/p"]);
        assert_eq!(data.files[0].name, "photo.webp");
    }

    #[test]
    fn reads_links_in_whatever_form_they_come() {
        assert_eq!(
            link(b"https://example.com/\nExample").as_deref(),
            Some("https://example.com/")
        );
        let with_bom: Vec<u8> = "\u{feff}https://example.com/a\r\ntitle\0"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        assert_eq!(link(&with_bom).as_deref(), Some("https://example.com/a"));
        assert_eq!(link(b""), None);
    }

    #[test]
    fn a_source_that_writes_nothing_leaves_nothing() {
        let (asked, data) = read(&["image/png"], &[("image/png", b"")]);
        assert_eq!(asked, ["image/png"]);
        assert!(data.is_empty());
    }

    #[test]
    fn names_files_by_what_there_is_to_go_by() {
        let url = Some("https://example.com/a%20b/Gr%C3%B6%C3%9Fe.JPEG#top");
        assert_eq!(file_name(None, url, "image/jpeg"), "Größe.JPEG");
        assert_eq!(file_name(None, url, "image/svg+xml"), "Größe.svg");
        assert_eq!(
            file_name(None, Some("data:image/png;base64,AA"), "image/png"),
            "image.png"
        );
        assert_eq!(
            file_name(None, Some("https://example.com/"), "image/x-icon"),
            "image.ico"
        );
        assert_eq!(file_name(None, None, OCTET_STREAM), "file");
        assert_eq!(file_name(Some("a.tar.gz"), url, OCTET_STREAM), "a.tar.gz");
        assert_eq!(file_name(Some(".hidden"), None, "image/png"), ".hidden.png");
    }
}
