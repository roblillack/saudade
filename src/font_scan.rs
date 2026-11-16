//! The host's installed fonts, as fontdb finds them, kept in an index on disk
//! so that finding them again costs a few milliseconds.
//!
//! fontdb's `load_system_fonts` opens every file in the system's font
//! directories and reads its names. On a Linux desktop with the Noto fonts that
//! is 2,700 files and 60 ms, at every start of every app, for an answer that
//! almost never changes. So the answer is kept in an index, one entry per
//! directory, and an entry is trusted for as long as its directory's
//! modification time stays the same: adding, removing or renaming a font
//! changes it. Writing over a font file in place doesn't, the same blind spot
//! fontconfig's own cache has. A directory that did change is read again, and
//! the files that have to be parsed are spread across the CPUs.
//!
//! The database comes out with the same faces in the same order that
//! `load_system_fonts` would have given, and order matters: of two equally
//! good matches, [`fontdb::Database::query`] takes the first. One thing is
//! dropped on the way through the index, the language each family name is
//! given in, which nothing here reads.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// The index's file name. The number goes up whenever the format changes, or
/// what fontdb reads out of a font file does (a fontdb upgrade, that is), so
/// that an old index is never read as a new one.
const INDEX_FILE: &str = "fonts-1.idx";

/// The first bytes of an index file.
const MAGIC: &[u8] = b"saudade font index 1\n";

/// How long after it was modified a directory has to have been read for the
/// index to trust what was read. A file system stamps a change with a clock
/// that may only tick every few milliseconds — every two seconds, on FAT — so
/// a font added just after a directory was read can leave it with the very
/// modification time it was read at. A directory read sooner than this after a
/// change is read again next time, by which time its time can be trusted.
const SETTLE: Duration = Duration::from_secs(2);

/// The installed fonts, ready to be queried.
pub(crate) fn system_fonts() -> fontdb::Database {
    scan(&roots(), index_path().as_deref(), SystemTime::now()).0
}

/// What a [`scan`] had to do, for the tests.
#[derive(Debug, Default, PartialEq)]
struct Work {
    /// Directories read, because the index had nothing for them or what it had
    /// was out of date.
    dirs_read: usize,
    /// Font files parsed.
    files_parsed: usize,
}

/// The fonts in `roots` and everything below them, the way fontdb's
/// `load_fonts_dir` would read them one root after the other, by way of the
/// index at `index` when there is one. `now` is the time, which decides
/// whether a directory has [settled](SETTLE).
fn scan(roots: &[PathBuf], index: Option<&Path>, now: SystemTime) -> (fontdb::Database, Work) {
    let mut scan = Scan {
        known: index.and_then(read_index).unwrap_or_default(),
        settled_before: nanos(now - SETTLE),
        ..Scan::default()
    };
    for root in roots {
        scan.visit(root);
    }
    scan.parse_files();
    // fontdb's fontconfig loader hardcodes /etc/fonts/fonts.conf, but the
    // FreeBSD port installs fontconfig at /usr/local/etc/fonts/fonts.conf, so
    // on a stock FreeBSD desktop the fontconfig directories come out empty.
    // Fall back to the conventional ports font directory.
    let mut roots = roots.to_vec();
    if !scan.has_faces() {
        let fallback = PathBuf::from("/usr/local/share/fonts");
        scan.visit(&fallback);
        scan.parse_files();
        roots.push(fallback);
    }

    // Anything left in `known` is a directory that is gone, or no longer below
    // any root: the index changes by losing it.
    if let Some(index) = index
        && (scan.work.dirs_read > 0 || !scan.known.is_empty())
    {
        write_index(index, &scan.dirs);
    }

    let mut db = fontdb::Database::new();
    let mut seen = HashSet::new();
    for root in &roots {
        scan.fill(&mut db, root, &mut seen);
    }
    (db, scan.work)
}

#[derive(Default)]
struct Scan {
    /// The directories as the index has them, taken out as they are visited.
    known: HashMap<PathBuf, Dir>,
    /// The directories as they are now.
    dirs: HashMap<PathBuf, Dir>,
    /// The directories visited so far, by their real path, so that one reached
    /// a second way (a symbolic link) isn't read twice, nor a link loop forever.
    seen: HashSet<PathBuf>,
    /// The modification time a directory read now must be older than for the
    /// index to trust it, as nanoseconds since the epoch.
    settled_before: u128,
    work: Work,
}

/// One font directory, as it was when it was last read.
struct Dir {
    /// Its modification time, as nanoseconds since the epoch. Zero when it was
    /// read too soon after it changed to be trusted, which no directory's time
    /// matches.
    mtime: u128,
    entries: Vec<Entry>,
}

enum Entry {
    /// A font file, and its faces once it has been parsed. A file that fontdb
    /// can't read has none.
    File(PathBuf, Option<Vec<fontdb::FaceInfo>>),
    /// A directory inside it.
    Dir(PathBuf),
}

impl Scan {
    /// Bring `dir` and the directories in it up to date: from the index when
    /// it has them as they are, by reading them otherwise.
    fn visit(&mut self, dir: &Path) {
        let Ok(real) = std::fs::canonicalize(dir) else {
            return;
        };
        if !self.seen.insert(real) {
            return;
        }
        // The time is taken before the directory is read, so that a change
        // made while it is being read leaves the index out of date (and the
        // directory read again next time) rather than missing the change.
        let Ok(mtime) = std::fs::metadata(dir).and_then(|meta| meta.modified()) else {
            return;
        };
        let mut mtime = nanos(mtime);

        let entries = match self.known.remove(dir) {
            Some(known) if known.mtime == mtime && mtime != 0 => known.entries,
            _ => {
                self.work.dirs_read += 1;
                if mtime >= self.settled_before {
                    mtime = 0;
                }
                read_dir(dir)
            }
        };
        for entry in &entries {
            if let Entry::Dir(sub) = entry {
                self.visit(sub);
            }
        }
        self.dirs.insert(dir.to_owned(), Dir { mtime, entries });
    }

    /// Whether any font file found so far has a face.
    fn has_faces(&self) -> bool {
        self.dirs.values().any(|dir| {
            dir.entries
                .iter()
                .any(|entry| matches!(entry, Entry::File(_, Some(faces)) if !faces.is_empty()))
        })
    }

    /// Parse the font files no one has parsed yet, on as many threads as it
    /// takes.
    fn parse_files(&mut self) {
        let mut files: Vec<_> = self
            .dirs
            .values_mut()
            .flat_map(|dir| dir.entries.iter_mut())
            .filter_map(|entry| match entry {
                Entry::File(path, faces @ None) => Some((path.as_path(), faces)),
                _ => None,
            })
            .collect();
        self.work.files_parsed += files.len();
        if files.is_empty() {
            return;
        }
        // A thread for every 64 files at most: a few files aren't worth
        // starting one for.
        let threads = std::thread::available_parallelism()
            .map_or(1, |n| n.get())
            .min(files.len().div_ceil(64));
        let per_thread = files.len().div_ceil(threads);
        std::thread::scope(|scope| {
            for chunk in files.chunks_mut(per_thread) {
                scope.spawn(move || {
                    for (path, faces) in chunk {
                        **faces = Some(parse_file(path));
                    }
                });
            }
        });
    }

    /// Add the faces in `dir`, and the directories in it, to `db` in the order
    /// fontdb's own directory loader would.
    fn fill(&self, db: &mut fontdb::Database, dir: &Path, seen: &mut HashSet<PathBuf>) {
        if !seen.insert(dir.to_owned()) {
            return;
        }
        let Some(found) = self.dirs.get(dir) else {
            return;
        };
        for entry in &found.entries {
            match entry {
                Entry::File(_, faces) => {
                    for face in faces.iter().flatten() {
                        db.push_face_info(face.clone());
                    }
                }
                Entry::Dir(sub) => self.fill(db, sub, seen),
            }
        }
    }
}

/// `time` as nanoseconds since the epoch, or zero for a time before it.
fn nanos(time: SystemTime) -> u128 {
    time.duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_nanos())
}

/// What's in `dir`, in the order the system lists it: the font files fontdb
/// would load (not parsed yet), and the directories it would go into.
fn read_dir(dir: &Path) -> Vec<Entry> {
    let Ok(listing) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut entries = Vec::new();
    for entry in listing.flatten() {
        let path = entry.path();
        // Like fontdb, follow a symbolic link to see what it points at.
        let Ok(mut kind) = entry.file_type() else {
            continue;
        };
        if kind.is_symlink() {
            match std::fs::metadata(&path) {
                Ok(meta) => kind = meta.file_type(),
                Err(_) => continue,
            }
        }
        if kind.is_dir() {
            entries.push(Entry::Dir(path));
        } else if kind.is_file() && is_font_file(&path) {
            entries.push(Entry::File(path, None));
        }
    }
    entries
}

/// Whether fontdb's directory loader would load `path`, which goes by the
/// file's extension alone.
fn is_font_file(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|ext| ext.to_str()),
        Some("ttf" | "ttc" | "TTF" | "TTC" | "otf" | "otc" | "OTF" | "OTC")
    )
}

/// The magic number a font collection starts with, `ttcf`.
const COLLECTION: u32 = 0x7474_6366;

/// The faces in the font file at `path`, as fontdb reads them.
///
/// fontdb maps a file into memory to read it, and taking a mapping down again
/// stops every thread of the process for a moment: on twelve threads the scan
/// went little faster than on one. What fontdb reads of a face is its table
/// directory and three tables, `name`, `OS/2` and `post`. So just those are
/// read out of the file, with plain reads, into a font of their own that
/// fontdb then reads as it would have read the file: the face's directory as
/// it is, with these three tables pointing at their copies and the other
/// tables at nothing, since nothing looks at them. Where a table lies past the
/// end of the file, fontdb finds nothing there, and so its copy points past
/// the end of the font made of them.
fn parse_file(path: &Path) -> Vec<fontdb::FaceInfo> {
    let Ok(mut file) = std::fs::File::open(path) else {
        return Vec::new();
    };
    let Ok(file_len) = file.metadata().map(|meta| meta.len()) else {
        return Vec::new();
    };
    let mut read = |offset: u64, len: u64| -> Option<Vec<u8>> {
        use std::io::{Read, Seek, SeekFrom};
        if offset.checked_add(len)? > file_len {
            return None;
        }
        let mut bytes = vec![0; usize::try_from(len).ok()?];
        file.seek(SeekFrom::Start(offset)).ok()?;
        file.read_exact(&mut bytes).ok()?;
        Some(bytes)
    };

    // A collection lists where each of its faces starts; any other file is one
    // face, at its start. ttf-parser rejects a face that would start inside the
    // collection's own header.
    let Some(head) = read(0, 12) else {
        return Vec::new();
    };
    let starts: Vec<(u32, u64)> = if be32(&head, 0) == COLLECTION {
        let count = be32(&head, 8);
        let Some(offsets) = read(12, 4 * u64::from(count)) else {
            return Vec::new();
        };
        let header_len = 12 + 4 * u64::from(count);
        (0..count)
            .map(|index| (index, u64::from(be32(&offsets, 4 * index as usize))))
            .filter(|&(_, start)| start >= header_len)
            .collect()
    } else {
        vec![(0, 0)]
    };

    starts
        .into_iter()
        .filter_map(|(index, start)| {
            let face = face_tables(&mut read, start)?;
            let mut db = fontdb::Database::new();
            db.load_font_data(face);
            let mut info = db.faces().next()?.clone();
            info.source = fontdb::Source::File(path.to_owned());
            info.index = index;
            Some(info)
        })
        .collect()
}

/// The face starting at `start` of a font file, made into a font of its own
/// out of its table directory and the three tables fontdb reads (see
/// [`parse_file`]). `read` reads `len` bytes at an offset into the file.
fn face_tables(read: &mut impl FnMut(u64, u64) -> Option<Vec<u8>>, start: u64) -> Option<Vec<u8>> {
    let head = read(start, 12)?;
    // A collection's faces can't be collections themselves.
    if start != 0 && be32(&head, 0) == COLLECTION {
        return None;
    }
    let tables = u16::from_be_bytes([head[4], head[5]]);
    let mut font = head;
    font.extend(read(start + 12, 16 * u64::from(tables))?);
    for table in 0..usize::from(tables) {
        let record = 12 + 16 * table;
        let (offset, len) = (be32(&font, record + 8), be32(&font, record + 12));
        let wanted = matches!(&font[record..record + 4], b"name" | b"OS/2" | b"post");
        let at = match wanted.then(|| read(u64::from(offset), u64::from(len))) {
            // Not a table fontdb reads.
            None => 0,
            Some(None) => u32::MAX,
            Some(Some(bytes)) => {
                let at = u32::try_from(font.len()).ok()?;
                font.extend(bytes);
                at
            }
        };
        font[record + 8..record + 12].copy_from_slice(&at.to_be_bytes());
        if !wanted {
            font[record + 12..record + 16].fill(0);
        }
    }
    Some(font)
}

/// The big-endian `u32` at `at` in `bytes`.
fn be32(bytes: &[u8], at: usize) -> u32 {
    u32::from_be_bytes(bytes[at..at + 4].try_into().expect("four bytes"))
}

// ---------------------------------------------------------------------------
// Where the fonts are
// ---------------------------------------------------------------------------

/// The directories fontdb's `load_system_fonts` loads, in the order it loads
/// them.
fn roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();

    #[cfg(target_os = "windows")]
    {
        match std::env::var_os("SYSTEMROOT") {
            Some(root) => roots.push(Path::new(&root).join("Fonts")),
            None => roots.push(PathBuf::from("C:\\Windows\\Fonts\\")),
        }
        if let Some(home) = std::env::var_os("USERPROFILE") {
            let home = Path::new(&home);
            roots.push(home.join("AppData\\Local\\Microsoft\\Windows\\Fonts"));
            roots.push(home.join("AppData\\Roaming\\Microsoft\\Windows\\Fonts"));
        }
    }

    #[cfg(target_os = "macos")]
    {
        roots.push(PathBuf::from("/Library/Fonts"));
        roots.push(PathBuf::from("/System/Library/Fonts"));
        // Downloadable fonts, whose location varies with the macOS release.
        if let Ok(assets) = std::fs::read_dir("/System/Library/AssetsV2") {
            for entry in assets.flatten() {
                if entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("com_apple_MobileAsset_Font")
                {
                    roots.push(entry.path());
                }
            }
        }
        roots.push(PathBuf::from("/Network/Library/Fonts"));
        if let Some(home) = std::env::var_os("HOME") {
            roots.push(Path::new(&home).join("Library/Fonts"));
        }
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    roots.extend(fontconfig_dirs());

    roots
}

/// The font directories fontconfig is set up with, read the way fontdb reads
/// them (see its `load_fontconfig`).
#[cfg(all(unix, not(target_os = "macos")))]
fn fontconfig_dirs() -> Vec<PathBuf> {
    let mut fontconfig = fontconfig_parser::FontConfig::default();
    let home = std::env::var_os("HOME");

    if let Some(file) = std::env::var_os("FONTCONFIG_FILE") {
        let _ = fontconfig.merge_config(Path::new(&file));
    } else {
        // $XDG_CONFIG_HOME, which is ~/.config when it isn't set.
        let config_home = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| home.as_ref().map(|home| Path::new(home).join(".config")));
        let read_global = match config_home {
            Some(dir) => fontconfig
                .merge_config(&dir.join("fontconfig/fonts.conf"))
                .is_err(),
            None => true,
        };
        if read_global {
            let _ = fontconfig.merge_config(Path::new("/etc/fonts/local.conf"));
        }
        let _ = fontconfig.merge_config(Path::new("/etc/fonts/fonts.conf"));
    }

    fontconfig
        .dirs
        .into_iter()
        .filter_map(|dir| match dir.path.strip_prefix("~") {
            Ok(rest) => home.as_ref().map(|home| Path::new(home).join(rest)),
            Err(_) => Some(dir.path),
        })
        .collect()
}

/// Where the index lives: the platform's cache directory for the user.
fn index_path() -> Option<PathBuf> {
    let env_dir = |name| {
        std::env::var_os(name)
            .map(PathBuf::from)
            .filter(|dir| dir.is_absolute())
    };
    #[cfg(target_os = "windows")]
    let cache = env_dir("LOCALAPPDATA");
    #[cfg(target_os = "macos")]
    let cache = env_dir("HOME").map(|home| home.join("Library/Caches"));
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let cache =
        env_dir("XDG_CACHE_HOME").or_else(|| env_dir("HOME").map(|home| home.join(".cache")));
    Some(cache?.join("saudade").join(INDEX_FILE))
}

// ---------------------------------------------------------------------------
// The index file
// ---------------------------------------------------------------------------
//
// A binary file: the magic line, then each directory with its entries. Numbers
// are little-endian; strings and paths a u32 length and their bytes. Anything
// that doesn't read back as it should makes the whole index count as missing,
// which costs one full scan and rewrites it.

/// Read the index at `path`. `None` when there isn't one, or it is damaged.
fn read_index(path: &Path) -> Option<HashMap<PathBuf, Dir>> {
    let data = std::fs::read(path).ok()?;
    let mut r = Reader(data.strip_prefix(MAGIC)?);
    let mut dirs = HashMap::new();
    for _ in 0..r.u32()? {
        let path = r.path()?;
        let mtime = r.u128()?;
        let mut entries = Vec::new();
        for _ in 0..r.u32()? {
            entries.push(match r.u8()? {
                0 => {
                    let file = r.path()?;
                    let mut faces = Vec::new();
                    for _ in 0..r.u32()? {
                        faces.push(r.face(&file)?);
                    }
                    Entry::File(file, Some(faces))
                }
                1 => Entry::Dir(r.path()?),
                _ => return None,
            });
        }
        dirs.insert(path, Dir { mtime, entries });
    }
    r.0.is_empty().then_some(dirs)
}

/// Write the index to `path`, by way of a file of its own beside it, so that
/// another app reading the index meanwhile sees either the old one or this. A
/// directory whose name the index can't hold is left out, and read every time.
fn write_index(path: &Path, dirs: &HashMap<PathBuf, Dir>) {
    let mut w = Vec::from(MAGIC);
    let mut count: u32 = 0;
    let count_at = w.len();
    put_u32(&mut w, 0);
    for (dir, found) in dirs {
        let start = w.len();
        if put_dir(&mut w, dir, found).is_none() {
            w.truncate(start);
        } else {
            count += 1;
        }
    }
    w[count_at..count_at + 4].copy_from_slice(&count.to_le_bytes());

    let Some(parent) = path.parent() else {
        return;
    };
    let _ = std::fs::create_dir_all(parent);
    let staging = path.with_extension(format!("{}.tmp", std::process::id()));
    if std::fs::write(&staging, &w).is_err() || std::fs::rename(&staging, path).is_err() {
        let _ = std::fs::remove_file(&staging);
    }
}

fn put_dir(w: &mut Vec<u8>, dir: &Path, found: &Dir) -> Option<()> {
    put_path(w, dir)?;
    w.extend_from_slice(&found.mtime.to_le_bytes());
    put_u32(w, found.entries.len() as u32);
    for entry in &found.entries {
        match entry {
            Entry::File(file, faces) => {
                w.push(0);
                put_path(w, file)?;
                let faces = faces.as_deref().unwrap_or_default();
                put_u32(w, faces.len() as u32);
                for face in faces {
                    put_face(w, face);
                }
            }
            Entry::Dir(sub) => {
                w.push(1);
                put_path(w, sub)?;
            }
        }
    }
    Some(())
}

fn put_face(w: &mut Vec<u8>, face: &fontdb::FaceInfo) {
    put_u32(w, face.index);
    put_u32(w, face.families.len() as u32);
    for (family, _) in &face.families {
        put_bytes(w, family.as_bytes());
    }
    put_bytes(w, face.post_script_name.as_bytes());
    w.push(match face.style {
        fontdb::Style::Normal => 0,
        fontdb::Style::Italic => 1,
        fontdb::Style::Oblique => 2,
    });
    w.extend_from_slice(&face.weight.0.to_le_bytes());
    w.extend_from_slice(&face.stretch.to_number().to_le_bytes());
    w.push(face.monospaced as u8);
}

fn put_u32(w: &mut Vec<u8>, n: u32) {
    w.extend_from_slice(&n.to_le_bytes());
}

fn put_bytes(w: &mut Vec<u8>, bytes: &[u8]) {
    put_u32(w, bytes.len() as u32);
    w.extend_from_slice(bytes);
}

/// A path as bytes: as the system has them on Unix, as UTF-8 elsewhere, where
/// a path that isn't valid Unicode can't be written.
fn put_path(w: &mut Vec<u8>, path: &Path) -> Option<()> {
    #[cfg(unix)]
    let bytes = std::os::unix::ffi::OsStrExt::as_bytes(path.as_os_str());
    #[cfg(not(unix))]
    let bytes = path.to_str()?.as_bytes();
    put_bytes(w, bytes);
    Some(())
}

/// The rest of an index file, read from the front.
struct Reader<'a>(&'a [u8]);

impl Reader<'_> {
    fn take<const N: usize>(&mut self) -> Option<[u8; N]> {
        let (head, rest) = self.0.split_first_chunk::<N>()?;
        self.0 = rest;
        Some(*head)
    }

    fn u8(&mut self) -> Option<u8> {
        self.take::<1>().map(|[b]| b)
    }

    fn u16(&mut self) -> Option<u16> {
        self.take().map(u16::from_le_bytes)
    }

    fn u32(&mut self) -> Option<u32> {
        self.take().map(u32::from_le_bytes)
    }

    fn u128(&mut self) -> Option<u128> {
        self.take().map(u128::from_le_bytes)
    }

    fn bytes(&mut self) -> Option<&[u8]> {
        let len = self.u32()? as usize;
        if len > self.0.len() {
            return None;
        }
        let (head, rest) = self.0.split_at(len);
        self.0 = rest;
        Some(head)
    }

    fn string(&mut self) -> Option<String> {
        String::from_utf8(self.bytes()?.to_vec()).ok()
    }

    fn path(&mut self) -> Option<PathBuf> {
        #[cfg(unix)]
        let path = <std::ffi::OsStr as std::os::unix::ffi::OsStrExt>::from_bytes(self.bytes()?);
        #[cfg(not(unix))]
        let path = std::str::from_utf8(self.bytes()?).ok()?;
        Some(PathBuf::from(path))
    }

    fn face(&mut self, file: &Path) -> Option<fontdb::FaceInfo> {
        let index = self.u32()?;
        let mut families = Vec::new();
        for _ in 0..self.u32()? {
            families.push((self.string()?, fontdb::Language::English_UnitedStates));
        }
        let post_script_name = self.string()?;
        let style = match self.u8()? {
            0 => fontdb::Style::Normal,
            1 => fontdb::Style::Italic,
            2 => fontdb::Style::Oblique,
            _ => return None,
        };
        let weight = fontdb::Weight(self.u16()?);
        let stretch = match self.u16()? {
            1 => fontdb::Stretch::UltraCondensed,
            2 => fontdb::Stretch::ExtraCondensed,
            3 => fontdb::Stretch::Condensed,
            4 => fontdb::Stretch::SemiCondensed,
            5 => fontdb::Stretch::Normal,
            6 => fontdb::Stretch::SemiExpanded,
            7 => fontdb::Stretch::Expanded,
            8 => fontdb::Stretch::ExtraExpanded,
            9 => fontdb::Stretch::UltraExpanded,
            _ => return None,
        };
        let monospaced = match self.u8()? {
            0 => false,
            1 => true,
            _ => return None,
        };
        Some(fontdb::FaceInfo {
            id: fontdb::ID::dummy(),
            source: fontdb::Source::File(file.to_owned()),
            index,
            families,
            post_script_name,
            style,
            weight,
            stretch,
            monospaced,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A directory of its own under the system's temporary one, removed again
    /// when the test is done with it.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("saudade-font-scan-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("a temporary directory");
            Self(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Everything a query can tell two faces apart by, in database order.
    fn faces(db: &fontdb::Database) -> Vec<String> {
        db.faces()
            .map(|face| {
                let fontdb::Source::File(path) = &face.source else {
                    panic!("a face from a file");
                };
                let families: Vec<_> = face.families.iter().map(|(name, _)| name).collect();
                format!(
                    "{} #{} {families:?} {} {:?} {} {:?} {}",
                    path.display(),
                    face.index,
                    face.post_script_name,
                    face.style,
                    face.weight.0,
                    face.stretch,
                    face.monospaced,
                )
            })
            .collect()
    }

    /// The bundled test fonts, copied into `dir`.
    fn copy_fixtures(dir: &Path) {
        for name in ["DejaVuSans.ttf", "DejaVuSansMono.ttf"] {
            std::fs::copy(format!("tests/fonts/{name}"), dir.join(name)).expect("a fixture");
        }
    }

    #[test]
    fn the_host_fonts_come_out_as_fontdb_finds_them() {
        let mut expected = fontdb::Database::new();
        expected.load_system_fonts();
        if expected.is_empty() {
            expected.load_fonts_dir("/usr/local/share/fonts");
        }

        let temp = TempDir::new("host");
        let index = temp.0.join(INDEX_FILE);
        let now = SystemTime::now();
        let (fresh, _) = scan(&roots(), Some(&index), now);
        assert_eq!(faces(&fresh), faces(&expected), "a scan without an index");
        let (indexed, work) = scan(&roots(), Some(&index), now);
        assert_eq!(faces(&indexed), faces(&expected), "a scan from the index");
        assert_eq!(work, Work::default(), "with nothing read again");
    }

    /// A time long enough after now for the directories a test has just made
    /// to have [settled](SETTLE).
    fn later() -> SystemTime {
        SystemTime::now() + Duration::from_secs(3600)
    }

    /// Wait out the file system's clock, so that a change made next can't
    /// carry the time of the one before it.
    fn tick() {
        std::thread::sleep(Duration::from_millis(50));
    }

    #[test]
    fn the_index_is_used_until_a_directory_changes() {
        let temp = TempDir::new("changes");
        let fonts = temp.0.join("fonts");
        let sub = fonts.join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        copy_fixtures(&fonts);
        let index = temp.0.join(INDEX_FILE);
        let roots = [fonts.clone()];

        let (db, work) = scan(&roots, Some(&index), later());
        assert_eq!(db.len(), 2);
        let all = Work {
            dirs_read: 2,
            files_parsed: 2,
        };
        assert_eq!(work, all);

        let (again, work) = scan(&roots, Some(&index), later());
        assert_eq!(faces(&again), faces(&db));
        assert_eq!(work, Work::default(), "everything comes from the index");

        // A font added to the subdirectory: only that directory is read again,
        // and only the new file parsed.
        tick();
        std::fs::copy("tests/fonts/DejaVuSans.ttf", sub.join("Copy.ttf")).unwrap();
        let (grown, work) = scan(&roots, Some(&index), later());
        assert_eq!(grown.len(), 3);
        let one = Work {
            dirs_read: 1,
            files_parsed: 1,
        };
        assert_eq!(work, one);

        // And one taken away.
        tick();
        std::fs::remove_file(fonts.join("DejaVuSansMono.ttf")).unwrap();
        let (shrunk, _) = scan(&roots, Some(&index), later());
        assert_eq!(shrunk.len(), 2);
        assert!(shrunk.faces().all(|face| !face.monospaced));
    }

    #[test]
    fn a_directory_read_just_after_it_changed_is_read_again() {
        let temp = TempDir::new("settle");
        let fonts = temp.0.join("fonts");
        std::fs::create_dir_all(&fonts).unwrap();
        copy_fixtures(&fonts);
        let index = temp.0.join(INDEX_FILE);
        let roots = [fonts];

        // The directory changed a moment ago, so what was read of it can't be
        // trusted yet...
        let (_, work) = scan(&roots, Some(&index), SystemTime::now());
        assert_eq!(work.dirs_read, 1);
        let (_, work) = scan(&roots, Some(&index), later());
        assert_eq!(work.dirs_read, 1, "so it is read again");
        // ...until it has settled.
        let (_, work) = scan(&roots, Some(&index), later());
        assert_eq!(work, Work::default());
    }

    #[test]
    fn a_damaged_index_is_read_as_none() {
        let temp = TempDir::new("damaged");
        let fonts = temp.0.join("fonts");
        std::fs::create_dir_all(&fonts).unwrap();
        copy_fixtures(&fonts);
        let index = temp.0.join(INDEX_FILE);
        let roots = [fonts];
        let (db, _) = scan(&roots, Some(&index), later());

        let whole = std::fs::read(&index).unwrap();
        std::fs::write(&index, &whole[..whole.len() - 3]).unwrap();
        let (rescanned, work) = scan(&roots, Some(&index), later());
        assert_eq!(faces(&rescanned), faces(&db));
        assert_eq!(
            work,
            Work {
                dirs_read: 1,
                files_parsed: 2
            }
        );
        assert_eq!(std::fs::read(&index).unwrap(), whole, "and written anew");
    }
}
