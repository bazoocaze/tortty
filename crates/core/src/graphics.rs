//! kitty graphics protocol (APC `ESC _ G … ST`) — the pure part.
//!
//! An app sends an image like this::
//!
//! ```text
//! ESC _ G a=T,f=100,i=1,q=2;<base64 of the PNG> ESC \
//! ```
//!
//! and the terminal replies (when silence is not asked for) `ESC _ G i=1;OK ESC \`.
//! Only the parse/decoding and the *placements* list live here; texture and
//! drawing belong to the app. That way the protocol is testable without display.
//!
//! Supported (enough for `kitten icat`): `a=t`/`a=T`/`a=p`/`a=d`/`a=q`,
//! `f=24` RGB, `f=32` RGBA, `f=100` PNG, `m=1` (image in chunks), `o=z`
//! (zlib), `i=`/`c=`/`r=`/`X=`/`Y=`/`C=`/`q=`.
//!
//! Out, on purpose: animation (`a=f`/`a=a`) and z-index — the child cannot
//! move what is already drawn, so there is nothing to animate or stack.

use std::collections::HashMap;

/// Cap on the buffer of a chunked transmission: an app that opens an `m=1` and
/// never closes it cannot grow forever (here the parser is
/// fed by anything that writes to the pty).
pub const MAX_APC: usize = 64 * 1024 * 1024;

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Format {
    Rgb,
    Rgba,
    Png,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Image {
    pub id: u32,
    /// Changes on every transmission: it is how the app knows the cached texture
    /// went stale when the app reuses the same `i=`.
    pub version: u64,
    pub width: u32,
    pub height: u32,
    pub format: Format,
    pub data: Vec<u8>,
}

/// What is drawn right now. `linha` is a **screen line**: 0..rows-1 is the live
/// grid, and negative enters the history (-1 = last scrollback line). So
/// scrolling is subtracting 1 from everybody, which is what the content does.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct Placement {
    pub image_id: u32,
    pub col: u16,
    pub line: i32,
    pub cols: u16,
    pub rows: u16,
    /// `X=`/`Y=`: offset in **pixels** inside the first cell. That is how
    /// `icat` centers: it walks whole cells and leaves the remainder here.
    pub dx: i32,
    pub dy: i32,
}

/// A transmission by file/temp/shared-memory (`B1`): what the app must load.
///
/// The payload of `t=f`/`t=t`/`t=s` is a path, not pixels. The core keeps the
/// request's context (who asked, where to show) and the app answers with
/// `Graphics::provide` — or the image stays missing and the child gets
/// `ENOENT`. Shell expansion (`~`) belongs to the app: the path here is raw.
#[derive(Clone, Debug)]
pub struct FileRequest {
    pub path: String,
    /// `t=t`: the file is temporary — the app deletes it after reading.
    /// `t=s` reads shared memory and never deletes.
    pub delete: bool,
    /// `a=T` also places the image once loaded; `a=t` only transmits.
    pub display: bool,
    pub ident: u32,
    pub quiet: bool,
    pub kv: HashMap<String, String>,
    pub cursor: (u16, u16),
    pub cell: (u32, u32),
}

/// What the `Term` must do after an APC.
#[derive(Default, Debug)]
pub struct Output {
    /// APC bodies to send back to the app (without `ESC _` / `ST`).
    pub responses: Vec<Vec<u8>>,
    /// How many lines the image took (`C=1` asks not to use them).
    pub lines: u16,
    /// Should the cursor move down `linhas`?
    pub move_cursor: bool,
    /// The image lives in a file: its path, decoded from the payload. The
    /// core never touches the filesystem — the app loads it and answers with
    /// `provide`, and only then does the child get `OK`/`ENOENT`.
    pub file: Option<FileRequest>,
}

#[derive(Default)]
pub struct Graphics {
    images: HashMap<u32, Image>,
    placements: Vec<Placement>,
    next_id: u32,
    /// Image version counter (one per transmission).
    generation: u64,
    /// Chunks of a transmission in progress (`m=1`).
    buf: Option<Buf>,
}

struct Buf {
    b64: String,
    kv: HashMap<String, String>,
}

impl Graphics {
    #[must_use]
    pub fn new() -> Self {
        Self {
            next_id: 1,
            ..Self::default()
        }
    }

    #[must_use]
    pub fn images(&self) -> &HashMap<u32, Image> {
        &self.images
    }

    #[must_use]
    pub fn placements(&self) -> &[Placement] {
        &self.placements
    }

    /// `ED 3` (H13): no image survives.
    pub fn clear(&mut self) {
        self.images.clear();
        self.placements.clear();
        self.buf = None;
    }

    /// Follows the scroll: the placements go up along with the text.
    ///
    /// `floor`: up to which line (negative, that is, inside the history) to keep
    /// a placement that left through the top; `None` = drop it (region scroll,
    /// where there is no history). Without it the image vanished on the first
    /// line that scrolled and did not come back when scrolling up through the
    /// history with the wheel.
    pub fn scroll(&mut self, top: u16, bottom: u16, floor: Option<i32>) {
        let top = i32::from(top);
        let bottom = i32::from(bottom);
        self.placements.retain_mut(|p| {
            if p.line > bottom {
                return true; // outside the region: leave it alone
            }
            p.line -= 1;
            match floor {
                Some(f) => p.line >= f,
                None => p.line >= top,
            }
        });
    }

    /// Drops what left the history for good (the oldest lines).
    pub fn drop_above(&mut self, floor: i32) {
        self.placements.retain(|p| p.line >= floor);
    }

    fn place(
        &mut self,
        img: &Image,
        kv: &HashMap<String, String>,
        cursor: (u16, u16),
        cell: (u32, u32),
    ) -> u16 {
        let cw = cell.0.max(1);
        let ch = cell.1.max(1);
        let num = |k: &str| kv.get(k).and_then(|v| v.parse::<i64>().ok());
        let cols = num("c").map_or_else(
            || {
                u16::try_from(img.width.div_ceil(cw))
                    .unwrap_or(u16::MAX)
                    .max(1)
            },
            |c| u16::try_from(c.max(1)).unwrap_or(u16::MAX),
        );
        let rows = num("r").map_or_else(
            || {
                u16::try_from(img.height.div_ceil(ch))
                    .unwrap_or(u16::MAX)
                    .max(1)
            },
            |r| u16::try_from(r.max(1)).unwrap_or(u16::MAX),
        );
        self.placements.push(Placement {
            image_id: img.id,
            col: cursor.0,
            line: i32::from(cursor.1),
            cols,
            rows,
            dx: i32::try_from(num("X").unwrap_or(0)).unwrap_or(0),
            dy: i32::try_from(num("Y").unwrap_or(0)).unwrap_or(0),
        });
        rows
    }

    /// Processes the body of an APC (`G…`, already without the delimiters).
    pub fn handle(&mut self, body: &[u8], cursor: (u16, u16), cell: (u32, u32)) -> Output {
        let Some(body) = body.strip_prefix(b"G") else {
            return Output::default();
        };
        let text = String::from_utf8_lossy(body);
        let (ctrl, payload) = text
            .split_once(';')
            .map_or((text.as_ref(), ""), |(a, b)| (a, b));
        let mut cur: HashMap<String, String> = HashMap::new();
        for item in ctrl.split(',') {
            if let Some((k, v)) = item.split_once('=') {
                cur.insert(k.to_string(), v.to_string());
            }
        }
        // the control data of the **first** chunk holds for the whole
        // transmission; the following ones only carry `m=1` (and it is per chunk,
        // it does not accumulate — keeping the `m` made the transmission never
        // end)
        let new = self.buf.is_none();
        if new {
            self.buf = Some(Buf {
                b64: String::new(),
                kv: cur.clone(),
            });
        }
        if let Some(buf) = self.buf.as_mut() {
            if buf.b64.len() + payload.len() > MAX_APC {
                self.buf = None;
                return Output {
                    responses: vec![b"Gi=0;EINVAL:too big".to_vec()],
                    ..Output::default()
                };
            }
            buf.b64.push_str(payload);
        }
        if cur.get("m").is_some_and(|m| m == "1") {
            return Output::default(); // there is still a chunk coming
        }
        let Some(buf) = self.buf.take() else {
            return Output::default();
        };
        let kv = buf.kv;
        let Some(data) = base64_decode(&buf.b64) else {
            return Self::error(&kv, "EINVAL:base64");
        };
        let param = |k: &str| kv.get(k).map(String::as_str);
        let action = param("a").unwrap_or("t");
        let medium = param("t").unwrap_or("d");
        let quiet = param("q").is_some_and(|q| q != "0");
        let ident = param("i").and_then(|i| i.parse::<u32>().ok()).unwrap_or(0);
        if (action == "t" || action == "T") && matches!(medium, "f" | "t" | "s") {
            // the payload names a file — the app reads it and answers with
            // `provide`. No response yet: `OK`/`ENOENT` comes with the bytes
            // (or without them).
            return Output {
                file: Some(FileRequest {
                    path: String::from_utf8_lossy(&data).into_owned(),
                    delete: medium == "t",
                    display: action == "T",
                    ident,
                    quiet,
                    kv,
                    cursor,
                    cell,
                }),
                ..Output::default()
            };
        }
        let mut data = data;
        if kv.get("o").is_some_and(|o| o == "z") && !data.is_empty() {
            // `o=z`: the payload comes compressed (`icat --place` sends the
            // decoded PNG + zlib to scale it without re-encoding)
            match inflate(&data) {
                Some(d) => data = d,
                None => return Self::error(&kv, "EINVAL:zlib"),
            }
        }
        self.dispatch(action, &kv, &data, ident, quiet, cursor, cell)
    }

    /// Routes the APC action. Kept apart from the parse so the protocol fits in
    /// one's head: first "what arrived", then "what to do".
    #[allow(clippy::too_many_arguments)] // these are the request's fields, not a bag
    fn dispatch(
        &mut self,
        action: &str,
        kv: &HashMap<String, String>,
        data: &[u8],
        ident: u32,
        quiet: bool,
        cursor: (u16, u16),
        cell: (u32, u32),
    ) -> Output {
        let param = |k: &str| kv.get(k).map(String::as_str);
        match action {
            // animation and control: ignored on purpose (H18)
            "f" | "a" | "c" => Output::default(),
            "q" => {
                // `a=q` is a **capability** probe, not an image query: `kitten icat`
                // sends three (direct, file, memory) and only proceeds if it gets
                // OK on the direct transmission. Answering ENOENT because the id
                // does not exist makes icat give up with "terminal does not
                // support the graphics protocol".
                let middle = param("t").unwrap_or("d");
                let fmt = param("f").and_then(|f| f.parse::<u32>().ok()).unwrap_or(32);
                let ok = matches!(middle, "d" | "f" | "t" | "s") && [24, 32, 100].contains(&fmt);
                let name = if ok {
                    "OK"
                } else {
                    "EINVAL:unsupported transmission"
                };
                Output {
                    responses: if ident == 0 {
                        Vec::new()
                    } else {
                        vec![format!("Gi={ident};{name}").into_bytes()]
                    },
                    ..Output::default()
                }
            },
            "d" => {
                let como = param("d").unwrap_or("a");
                if como == "i" {
                    self.images.remove(&ident);
                    self.placements.retain(|p| p.image_id != ident);
                } else {
                    self.placements.clear();
                    if como == "a" {
                        self.images.clear();
                    }
                }
                Output {
                    responses: if quiet {
                        Vec::new()
                    } else {
                        vec![format!("Gi={ident};OK").into_bytes()]
                    },
                    ..Output::default()
                }
            },
            "p" => match self.images.get(&ident) {
                None => Output {
                    responses: vec![format!("Gi={ident};ENOENT").into_bytes()],
                    ..Output::default()
                },
                Some(img) => {
                    let img = img.clone();
                    let lines = self.place(&img, kv, cursor, cell);
                    Output {
                        responses: if quiet {
                            Vec::new()
                        } else {
                            vec![format!("Gi={ident};OK").into_bytes()]
                        },
                        lines,
                        move_cursor: param("C") != Some("1"),
                        file: None,
                    }
                },
            },
            "t" | "T" => self.transmit(kv, data, ident, quiet, cursor, cell),
            _ => {
                if quiet {
                    Output::default()
                } else {
                    Self::error(kv, "EINVAL:action")
                }
            },
        }
    }

    fn transmit(
        &mut self,
        kv: &HashMap<String, String>,
        data: &[u8],
        ident: u32,
        quiet: bool,
        cursor: (u16, u16),
        cell: (u32, u32),
    ) -> Output {
        let param = |k: &str| kv.get(k).map(String::as_str);
        let mut fmt = param("f").and_then(|f| f.parse::<u32>().ok()).unwrap_or(32);
        if data.starts_with(b"\x89PNG\r\n\x1a\n") {
            fmt = 100; // the PNG already says what it is, regardless of `f=`
        }
        let (width, height) = if fmt == 100 {
            let (pw, ph) = png_size(data);
            (
                param("s").and_then(|s| s.parse().ok()).unwrap_or(pw),
                param("v").and_then(|v| v.parse().ok()).unwrap_or(ph),
            )
        } else {
            let w: u32 = param("s").and_then(|s| s.parse().ok()).unwrap_or(0);
            let h: u32 = param("v").and_then(|v| v.parse().ok()).unwrap_or(0);
            let bpp = if fmt == 32 { 4 } else { 3 };
            let needed = u64::from(w) * u64::from(h) * bpp;
            if w == 0 || h == 0 || (data.len() as u64) < needed {
                return Self::error(kv, "EINVAL:size");
            }
            (w, h)
        };
        if width == 0 || height == 0 {
            return Self::error(kv, "EINVAL:size");
        }
        let id = if ident == 0 {
            let i = self.next_id.max(1);
            self.next_id = i + 1;
            i
        } else {
            ident
        };
        let format = match fmt {
            100 => Format::Png,
            24 => Format::Rgb,
            _ => Format::Rgba,
        };
        self.generation += 1;
        let img = Image {
            id,
            version: self.generation,
            width,
            height,
            format,
            data: data.to_vec(),
        };
        self.images.insert(id, img.clone());
        let mut out = Output {
            responses: if quiet {
                Vec::new()
            } else {
                vec![format!("Gi={id};OK").into_bytes()]
            },
            ..Output::default()
        };
        if param("a") == Some("T") {
            out.lines = self.place(&img, kv, cursor, cell);
            out.move_cursor = param("C") != Some("1");
        }
        out
    }

    /// Completes a file transmission (`B1`): the app loaded `req.path` and
    /// hands the bytes over — from here it is a direct transmission, same
    /// checks, same answers. Oversize answers like an overlong chunked one.
    pub fn provide(&mut self, req: &FileRequest, data: &[u8]) -> Output {
        if data.len() > MAX_APC {
            return Self::error(&req.kv, "EINVAL:too big");
        }
        self.transmit(&req.kv, data, req.ident, req.quiet, req.cursor, req.cell)
    }

    /// The app could not load the file (`B1`): missing, unreadable, a
    /// directory — the child cannot tell which, it only learns the image is
    /// not coming. Existence still leaks (as with any readable path), which is
    /// why the app never sends bytes back, only this verdict.
    pub fn missing(req: &FileRequest) -> Output {
        Self::error(&req.kv, "ENOENT")
    }

    fn error(kv: &HashMap<String, String>, msg: &str) -> Output {
        let ident = kv.get("i").map_or("0", String::as_str);
        Output {
            responses: vec![format!("Gi={ident};{msg}").into_bytes()],
            ..Output::default()
        }
    }
}

/// IHDR width/height: the PNG already says it, so `s=`/`v=` is not needed.
#[must_use]
pub fn png_size(data: &[u8]) -> (u32, u32) {
    if data.len() < 24 || !data.starts_with(b"\x89PNG\r\n\x1a\n") {
        return (0, 0);
    }
    let w = u32::from_be_bytes([data[16], data[17], data[18], data[19]]);
    let h = u32::from_be_bytes([data[20], data[21], data[22], data[23]]);
    (w, h)
}

/// base64 with optional padding (the app may send it unpadded): `None` = skewed.
fn base64_decode(text: &str) -> Option<Vec<u8>> {
    use base64::Engine as _;
    let clean: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    base64::engine::general_purpose::STANDARD_NO_PAD
        .decode(clean.trim_end_matches('='))
        .ok()
}

/// `o=z`: zlib (not gzip, not raw deflate) — it is what kitty sends.
fn inflate(data: &[u8]) -> Option<Vec<u8>> {
    use std::io::Read as _;
    let mut out = Vec::new();
    flate2::read::ZlibDecoder::new(data)
        .read_to_end(&mut out)
        .ok()?;
    Some(out)
}

#[cfg(test)]
mod checks_graphics {
    use super::{png_size, Format, Graphics};
    use base64::Engine as _;

    fn b64(data: &[u8]) -> String {
        base64::engine::general_purpose::STANDARD.encode(data)
    }

    /// 2x2 RGBA in a single color.
    fn rgba2x2() -> String {
        b64(&[9, 8, 7, 255].repeat(4))
    }

    fn cell() -> (u32, u32) {
        (10, 19)
    }

    #[test]
    fn image_with_offset_and_size_derived_from_cell() {
        // 2x2 in a 10x19 cell = 1x1
        let mut g = Graphics::new();
        let body = format!("Ga=T,f=32,s=2,v=2,i=3;{}", rgba2x2());
        let out = g.handle(body.as_bytes(), (0, 1), cell());
        assert_eq!(out.responses, [b"Gi=3;OK".to_vec()]);
        assert!(out.move_cursor);
        assert_eq!(out.lines, 1);
        let img = g.images().get(&3).expect("image stored");
        assert_eq!((img.width, img.height, img.format), (2, 2, Format::Rgba));
        let p = g.placements()[0];
        assert_eq!((p.image_id, p.col, p.line, p.cols, p.rows), (3, 0, 1, 1, 1));
    }

    #[test]
    fn zlib_with_pixel_offset_and_c_1_should_not_move() {
        // `o=z` + `X=3` + `C=1`: 4x2 RGB in a 10x19 cell = 1x1
        let data: Vec<u8> = (0..24).collect();
        let mut comp = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        std::io::Write::write_all(&mut comp, &data).expect("compresses");
        let zip = comp.finish().expect("zlib");
        let mut g = Graphics::new();
        let body = format!("Ga=T,f=24,o=z,s=4,v=2,C=1,X=3;{}", b64(&zip));
        let out = g.handle(body.as_bytes(), (0, 0), cell());
        assert!(!out.move_cursor, "C=1 asks not to touch the cursor");
        let p = g.placements()[0];
        assert_eq!((p.dx, p.cols, p.rows), (3, 1, 1));
    }

    #[test]
    fn chunked_transmission_only_ends_on_the_last() {
        let mut g = Graphics::new();
        let all = rgba2x2();
        let (a, b) = all.split_at(8);
        let first = format!("Ga=t,f=32,s=2,v=2,i=5,m=1;{a}");
        let out = g.handle(first.as_bytes(), (0, 0), cell());
        assert_eq!(out.responses.len(), 0, "a chunk does not answer");
        assert!(g.images().is_empty(), "the image only exists at the end");
        // the second chunk carries only `m=0`: the first one's control data holds
        let second = format!("Gm=0;{b}");
        let out = g.handle(second.as_bytes(), (0, 0), cell());
        assert_eq!(out.responses, [b"Gi=5;OK".to_vec()]);
        assert_eq!(g.images()[&5].data, [9, 8, 7, 255].repeat(4));
    }

    #[test]
    fn capacity_query_answers_by_medium_and_by_format() {
        // `kitten icat`'s `a=q`: direct, file, temp and shm with 24/32/100
        // are OK (`B1` unlocked the three file media)
        let mut g = Graphics::new();
        for (f, ok) in [(24, true), (32, true), (100, true), (16, false)] {
            let body = format!("Ga=q,i=1,f={f},t=d");
            let out = g.handle(body.as_bytes(), (0, 0), cell());
            let expected = if ok {
                "Gi=1;OK"
            } else {
                "Gi=1;EINVAL:unsupported transmission"
            };
            assert_eq!(out.responses, [expected.as_bytes().to_vec()], "f={f}");
        }
        for t in ["f", "t", "s"] {
            let body = format!("Ga=q,i=1,f=32,t={t}");
            let out = g.handle(body.as_bytes(), (0, 0), cell());
            assert_eq!(out.responses, [b"Gi=1;OK".to_vec()], "t={t}");
        }
    }

    fn file_body(t: &str, action: &str) -> Vec<u8> {
        let path = b64(b"/tmp/x.png");
        format!("Ga={action},f=100,t={t},i=7;{path}").into_bytes()
    }

    #[test]
    fn file_transmission_waits_for_the_app_instead_of_answering() {
        let mut g = Graphics::new();
        let out = g.handle(&file_body("f", "T"), (1, 2), cell());
        assert!(out.responses.is_empty(), "no verdict before the bytes");
        assert!(g.images().is_empty(), "no image before the bytes");
        let req = out.file.expect("the path waits for the app");
        assert_eq!(req.path, "/tmp/x.png");
        assert!(!req.delete, "t=f never deletes");
        assert!(req.display, "a=T also places");
        assert_eq!((req.ident, req.cursor, req.quiet), (7, (1, 2), false));
    }

    #[test]
    fn temp_marks_for_delete_and_transmit_only_does_not_place() {
        let mut g = Graphics::new();
        let out = g.handle(&file_body("t", "t"), (0, 0), cell());
        let req = out.file.expect("t=t also waits");
        assert!(req.delete, "t=t deletes after reading");
        assert!(!req.display, "a=t only transmits");
        let mut g = Graphics::new();
        let out = g.handle(&file_body("s", "T"), (0, 0), cell());
        assert!(!out.file.expect("t=s waits").delete, "shm never deletes");
    }

    #[test]
    fn provide_turns_the_loaded_bytes_into_an_image() {
        let mut g = Graphics::new();
        let path = b64(b"/tmp/x.rgba");
        let body = format!("Ga=T,f=32,s=2,v=2,t=f,i=7;{path}").into_bytes();
        let out = g.handle(&body, (0, 1), cell());
        let req = out.file.expect("request");
        let data = [9, 8, 7, 255].repeat(4);
        let out = g.provide(&req, &data);
        assert_eq!(out.responses, [b"Gi=7;OK".to_vec()]);
        assert!(out.move_cursor);
        assert_eq!(out.lines, 1);
        let img = g.images().get(&7).expect("image stored");
        assert_eq!((img.width, img.height), (2, 2));
        assert_eq!(g.placements().len(), 1, "a=T placed it");
    }

    #[test]
    fn missing_file_answers_enoent_and_stores_nothing() {
        let mut g = Graphics::new();
        let out = g.handle(&file_body("f", "T"), (0, 0), cell());
        let req = out.file.expect("request");
        let out = Graphics::missing(&req);
        assert_eq!(out.responses, [b"Gi=7;ENOENT".to_vec()]);
        assert!(g.images().is_empty());
    }

    #[test]
    fn oversize_file_answers_like_an_overlong_chunk() {
        let mut g = Graphics::new();
        let out = g.handle(&file_body("f", "t"), (0, 0), cell());
        let req = out.file.expect("request");
        let out = g.provide(&req, &vec![0; super::MAX_APC + 1]);
        assert_eq!(out.responses, [b"Gi=7;EINVAL:too big".to_vec()]);
        assert!(g.images().is_empty());
    }

    #[test]
    fn display_already_transmitted_and_enoent_when_missing() {
        let mut g = Graphics::new();
        g.handle(
            format!("Ga=t,f=32,s=2,v=2,i=9;{}", rgba2x2()).as_bytes(),
            (0, 0),
            cell(),
        );
        let out = g.handle(b"Ga=p,i=9,c=3,r=2", (2, 1), cell());
        assert_eq!(out.responses, [b"Gi=9;OK".to_vec()]);
        let p = g.placements()[0];
        assert_eq!((p.col, p.line, p.cols, p.rows), (2, 1, 3, 2));
        let out = g.handle(b"Ga=p,i=404", (0, 0), cell());
        assert_eq!(out.responses, [b"Gi=404;ENOENT".to_vec()]);
    }

    #[test]
    fn clear_per_id_and_clear_all() {
        let mut g = Graphics::new();
        for i in [1, 2] {
            g.handle(
                format!("Ga=T,f=32,s=2,v=2,i={i};{}", rgba2x2()).as_bytes(),
                (0, 0),
                cell(),
            );
        }
        assert_eq!(g.placements().len(), 2);
        // `d=i`: one image and its placement
        g.handle(b"Ga=d,d=i,i=1", (0, 0), cell());
        assert!(!g.images().contains_key(&1));
        assert!(g.images().contains_key(&2));
        assert_eq!(g.placements().len(), 1);
        // `d=a`: everything goes
        g.handle(b"Ga=d,d=a", (0, 0), cell());
        assert_eq!(g.images().len(), 0);
        assert_eq!(g.placements(), []);
    }

    #[test]
    fn q_2_silences_the_response() {
        let mut g = Graphics::new();
        let out = g.handle(
            format!("Ga=T,f=32,s=2,v=2,i=4,q=2;{}", rgba2x2()).as_bytes(),
            (0, 0),
            cell(),
        );
        assert_eq!(out.responses.len(), 0, "q=2 asks for silence");
        assert_eq!(g.images()[&4].width, 2);
    }

    #[test]
    fn png_derives_size_from_ihdr() {
        // minimal header: magic + IHDR with 7x5
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        png.extend_from_slice(&13u32.to_be_bytes());
        png.extend_from_slice(b"IHDR");
        png.extend_from_slice(&7u32.to_be_bytes());
        png.extend_from_slice(&5u32.to_be_bytes());
        assert_eq!(png_size(&png), (7, 5));
        let mut g = Graphics::new();
        // magic decides the format even with `f=32` lying
        let out = g.handle(
            format!("Ga=t,f=32,i=8;{}", b64(&png)).as_bytes(),
            (0, 0),
            cell(),
        );
        assert_eq!(out.responses, [b"Gi=8;OK".to_vec()]);
        let img = &g.images()[&8];
        assert_eq!((img.width, img.height, img.format), (7, 5, Format::Png));
    }

    #[test]
    fn automatic_id_when_the_app_sends_none() {
        let mut g = Graphics::new();
        let out = g.handle(
            format!("Ga=t,f=32,s=2,v=2;{}", rgba2x2()).as_bytes(),
            (0, 0),
            cell(),
        );
        assert_eq!(out.responses, [b"Gi=1;OK".to_vec()]);
        let out = g.handle(
            format!("Ga=t,f=32,s=2,v=2;{}", rgba2x2()).as_bytes(),
            (0, 0),
            cell(),
        );
        assert_eq!(out.responses, [b"Gi=2;OK".to_vec()]);
    }

    #[test]
    fn skewed_payload_answers_einval_and_keeps_nothing() {
        let mut g = Graphics::new();
        let out = g.handle(b"Ga=t,f=32,s=2,v=2,i=1;@@@@", (0, 0), cell());
        assert_eq!(out.responses, [b"Gi=1;EINVAL:base64".to_vec()]);
        let out = g.handle(b"Ga=t,f=24,o=z,s=2,v=2,i=1;AAAA", (0, 0), cell());
        assert_eq!(out.responses, [b"Gi=1;EINVAL:zlib".to_vec()]);
        // raw RGB announcing 2x2 but with 2 bytes: the size lied
        let out = g.handle(
            format!("Ga=t,f=24,s=2,v=2,i=1;{}", b64(&[1, 2])).as_bytes(),
            (0, 0),
            cell(),
        );
        assert_eq!(out.responses, [b"Gi=1;EINVAL:size".to_vec()]);
        assert!(g.images().is_empty(), "no half image");
    }

    #[test]
    fn full_screen_scrollback_goes_to_history_and_region_discards() {
        let mut g = Graphics::new();
        g.handle(
            format!("Ga=T,f=32,s=2,v=2,i=1;{}", rgba2x2()).as_bytes(),
            (0, 0),
            cell(),
        );
        assert_eq!(g.placements()[0].line, 0);
        // full screen with history: the placement goes to line -1
        g.scroll(0, 3, Some(-10));
        assert_eq!(g.placements()[0].line, -1);
        // and drops out when the history shrinks until nothing is left
        g.drop_above(-1);
        assert_eq!(g.placements().len(), 1);
        g.drop_above(0);
        assert_eq!(g.placements(), []);
        // region scroll (no history) drops what left through the top
        let mut g = Graphics::new();
        g.handle(
            format!("Ga=T,f=32,s=2,v=2,i=1;{}", rgba2x2()).as_bytes(),
            (0, 0),
            cell(),
        );
        g.scroll(0, 1, None);
        assert_eq!(g.placements(), []);
        // outside the region nothing is touched
        let mut g = Graphics::new();
        g.handle(
            format!("Ga=T,f=32,s=2,v=2,i=1;{}", rgba2x2()).as_bytes(),
            (0, 3),
            cell(),
        );
        g.scroll(0, 1, None);
        assert_eq!(g.placements()[0].line, 3);
    }

    #[test]
    fn animation_and_control_are_ignored_on_purpose() {
        // H18: `a=f`/`a=a`/`a=c` answer nothing and store nothing
        let mut g = Graphics::new();
        for action in ["f", "a", "c"] {
            let out = g.handle(
                format!("Ga={action},i=1;{}", rgba2x2()).as_bytes(),
                (0, 0),
                cell(),
            );
            assert_eq!(out.responses.len(), 0, "a={action}");
        }
        assert_eq!(g.images().len(), 0);
    }
}
