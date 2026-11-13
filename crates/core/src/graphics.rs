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
    /// Frame 1 is `data`; these are it and the ones `a=f` added after it, so
    /// `frames[0]` only ever holds the root's gap (`B2`).
    pub frames: Vec<Frame>,
    pub anim: Animation,
}

/// One rectangle of animation frame data: the bytes the client sent, where in
/// the image they go, and how they land on what is already there.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Step {
    pub data: Vec<u8>,
    pub format: Format,
    pub width: u32,
    pub height: u32,
    /// `x=`,`y=`: where the rectangle goes, in image pixels.
    pub at: (u32, u32),
    /// `X=1`: overwrite instead of alpha-blend.
    pub replace: bool,
    /// `Y=`: 32-bit RGBA that fills the canvas the data does not reach.
    pub background: u32,
    /// `c=`: the 1-based frame used as the canvas; `None` starts from the
    /// background colour (and `0` is a black, fully transparent pixel).
    pub base: Option<u32>,
}

/// One animation frame (`B2`): `steps` is everything the client painted into it,
/// in the order it arrived — `r=` appends to a frame that already exists instead
/// of creating a new one.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Frame {
    pub steps: Vec<Step>,
    /// `z=`: milliseconds before the next frame. Negative means *gapless* (the
    /// frame exists as base data but is never shown). `None` is the 40 ms default
    /// for a frame and "the root frame shows no gap" for frame 1.
    pub gap: Option<i32>,
}

/// The clock-free half of an animation: the app owns the timer, the core owns
/// which frame is current and what the gaps ask for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Animation {
    /// `c=`: 1-based frame on screen.
    pub current: u32,
    /// `s=`: `0` never controlled, `1` stopped, `2` running and waiting for more
    /// frames at the end, `3` running and looping.
    pub mode: u8,
    /// `v=`: `1` (the default) loops forever, `n` plays `n-1` loops and stops.
    pub loops: u32,
    /// How many times the last frame was reached.
    pub played: u32,
}

impl Default for Animation {
    fn default() -> Self {
        Self {
            current: 1,
            mode: 0,
            loops: 1,
            played: 0,
        }
    }
}

impl Image {
    /// A frame with no `z=` waits this long.
    const DEFAULT_GAP: u32 = 40;

    #[must_use]
    pub fn frame_count(&self) -> u32 {
        u32::try_from(self.frames.len().max(1)).unwrap_or(u32::MAX)
    }

    /// The frame on screen, clamped to what exists (a client can `d=f` the frames
    /// out from under the current one).
    #[must_use]
    pub fn current(&self) -> u32 {
        self.anim.current.clamp(1, self.frame_count())
    }

    /// Milliseconds before the next frame, or `None` when this frame is skipped
    /// over (gapless) or is the only one.
    #[must_use]
    pub fn gap_ms(&self, frame: u32) -> Option<u32> {
        if self.frame_count() < 2 {
            return None;
        }
        let gap = self
            .frames
            .get(usize::try_from(frame.saturating_sub(1)).ok()?)
            .and_then(|f| f.gap);
        match gap {
            // negative is gapless: the frame is base data, never presented
            Some(g) if g < 0 => None,
            // `z=0` is "unspecified" for the spec, and so is a frame with no `z=`
            Some(0) | None => Some(Self::DEFAULT_GAP),
            Some(g) => u32::try_from(g).ok(),
        }
    }
}

/// What is drawn right now. `linha` is a **screen line**: 0..rows-1 is the live
/// grid, and negative enters the history (-1 = last scrollback line). So
/// scrolling is subtracting 1 from everybody, which is what the content does.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct Placement {
    pub image_id: u32,
    /// `p=`: the placement's own id inside the image. `0` = the image has one
    /// placement (the common case); a program that puts the same image in two
    /// places gives each one a number, and `d=p` addresses them one by one.
    pub id: u32,
    /// `z=`: negative draws **behind** the text, positive in front. Ties keep the
    /// order they were placed in.
    pub z: i32,
    pub col: u16,
    pub line: i32,
    pub cols: u16,
    pub rows: u16,
    /// `X=`/`Y=`: offset in **pixels** inside the first cell. That is how
    /// `icat` centers: it walks whole cells and leaves the remainder here.
    pub dx: i32,
    pub dy: i32,
    /// `x=`/`y=`/`w=`/`h=`: the rectangle of the **source** to show, in image
    /// pixels — the whole image when the keys are absent. The cells (`c`/`r`)
    /// are the destination; this is what is cropped out of the picture.
    pub crop: (u32, u32, u32, u32),
}

impl Placement {
    /// The `uv` rectangle this placement samples: the crop as a fraction of the
    /// source, which is all a texture sample needs.
    #[must_use]
    pub fn uv(&self, source: (u32, u32)) -> [f32; 4] {
        let full = (source.0.max(1), source.1.max(1));
        let (x, y, width, height) = self.crop;
        #[allow(clippy::cast_precision_loss)] // both sides are bounded by 2^32
        let part = |v: u32, of: u32| v.min(of) as f32 / of as f32;
        [
            part(x, full.0),
            part(y, full.1),
            part(x.saturating_add(width), full.0),
            part(y.saturating_add(height), full.1),
        ]
    }
}

/// A transmission by file/temp/shared-memory (`B1`): what the app must load.
///
/// The payload of `t=f`/`t=t` is a path, not pixels; the payload of `t=s`
/// is a POSIX shared-memory NAME, resolved here to `/dev/shm/<name>` — the
/// app only ever opens `FileRequest::path`. The core keeps the request's
/// context (who asked, where to show) and the app answers with
/// `Graphics::provide` — or the image stays missing and the child gets
/// `ENOENT`. Shell expansion (`~`) belongs to the app: the path here is raw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileUse {
    /// `a=t`: transmit only.
    Transmit,
    /// `a=T`: transmit and place the image once it is loaded.
    Display,
    /// `a=q`: the child asks whether the file could be sent, so the bytes are
    /// read and answered — not stored (the spec's query "will try to load the
    /// image and respond with either OK or an error").
    Query,
}

#[derive(Clone, Debug)]
pub struct FileRequest {
    pub path: String,
    /// `t=t`: the file is temporary — the app deletes it after reading.
    /// `t=s` reads shared memory and never deletes.
    pub delete: bool,
    pub usage: FileUse,
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
    /// Something the child asked for that is not going to happen, with the
    /// reason — the app shows it once (`B3`). No reply goes back: the protocol
    /// that asks this way has no error channel.
    pub refused: Option<&'static str>,
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

    /// `None`: the source rectangle does not fit the image (nothing is placed).
    fn place(
        &mut self,
        img: &Image,
        kv: &HashMap<String, String>,
        cursor: (u16, u16),
        cell: (u32, u32),
    ) -> Option<u16> {
        let cw = cell.0.max(1);
        let ch = cell.1.max(1);
        let num = |k: &str| kv.get(k).and_then(|v| v.parse::<i64>().ok());
        // `x`/`y`/`w`/`h`: the source rectangle, in image pixels (`x`/`y` are the
        // offset, `w`/`h` how much of it). Absent keys mean the whole image, and
        // a rectangle that leaves it is refused rather than clamped: a caller
        // that got the geometry wrong should hear about it.
        let (iw, ih) = (img.width.max(1), img.height.max(1));
        let (x, y) = (num("x").unwrap_or(0), num("y").unwrap_or(0));
        let (w, h) = (
            num("w").unwrap_or(i64::from(iw)),
            num("h").unwrap_or(i64::from(ih)),
        );
        if x < 0 || y < 0 || w <= 0 || h <= 0 {
            return None;
        }
        let (x, y) = (u32::try_from(x).ok()?, u32::try_from(y).ok()?);
        let (w, h) = (u32::try_from(w).ok()?, u32::try_from(h).ok()?);
        if x.saturating_add(w) > iw || y.saturating_add(h) > ih {
            return None;
        }
        let cols = num("c").map_or_else(
            || u16::try_from(w.div_ceil(cw)).unwrap_or(u16::MAX).max(1),
            |c| u16::try_from(c.max(1)).unwrap_or(u16::MAX),
        );
        let rows = num("r").map_or_else(
            || u16::try_from(h.div_ceil(ch)).unwrap_or(u16::MAX).max(1),
            |r| u16::try_from(r.max(1)).unwrap_or(u16::MAX),
        );
        let pid = u32::try_from(num("p").unwrap_or(0)).unwrap_or(0);
        let depth = i32::try_from(num("z").unwrap_or(0)).unwrap_or(0);
        // a placement id is unique per image: reusing it moves that placement
        // instead of adding another one
        if pid != 0 {
            self.placements
                .retain(|p| !(p.image_id == img.id && p.id == pid));
        }
        self.placements.push(Placement {
            image_id: img.id,
            id: pid,
            z: depth,
            col: cursor.0,
            line: i32::from(cursor.1),
            cols,
            rows,
            dx: i32::try_from(num("X").unwrap_or(0)).unwrap_or(0),
            dy: i32::try_from(num("Y").unwrap_or(0)).unwrap_or(0),
            crop: (x, y, w, h),
        });
        Some(rows)
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
        if (action == "t" || action == "T" || action == "q")
            && matches!(medium, "f" | "t" | "s")
            && !(action == "q" && data.is_empty())
        {
            // the payload names a file — the app reads it and answers with
            // `provide`. No response yet: `OK`/`ENOENT` comes with the bytes
            // (or without them). A query with no payload at all carries no
            // name: it is the capability probe and stays one.
            let path = if medium == "s" {
                // `t=s` is a shared-memory NAME, not a path (`kitten icat`
                // sends `icat-XXX` and waits): it lives in `/dev/shm`.
                // Opened as-is it resolved against the app cwd and every
                // shm transmission answered `ENOENT`. A slash in it is not
                // a name — refuse instead of loading an escaped path.
                let name = String::from_utf8_lossy(&data);
                let name = name.strip_prefix('/').unwrap_or(&name);
                if name.contains('/') {
                    return Self::error(&kv, "EINVAL:shm name");
                }
                format!("/dev/shm/{name}")
            } else {
                String::from_utf8_lossy(&data).into_owned()
            };
            return Output {
                file: Some(FileRequest {
                    path,
                    delete: medium == "t",
                    usage: match action {
                        "T" => FileUse::Display,
                        "q" => FileUse::Query,
                        _ => FileUse::Transmit,
                    },
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
            "f" => self.frame(kv, data, ident, quiet),
            "a" => self.anim(kv, ident, quiet),
            // frame composition (`a=c`): a real feature of the spec, and one no
            // client here used. Refusing beats pretending.
            "c" => Self::error(kv, "EINVAL:a=c not implemented"),
            "q" => {
                // `a=q` is a **capability** probe, not an image query: `kitten icat`
                // sends three (direct, file, memory) and only proceeds if it gets
                // OK on the direct transmission. Answering ENOENT because the id
                // does not exist makes icat give up with "terminal does not
                // support the graphics protocol". The two that name a file are
                // read before they get here (`B59`), and answer by the same rule.
                let name = Self::query_verdict(kv, None);
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
                // the spec's delete kinds: `i` an image with its placements, `p`
                // one placement, `P` every placement of the image, `a` everything.
                // Anything else deletes everything, which is what the spec says a
                // terminal that does not know the kind should do.
                let como = param("d").unwrap_or("a");
                let pid = param("p").and_then(|p| p.parse::<u32>().ok());
                match como {
                    "i" => {
                        self.images.remove(&ident);
                        self.placements.retain(|p| p.image_id != ident);
                    },
                    "p" => {
                        self.placements
                            .retain(|p| !(p.image_id == ident && Some(p.id) == pid));
                    },
                    "P" => self.placements.retain(|p| p.image_id != ident),
                    // `d=f`: the frames go, the image and its placements stay
                    "f" | "F" => {
                        if let Some(img) = self.images.get_mut(&ident) {
                            img.frames.truncate(1);
                            img.anim = Animation::default();
                        }
                    },
                    _ => {
                        self.placements.clear();
                        self.images.clear();
                    },
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
                    let Some(rows) = self.place(&img, kv, cursor, cell) else {
                        return Self::error(kv, "EINVAL:crop outside the image");
                    };
                    Output {
                        responses: if quiet {
                            Vec::new()
                        } else {
                            vec![format!("Gi={ident};OK").into_bytes()]
                        },
                        lines: rows,
                        move_cursor: param("C") != Some("1"),
                        ..Output::default()
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
        let Some((width, height, fmt)) = Self::image_size(kv, data) else {
            return Self::error(kv, "EINVAL:size");
        };
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
            frames: vec![Frame::default()],
            anim: Animation::default(),
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
        if kv.get("a").is_some_and(|a| a == "T") {
            match self.place(&img, kv, cursor, cell) {
                Some(rows) => {
                    out.lines = rows;
                    out.move_cursor = kv.get("C").is_none_or(|c| c != "1");
                },
                // the image was stored (it is what `transmit` does); only the
                // placement of a rectangle that does not fit is refused
                None => {
                    out.responses = if quiet {
                        Vec::new()
                    } else {
                        vec![format!("Gi={id};EINVAL:crop outside the image").into_bytes()]
                    };
                },
            }
        }
        out
    }

    /// `a=f` (`B2`): the payload is a rectangle of an animation frame, not an
    /// image of its own. Without `r=` it becomes the next frame; with `r=` it is
    /// painted into a frame that already exists. `r=1` is refused: the root frame
    /// is `Image.data` itself, and rewriting it would mean composing pixels here,
    /// in the core, which has no PNG decoder.
    fn frame(
        &mut self,
        kv: &HashMap<String, String>,
        data: &[u8],
        ident: u32,
        quiet: bool,
    ) -> Output {
        let param = |k: &str| kv.get(k).map(String::as_str);
        let number = |k: &str| {
            param(k)
                .and_then(|v| v.parse::<u32>().ok())
                .filter(|n| *n > 0)
        };
        let Some((width, height, fmt)) = Self::image_size(kv, data) else {
            return Self::error(kv, "EINVAL:size");
        };
        let edit = number("r");
        if edit == Some(1) {
            return Self::error(kv, "EINVAL:r=1");
        }
        if ident == 0 {
            return Self::error(kv, "EINVAL:i needed");
        }
        let format = match fmt {
            100 => Format::Png,
            24 => Format::Rgb,
            _ => Format::Rgba,
        };
        let step = Step {
            data: data.to_vec(),
            format,
            width,
            height,
            at: (number("x").unwrap_or(0), number("y").unwrap_or(0)),
            replace: param("X") == Some("1"),
            background: number("Y").unwrap_or(0),
            base: number("c"),
        };
        let gap = param("z").and_then(|z| z.parse::<i32>().ok());
        let Some(img) = self.images.get_mut(&ident) else {
            return Self::error(kv, "ENOENT:no such image");
        };
        let target = if let Some(r) = edit {
            let Some(f) = img.frames.get_mut(r as usize - 1) else {
                return Self::error(kv, "ENOENT:no such frame");
            };
            f.steps.push(step);
            r
        } else {
            img.frames.push(Frame {
                steps: vec![step],
                gap: None,
            });
            img.frame_count()
        };
        if let Some(gap) = gap {
            // the gap belongs to the frame the data landed in, existing or new
            if let Some(f) = img.frames.get_mut(target as usize - 1) {
                f.gap = Some(gap);
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
    }

    /// `a=a` (`B2`): stop, run, jump to a frame, set a gap, set the loop count.
    /// Nothing here needs a clock — the app decides when to ask for the next frame.
    fn anim(&mut self, kv: &HashMap<String, String>, ident: u32, quiet: bool) -> Output {
        let param = |k: &str| kv.get(k).map(String::as_str);
        let number = |k: &str| {
            param(k)
                .and_then(|v| v.parse::<u32>().ok())
                .filter(|n| *n > 0)
        };
        if ident == 0 {
            return Self::error(kv, "EINVAL:i needed");
        }
        let Some(img) = self.images.get_mut(&ident) else {
            return Self::error(kv, "ENOENT:no such image");
        };
        let count = img.frame_count();
        if let Some(z) = param("z").and_then(|z| z.parse::<i32>().ok()) {
            let at = number("r").unwrap_or(img.anim.current).clamp(1, count);
            if let Some(f) = img.frames.get_mut(at as usize - 1) {
                f.gap = Some(z);
            }
        }
        match number("s") {
            // both ways of running start over, which is what the spec's loop
            // counter reset means (`s=1` resets it too)
            Some(2 | 3) => {
                img.anim.mode = if number("s") == Some(2) { 2 } else { 3 };
                img.anim.played = 0;
                img.anim.current = 1;
            },
            Some(1) => img.anim.mode = 1,
            _ => {},
        }
        if let Some(v) = number("v") {
            img.anim.loops = v;
        }
        if let Some(c) = number("c") {
            img.anim.current = c.clamp(1, count);
        }
        Output {
            responses: if quiet {
                Vec::new()
            } else {
                vec![format!("Gi={ident};OK").into_bytes()]
            },
            ..Output::default()
        }
    }

    /// One tick of an animation (`B2`): the app's clock asked for the next frame.
    /// Answers `false` when nothing changed — stopped, only the root frame, or
    /// loading and already on the last frame.
    pub fn advance(&mut self, id: u32) -> bool {
        let Some(img) = self.images.get_mut(&id) else {
            return false;
        };
        let count = img.frame_count();
        // `s=0` was never started and `s=1` was stopped
        if img.anim.mode < 2 || count < 2 {
            return false;
        }
        let mut next = img.current() + 1;
        if next > count {
            if img.anim.mode == 2 {
                return false; // loading: the last frame waits for more to arrive
            }
            img.anim.played += 1;
            if img.anim.loops > 1 && img.anim.played + 1 >= img.anim.loops {
                img.anim.mode = 1;
                img.anim.current = count;
                return true;
            }
            next = 1;
        }
        // a gapless frame is base data, never a picture: step over it
        let mut hops = 0;
        while hops <= count && img.frames[next as usize - 1].gap.is_some_and(|g| g < 0) {
            next = if next >= count { 1 } else { next + 1 };
            hops += 1;
        }
        img.anim.current = next;
        true
    }

    /// The size `s=`/`v=`/`f=` and the bytes agree on — `None` when they do not
    /// (`EINVAL:size` for a transmission, the query's error word for a probe).
    fn image_size(kv: &HashMap<String, String>, data: &[u8]) -> Option<(u32, u32, u32)> {
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
                return None;
            }
            (w, h)
        };
        if width == 0 || height == 0 {
            return None;
        }
        Some((width, height, fmt))
    }

    /// `a=q` answers one of two words, and both who ask get the same rule: the
    /// medium and the format have to be ones we take, and — with `data` — the
    /// bytes have to be the size they claim.
    fn query_verdict(kv: &HashMap<String, String>, data: Option<&[u8]>) -> &'static str {
        let param = |k: &str| kv.get(k).map(String::as_str);
        let medium = param("t").unwrap_or("d");
        let fmt = param("f").and_then(|f| f.parse::<u32>().ok()).unwrap_or(32);
        let ok = matches!(medium, "d" | "f" | "t" | "s")
            && [24, 32, 100].contains(&fmt)
            && match data {
                None => true,
                Some(d) => Self::image_size(kv, d).is_some(),
            };
        if ok {
            "OK"
        } else {
            "EINVAL:unsupported transmission"
        }
    }

    /// Completes a file transmission (`B1`): the app loaded `req.path` and
    /// hands the bytes over — from here it is a direct transmission, same
    /// checks, same answers. Oversize answers like an overlong chunked one.
    pub fn provide(&mut self, req: &FileRequest, data: &[u8]) -> Output {
        if data.len() > MAX_APC {
            return Self::error(&req.kv, "EINVAL:too big");
        }
        if req.usage == FileUse::Query {
            // `B59`: the bytes prove the file could be sent and nothing is
            // stored — an id with no image behind it would leak a broken frame
            return Output {
                responses: if req.ident == 0 {
                    Vec::new()
                } else {
                    vec![format!(
                        "Gi={};{}",
                        req.ident,
                        Self::query_verdict(&req.kv, Some(data))
                    )
                    .into_bytes()]
                },
                ..Output::default()
            };
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

    /// iTerm2's inline image (`OSC 1337;File=…:base64`) — the half of `B3` that
    /// reuses the direct transmission, since the bytes are the same PNG.
    ///
    /// `inline=0` is the protocol's **default**, and it asks the terminal to
    /// *write the file into the cwd*: the bytes come from the child, so the
    /// terminal does not write them anywhere — it refuses and says so.
    pub fn iterm(
        &mut self,
        arg: &[u8],
        cursor: (u16, u16),
        cell: (u32, u32),
        grid: (u16, u16),
    ) -> Output {
        let Some(rest) = arg.strip_prefix(b"File=") else {
            return Output::default();
        };
        // the payload follows the **last** colon: a `name=` value may carry a
        // colon of its own, the base64 alphabet never does
        let Some(colon) = rest.iter().rposition(|&b| b == b':') else {
            return Output::default();
        };
        let mut kv: HashMap<String, String> = HashMap::new();
        for item in String::from_utf8_lossy(&rest[..colon]).split(';') {
            if let Some((k, v)) = item.split_once('=') {
                kv.insert(k.trim().to_string(), v.trim().to_string());
            }
        }
        if kv.get("inline").is_none_or(|v| v != "1") {
            return Output {
                refused: Some("inline=0 asks for a file on disk — refused"),
                ..Output::default()
            };
        }
        let Some(data) = base64_decode(&String::from_utf8_lossy(&rest[colon + 1..])) else {
            return Output {
                refused: Some("the payload is not base64"),
                ..Output::default()
            };
        };
        if data.len() > MAX_APC {
            return Output {
                refused: Some("the image is above the 64 MiB cap"),
                ..Output::default()
            };
        }
        // the full-size cells are what the placement counts in, so a bare
        // `width`/`height` means the terminal's cells
        let cw = cell.0.max(1);
        let ch = cell.1.max(1);
        let cells = |v: Option<&str>, total: u16, px: u32, image_px: u32| -> Option<u16> {
            let v = v?.trim();
            if v.is_empty() || v == "auto" {
                return None; // the image's own size, as `place` computes it
            }
            let split = v
                .find(|c: char| !(c.is_ascii_digit() || c == '.'))
                .unwrap_or(v.len());
            let n: f64 = v[..split].parse().ok()?;
            let unit = &v[split..];
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let out = match unit {
                "" => n.max(1.0) as u32, // cells
                "px" => n.max(1.0).div_euclid(f64::from(px)).max(1.0) as u32,
                "%" => (n * f64::from(total) / 100.0).ceil().max(1.0) as u32,
                _ => image_px.div_ceil(px).max(1), // not ours
            };
            u16::try_from(out).ok()
        };
        let (iw, ih) = Self::image_size(&kv, &data).map_or((0, 0), |(w, h, _)| (w, h));
        // a display transmission, without the kitty reply and without moving the
        // cursor: iTerm2 draws over the cells and the program's own newline
        // decides where the text goes on
        kv.insert("a".to_string(), "T".to_string());
        kv.insert("C".to_string(), "1".to_string());
        if let Some(c) = cells(kv.get("width").map(String::as_str), grid.0, cw, iw) {
            kv.insert("c".to_string(), c.to_string());
        }
        if let Some(r) = cells(kv.get("height").map(String::as_str), grid.1, ch, ih) {
            kv.insert("r".to_string(), r.to_string());
        }
        if Self::image_size(&kv, &data).is_none() && !data.starts_with(b"\x89PNG") {
            return Output {
                refused: Some("the image is not the size it claims"),
                ..Output::default()
            };
        }
        self.transmit(&kv, &data, 0, true, cursor, cell)
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
pub(crate) fn base64_decode(text: &str) -> Option<Vec<u8>> {
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
    use super::{png_size, FileUse, Format, Graphics};
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

    /// `a=f` adds frames to an image and `a=a` says what to do with them; the
    /// clock is the app's, so `advance` is what stands in for it.
    #[test]
    fn animation_frames_arrive_and_advance_until_stopped() {
        let mut g = Graphics::new();
        let body = format!("Ga=T,f=32,s=2,v=2,i=7;{}", rgba2x2());
        g.handle(body.as_bytes(), (0, 0), cell());
        assert_eq!(g.images()[&7].frame_count(), 1, "an image is one frame");
        // two more frames, the second one only a 1x1 patch over the first
        let one = b64(&[1, 2, 3, 255]);
        let frame = format!("Ga=f,i=7,f=32,s=2,v=2,z=25;{}", rgba2x2());
        assert_eq!(
            g.handle(frame.as_bytes(), (0, 0), cell()).responses,
            [b"Gi=7;OK".to_vec()]
        );
        let patch = format!("Ga=f,i=7,f=32,s=1,v=1,x=1,y=1,X=1;{one}");
        g.handle(patch.as_bytes(), (0, 0), cell());
        let img = &g.images()[&7];
        assert_eq!(img.frame_count(), 3);
        assert_eq!(img.frames[2].steps[0].at, (1, 1));
        assert!(img.frames[2].steps[0].replace);
        // `s=3` runs from the top, looping
        g.handle(b"Ga=a,i=7,s=3", (0, 0), cell());
        assert_eq!(g.images()[&7].current(), 1);
        assert!(g.advance(7));
        assert_eq!(g.images()[&7].current(), 2);
        assert!(g.advance(7));
        assert_eq!(g.images()[&7].current(), 3);
        assert!(g.advance(7), "s=3 loops back to the first frame");
        assert_eq!(g.images()[&7].current(), 1);
        // the gap of the second frame is the one `z=25` set
        assert_eq!(g.images()[&7].gap_ms(2), Some(25));
        assert_eq!(g.images()[&7].gap_ms(1), Some(40), "the 40 ms default");
        // `s=1` stops where it is
        g.handle(b"Ga=a,i=7,s=1,c=2", (0, 0), cell());
        assert_eq!(g.images()[&7].current(), 2);
        assert!(!g.advance(7));
        assert_eq!(g.images()[&7].current(), 2);
    }

    /// A gapless frame (`z<0`) is base data: it is never the frame on screen.
    #[test]
    fn a_gapless_frame_is_stepped_over() {
        let mut g = Graphics::new();
        let body = format!("Ga=T,f=32,s=2,v=2,i=9;{}", rgba2x2());
        g.handle(body.as_bytes(), (0, 0), cell());
        for gap in [-1, 30] {
            let f = format!("Ga=f,i=9,f=32,s=2,v=2,z={gap};{}", rgba2x2());
            g.handle(f.as_bytes(), (0, 0), cell());
        }
        g.handle(b"Ga=a,i=9,s=3", (0, 0), cell());
        assert!(g.advance(9));
        assert_eq!(g.images()[&9].current(), 3, "frame 2 is never shown");
        assert_eq!(g.images()[&9].gap_ms(2), None);
    }

    /// `v=2` plays the animation once and stops; `s=2` (loading) waits instead of
    /// looping when it reaches the end.
    #[test]
    fn loops_and_loading_mode_stop_on_the_last_frame() {
        let mut g = Graphics::new();
        let body = format!("Ga=T,f=32,s=2,v=2,i=4;{}", rgba2x2());
        g.handle(body.as_bytes(), (0, 0), cell());
        let f = format!("Ga=f,i=4,f=32,s=2,v=2;{}", rgba2x2());
        g.handle(f.as_bytes(), (0, 0), cell());
        // loading: on the last frame it waits for more to arrive
        g.handle(b"Ga=a,i=4,s=2", (0, 0), cell());
        assert!(g.advance(4));
        assert!(!g.advance(4), "s=2 does not wrap");
        assert_eq!(g.images()[&4].current(), 2);
        // v=2: one loop, then it stops on its own
        g.handle(b"Ga=a,i=4,s=3,v=2", (0, 0), cell());
        assert_eq!(g.images()[&4].current(), 1, "s=3 starts over");
        assert!(g.advance(4));
        assert_eq!(g.images()[&4].current(), 2);
        assert!(g.advance(4), "the end of the one loop it plays");
        assert_eq!(g.images()[&4].current(), 2, "it stops on the last frame");
        assert!(!g.advance(4), "v=2 has played its loops");
    }

    /// What the core does not implement it says so: `a=c` and `r=1` answer with a
    /// reason instead of silently doing nothing.
    #[test]
    fn frame_composition_and_root_edits_are_refused() {
        let mut g = Graphics::new();
        let body = format!("Ga=T,f=32,s=2,v=2,i=2;{}", rgba2x2());
        g.handle(body.as_bytes(), (0, 0), cell());
        let compose = g.handle(b"Ga=c,i=2,r=1,c=2,w=1,h=1", (0, 0), cell());
        assert_eq!(
            compose.responses,
            [b"Gi=2;EINVAL:a=c not implemented".to_vec()]
        );
        let edit = format!("Ga=f,i=2,r=1,f=32,s=2,v=2;{}", rgba2x2());
        assert_eq!(
            g.handle(edit.as_bytes(), (0, 0), cell()).responses,
            [b"Gi=2;EINVAL:r=1".to_vec()]
        );
        // a frame for an image nobody sent
        let orphan = format!("Ga=f,i=99,f=32,s=2,v=2;{}", rgba2x2());
        assert_eq!(
            g.handle(orphan.as_bytes(), (0, 0), cell()).responses,
            [b"Gi=99;ENOENT:no such image".to_vec()]
        );
        // `d=f` drops the frames and keeps the image
        let f = format!("Ga=f,i=2,f=32,s=2,v=2;{}", rgba2x2());
        g.handle(f.as_bytes(), (0, 0), cell());
        assert_eq!(g.images()[&2].frame_count(), 2);
        g.handle(b"Ga=d,d=f,i=2", (0, 0), cell());
        assert_eq!(g.images()[&2].frame_count(), 1);
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

    /// An `a=q` that names a file: what `kitten icat`'s detection sends before
    /// it trusts the terminal with a real transmission (`B59`).
    fn query_body(t: &str, f: u32, s: u32, v: u32) -> Vec<u8> {
        let path = b64(b"/tmp/x.rgba");
        format!("Ga=q,f={f},s={s},v={v},t={t},i=21;{path}").into_bytes()
    }

    /// `t=s` carries a shared-memory name, not a path.
    fn shm_body(name: &str, action: &str) -> Vec<u8> {
        let name = b64(name.as_bytes());
        format!("Ga={action},f=100,t=s,i=7;{name}").into_bytes()
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
        assert_eq!(req.usage, FileUse::Display, "a=T also places");
        assert_eq!((req.ident, req.cursor, req.quiet), (7, (1, 2), false));
    }

    #[test]
    fn temp_marks_for_delete_and_transmit_only_does_not_place() {
        let mut g = Graphics::new();
        let out = g.handle(&file_body("t", "t"), (0, 0), cell());
        let req = out.file.expect("t=t also waits");
        assert!(req.delete, "t=t deletes after reading");
        assert_eq!(req.usage, FileUse::Transmit, "a=t only transmits");
        let mut g = Graphics::new();
        let out = g.handle(&shm_body("icat-ABC", "T"), (0, 0), cell());
        assert!(!out.file.expect("t=s waits").delete, "shm never deletes");
    }

    #[test]
    fn shm_name_resolves_under_dev_shm() {
        let mut g = Graphics::new();
        // what `kitten icat` sends: a bare name, not a path. Opened as-is
        // it resolved against the app cwd and every shm transmission
        // answered `ENOENT` with the file sitting in `/dev/shm`.
        let out = g.handle(&shm_body("icat-ABC", "T"), (1, 2), cell());
        assert!(out.responses.is_empty(), "no verdict before the bytes");
        let req = out.file.expect("the shm waits for the app");
        assert_eq!(req.path, "/dev/shm/icat-ABC");
        assert!(!req.delete, "shm never deletes");
        assert_eq!(req.usage, FileUse::Display, "a=T also places");
        assert_eq!((req.ident, req.cursor), (7, (1, 2)));
    }

    #[test]
    fn shm_name_with_a_slash_is_not_a_path() {
        let mut g = Graphics::new();
        // `../../x` under `/dev/shm` escapes it: refuse instead of loading.
        let out = g.handle(&shm_body("../../x", "T"), (0, 0), cell());
        assert!(out.file.is_none(), "no load of an escaped name");
        assert_eq!(out.responses, [b"Gi=7;EINVAL:shm name".to_vec()]);
        assert!(g.images().is_empty());
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

    /// `a=q` over a file medium reads the file and answers, and stores nothing:
    /// the app is the one that opens it, so the verdict only comes back through
    /// `provide`. Before `B59` the query fell through to the capability probe
    /// and answered `OK` for a file that was never opened.
    #[test]
    fn a_query_over_a_file_loads_it_and_stores_nothing() {
        let mut g = Graphics::new();
        let out = g.handle(&query_body("f", 24, 1, 1), (0, 0), cell());
        assert!(out.responses.is_empty(), "the verdict waits for the bytes");
        let req = out.file.expect("the query names a file too");
        assert_eq!(
            req.usage,
            FileUse::Query,
            "the bytes answer instead of filling an image"
        );
        assert!(!req.delete, "a query does not delete");
        let out = g.provide(&req, &[1, 2, 3]);
        assert_eq!(out.responses, [b"Gi=21;OK".to_vec()]);
        assert!(g.images().is_empty(), "a query stores no image");
        assert!(g.placements().is_empty(), "and places nothing");
    }

    #[test]
    fn a_query_over_a_missing_file_answers_enoent() {
        let mut g = Graphics::new();
        let req = g
            .handle(&query_body("f", 24, 1, 1), (0, 0), cell())
            .file
            .expect("request");
        let out = Graphics::missing(&req);
        assert_eq!(out.responses, [b"Gi=21;ENOENT".to_vec()]);
        assert!(g.images().is_empty());
    }

    /// A query whose bytes are not the size they claim is refused: the client
    /// must not go on to send a real transmission the terminal would refuse.
    #[test]
    fn a_query_over_data_of_another_size_is_refused() {
        let mut g = Graphics::new();
        let req = g
            .handle(&query_body("f", 24, 1, 1), (0, 0), cell())
            .file
            .expect("request");
        let out = g.provide(&req, &[1, 2]);
        assert_eq!(
            out.responses,
            [b"Gi=21;EINVAL:unsupported transmission".to_vec()]
        );
    }

    /// The temp medium is the `t=t` probe `icat` sends for real: the request
    /// carries the delete, so the app removes the file it was told to read.
    #[test]
    fn a_query_marked_temp_asks_for_the_delete() {
        let mut g = Graphics::new();
        let req = g
            .handle(&query_body("t", 24, 1, 1), (0, 0), cell())
            .file
            .expect("request");
        assert!(req.delete, "t=t is ours to remove, query or not");
        assert_eq!(req.usage, FileUse::Query);
        let out = g.provide(&req, &[1, 2, 3]);
        assert_eq!(out.responses, [b"Gi=21;OK".to_vec()]);
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

    /// A PNG header with the given size — enough for the size the payload
    /// claims; the pixels are the app's business.
    fn png(w: u32, h: u32) -> Vec<u8> {
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        png.extend_from_slice(&13u32.to_be_bytes());
        png.extend_from_slice(b"IHDR");
        png.extend_from_slice(&w.to_be_bytes());
        png.extend_from_slice(&h.to_be_bytes());
        png
    }

    #[test]
    fn crop_shows_only_the_source_rectangle() {
        // `x`/`y`/`w`/`h`: the rectangle of the picture to show, and the cells
        // follow it — a 20x19 image cropped to its bottom-right quarter is
        // 1x1 cell in a 10x19 cell, and the uv is that quarter
        let mut g = Graphics::new();
        let body = format!("Ga=T,f=100,i=3,x=10,y=9,w=10,h=10;{}", b64(&png(20, 19)));
        let out = g.handle(body.as_bytes(), (0, 0), cell());
        assert_eq!(out.responses, [b"Gi=3;OK".to_vec()]);
        let p = &g.placements()[0];
        assert_eq!(p.crop, (10, 9, 10, 10));
        assert_eq!((p.cols, p.rows), (1, 1));
        let uv = p.uv((20, 19));
        assert!((uv[0] - 0.5).abs() < 1e-6 && (uv[1] - 9.0 / 19.0).abs() < 1e-6);
        assert!((uv[2] - 1.0).abs() < 1e-6 && (uv[3] - 1.0).abs() < 1e-6);
        // no keys at all: the whole picture, `uv` = the full texture
        let mut g = Graphics::new();
        let body = format!("Ga=T,f=100,i=4;{}", b64(&png(20, 19)));
        let _ = g.handle(body.as_bytes(), (0, 0), cell());
        let p = &g.placements()[0];
        assert_eq!(p.crop, (0, 0, 20, 19));
        assert_eq!(p.uv((20, 19)), [0.0, 0.0, 1.0, 1.0]);
    }

    #[test]
    fn crop_outside_the_image_is_refused() {
        // a rectangle that leaves the picture is a caller mistake: the image is
        // stored (the transmission happened) and the placement is refused
        let mut g = Graphics::new();
        for bad in ["x=10,w=20", "y=19,h=1", "w=0", "x=-1", "h=99"] {
            let body = format!("Ga=T,f=100,i=5,{bad};{}", b64(&png(20, 19)));
            let out = g.handle(body.as_bytes(), (0, 0), cell());
            assert_eq!(
                out.responses,
                [b"Gi=5;EINVAL:crop outside the image".to_vec()],
                "{bad} should be refused"
            );
            assert_eq!(out.lines, 0, "{bad} must not move the cursor");
        }
        assert_eq!(g.images().len(), 1, "the bytes were stored once");
        assert_eq!(g.placements().len(), 0);
    }

    #[test]
    fn iterm_inline_places_the_image_at_the_cursor() {
        // `B3`: the iTerm2 payload is a PNG in base64, `inline=1`, and it lands
        // on the cell the cursor was on — the same placements list as kitty
        let mut g = Graphics::new();
        let body = format!(
            "File=name={};inline=1;width=2;height=1:{}",
            b64(b"x.png"),
            b64(&png(20, 19))
        );
        let out = g.iterm(body.as_bytes(), (3, 4), cell(), (80, 24));
        assert!(out.responses.is_empty(), "iTerm2 has no reply channel");
        assert_eq!(out.refused, None);
        assert!(!out.move_cursor, "the program's own newline decides");
        let p = g.placements();
        assert_eq!(p.len(), 1);
        assert_eq!((p[0].col, p[0].line, p[0].cols, p[0].rows), (3, 4, 2, 1));
    }

    #[test]
    fn iterm_width_in_px_and_percent_resolves_against_the_cell_and_the_grid() {
        let mut g = Graphics::new();
        // 20 px in a 10 px cell = 2 cells; 50% of 80 columns = 40
        let body = format!("File=inline=1;width=20px;height=50%:{}", b64(&png(20, 19)));
        let out = g.iterm(body.as_bytes(), (0, 0), cell(), (80, 24));
        assert_eq!(out.refused, None);
        let p = g.placements();
        assert_eq!((p[0].cols, p[0].rows), (2, 12));
        // `auto` is the image's own size in cells: 20x19 px in 10x19
        let body = format!("File=inline=1;width=auto;height=auto:{}", b64(&png(20, 19)));
        let _ = g.iterm(body.as_bytes(), (0, 0), cell(), (80, 24));
        let p = g.placements();
        assert_eq!((p[1].cols, p[1].rows), (2, 1));
    }

    #[test]
    fn iterm_without_inline_is_refused_and_stores_nothing() {
        // `inline=0` (the protocol's default) asks the terminal to write the file
        // into the cwd: the bytes come from the child, so it does not happen
        let mut g = Graphics::new();
        let body = format!("File=name={};inline=0:{}", b64(b"a.png"), b64(&png(4, 4)));
        let out = g.iterm(body.as_bytes(), (0, 0), cell(), (80, 24));
        assert!(out.refused.is_some());
        assert!(g.images().is_empty());
        assert_eq!(g.placements(), []);
        // and a payload that is not base64 is refused too, not stored
        let out = g.iterm(b"File=inline=1:@@@", (0, 0), cell(), (80, 24));
        assert!(out.refused.is_some());
        assert!(g.images().is_empty());
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
    fn animation_actions_answer_instead_of_being_ignored() {
        // H18 was "animation is ignored on purpose"; `B2` is the other side of it:
        // the three actions answer, and none of them invents an image
        let mut g = Graphics::new();
        let body = format!("Ga=f,i=1,f=32,s=2,v=2;{}", rgba2x2());
        let out = g.handle(body.as_bytes(), (0, 0), cell());
        assert_eq!(out.responses, [b"Gi=1;ENOENT:no such image".to_vec()]);
        let out = g.handle(b"Ga=a,i=1,s=3", (0, 0), cell());
        assert_eq!(out.responses, [b"Gi=1;ENOENT:no such image".to_vec()]);
        let out = g.handle(b"Ga=c,i=1,r=1,c=2", (0, 0), cell());
        assert_eq!(out.responses, [b"Gi=1;EINVAL:a=c not implemented".to_vec()]);
        assert_eq!(g.images().len(), 0);
    }
}
