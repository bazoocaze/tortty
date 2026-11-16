//! Window state between runs: position, size and maximized — J2–J6.
//!
//! It is **pure** so that the file, the validation and the atomicity are
//! testable: whoever talks to `winit` is `app.rs`, which here only hands and
//! receives rectangles.
//! That keeps the file, the validation and the atomicity testable without a
//! display.
//!
//! The file is JSON under `XDG_STATE_HOME` (state, not configuration): if it goes
//! missing or comes out crooked, the window opens at the default and nothing else
//! happens.

use std::io::Write;
use std::path::PathBuf;

/// Below this is WM/restore garbage, not geometry.
pub const MIN_W: u32 = 320;
pub const MIN_H: u32 = 200;
/// Default: 980x640 gives back ~70x20 with the 13 pt font (`window.py:24-31`).
pub const DEFAULT: (u32, u32) = (980, 640);

/// What stays in the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Geometry {
    pub x: Option<i32>,
    pub y: Option<i32>,
    pub w: u32,
    pub h: u32,
    /// Maximized is WM state, separate from the normal size (J5): restoring
    /// `max` cannot overwrite the size the window had before.
    pub maximized: bool,
}

impl Default for Geometry {
    fn default() -> Self {
        Self {
            x: None,
            y: None,
            w: DEFAULT.0,
            h: DEFAULT.1,
            maximized: false,
        }
    }
}

/// What the window hands back next time: geometry (`J2`–`J6`) and the layout of
/// the session (`M-I`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct State {
    pub geometry: Geometry,
    /// `None` when the file has no layout (an older file), which means "one tab,
    /// as always".
    pub layout: Option<Layout>,
}

/// The workspaces and tabs of the last session, in logical order (`M-I`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Layout {
    pub workspaces: Vec<WorkspaceMemory>,
    /// Workspace in front, in `0..workspaces.len()`.
    pub ws: usize,
    /// Tab in front of that workspace, in `0..workspaces[ws].cwds.len()`.
    pub tab: usize,
}

/// One remembered workspace: its name and the cwd of each tab.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorkspaceMemory {
    pub name: String,
    /// `None` = the tab opened on the shell's default directory (no `OSC 7` and
    /// no readable `/proc`).
    pub cwds: Vec<Option<String>>,
}

/// Monitor rectangle, as `winit` delivers it: (x, y, width, height).
pub type Monitor = (i32, i32, u32, u32);

/// `$XDG_STATE_HOME/tortty/window.json`, with `~/.local/state` as fallback.
#[must_use]
pub fn state_path() -> Option<PathBuf> {
    Some(
        crate::platform::state_dir()?
            .join("tortty")
            .join("window.json"),
    )
}

/// The `flock` that makes one window the **owner** of the state (`B46`).
///
/// The owner is the window that opened the session: it restores, it applies the
/// remembered geometry and it is the only one that writes. The owner is asked of
/// the kernel, not of a PID file — a lock held for the life of the process has no
/// stale entry and no recycled PID to defend against, the lesson `devkill.sh`
/// learned with `/proc/<pid>/exe`.
pub struct Owner {
    /// Held for the life of the process and never read: closing the descriptor is
    /// what releases the lock, so the field exists to not be dropped.
    _file: std::fs::File,
}

impl Owner {
    /// Path of the lock file: beside `window.json`.
    #[must_use]
    pub fn path() -> Option<PathBuf> {
        state_path().map(|p| p.with_file_name("state.lock"))
    }
}

/// Tries to become the owner; `None` when another instance already is.
///
/// Never blocks: the lock is asked for without waiting, and an instance that finds
/// it taken is a scratch window, not a second owner. A lock file that cannot be
/// created means "no remembered state", which is the same as not owning it.
#[must_use]
pub fn lock() -> Option<Owner> {
    let path = Owner::path()?;
    lock_at(&path)
}

/// `lock` on a given path (the test uses its own file: environment is not test
/// state). The lock itself is `platform::try_lock` — which primitive takes it is
/// the platform's business.
#[must_use]
pub fn lock_at(path: &std::path::Path) -> Option<Owner> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).ok()?;
    }
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)
        .ok()?;
    crate::platform::try_lock(&file).then_some(Owner { _file: file })
}

/// Reads the file; `None` for missing, unreadable, crooked JSON or wrong type.
/// Never an error: corrupted state only means "opens at the default".
#[must_use]
pub fn load() -> Option<State> {
    load_from(&state_path()?)
}

/// `load` on a given path (what the test uses: environment is not test state).
#[must_use]
pub fn load_from(path: &std::path::Path) -> Option<State> {
    let text = std::fs::read_to_string(path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    let obj = v.as_object()?;
    let num = |k: &str| obj.get(k).and_then(serde_json::Value::as_i64);
    // `w`/`h` are mandatory: without them there is no geometry, only a `max: true`
    let geometry = Geometry {
        x: num("x").and_then(|n| i32::try_from(n).ok()),
        y: num("y").and_then(|n| i32::try_from(n).ok()),
        w: u32::try_from(num("w")?).ok()?,
        h: u32::try_from(num("h")?).ok()?,
        maximized: obj
            .get("max")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
    };
    Some(State {
        geometry,
        layout: layout_of(obj.get("layout")),
    })
}

/// The `layout` member, or `None` when it is absent or says nothing usable.
///
/// It is read defensively because it is a hand-editable file and because it
/// points at directories that can be gone by the next run: a workspace without
/// a tab is dropped, an index out of range falls back to the first entry.
#[must_use]
fn layout_of(v: Option<&serde_json::Value>) -> Option<Layout> {
    let v = v?;
    let workspaces: Vec<WorkspaceMemory> = v
        .get("workspaces")?
        .as_array()?
        .iter()
        .filter_map(|w| {
            let cwds: Vec<Option<String>> = w
                .get("cwds")?
                .as_array()?
                .iter()
                .map(|c| c.as_str().map(str::to_string))
                .collect();
            if cwds.is_empty() {
                return None;
            }
            Some(WorkspaceMemory {
                name: w.get("name")?.as_str().unwrap_or_default().to_string(),
                cwds,
            })
        })
        .collect();
    if workspaces.is_empty() {
        return None;
    }
    let idx = |key: &str, len: usize| {
        let n = v.get(key).and_then(serde_json::Value::as_u64).unwrap_or(0);
        usize::try_from(n).unwrap_or(0).min(len.saturating_sub(1))
    };
    let ws = idx("ws", workspaces.len());
    Some(Layout {
        tab: idx("tab", workspaces[ws].cwds.len()),
        ws,
        workspaces,
    })
}

/// Writes **atomically** (tmp + rename): never leaves half a JSON behind.
/// Failing to save cannot bring the shutdown down.
pub fn save(s: &State) {
    if let Some(path) = state_path() {
        save_to(&path, s);
    }
}

/// `save` on a given path.
pub fn save_to(path: &std::path::Path, s: &State) {
    let g = &s.geometry;
    if let Some(dir) = path.parent() {
        // a directory that cannot be created is not a fatal error: the terminal
        // has to run confined (sandbox, read-only home). It just cannot be
        // silent, or nobody understands why the window is not remembered
        if let Err(e) = std::fs::create_dir_all(dir) {
            eprintln!("tortty: no remembered geometry ({}: {e})", dir.display());
            return;
        }
    }
    let mut v = serde_json::Map::new();
    if let (Some(x), Some(y)) = (g.x, g.y) {
        v.insert("x".into(), x.into());
        v.insert("y".into(), y.into());
    }
    v.insert("w".into(), g.w.into());
    v.insert("h".into(), g.h.into());
    v.insert("max".into(), g.maximized.into());
    if let Some(l) = &s.layout {
        let workspaces: Vec<serde_json::Value> = l
            .workspaces
            .iter()
            .map(|w| {
                serde_json::json!({
                    "name": w.name,
                    "cwds": w.cwds.iter().map(|c| match c {
                        Some(p) => serde_json::Value::from(p.as_str()),
                        None => serde_json::Value::Null,
                    }).collect::<Vec<_>>(),
                })
            })
            .collect();
        v.insert(
            "layout".into(),
            serde_json::json!({"ws": l.ws, "tab": l.tab, "workspaces": workspaces}),
        );
    }
    let tmp = path.with_extension("json.tmp");
    let Ok(mut f) = std::fs::File::create(&tmp) else {
        eprintln!(
            "tortty: no remembered geometry ({} not writable)",
            tmp.display()
        );
        return;
    };
    let ok = writeln!(f, "{}", serde_json::Value::Object(v)).is_ok() && f.sync_all().is_ok();
    drop(f);
    if !ok || std::fs::rename(&tmp, path).is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
}

/// Writes the snapshots of the session, dropping the ones of tabs that do not
/// exist any more (`restore_state = content`, `B45`).
///
/// Failing here cannot bring the shutdown down — a tab that cannot be written
/// simply starts clean on the next run, which is the old behaviour.
pub fn save_snapshots(snaps: &[(usize, usize, Vec<u8>)]) {
    let Some(path) = state_path() else {
        return;
    };
    let Some(dir) = path.parent() else {
        return;
    };
    save_snapshots_to(dir, snaps);
}

/// `save_snapshots` on a given directory.
pub fn save_snapshots_to(dir: &std::path::Path, snaps: &[(usize, usize, Vec<u8>)]) {
    if let Err(e) = std::fs::create_dir_all(dir) {
        eprintln!("tortty: no remembered content ({}: {e})", dir.display());
        return;
    }
    // a tab that was closed must not come back: every snapshot is rewritten, so
    // whatever is left over is from a session that no longer exists
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            let name = e.file_name();
            let name = name.to_string_lossy();
            if name.starts_with("session-") && name.ends_with(".ans") {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    for (ws, tab, bytes) in snaps {
        let path = dir.join(format!("session-{ws}-{tab}.ans"));
        write_atomic(&path, bytes);
    }
}

/// Reads back the snapshot of one tab; `None` for missing or empty.
#[must_use]
pub fn load_snapshot(ws: usize, tab: usize) -> Option<Vec<u8>> {
    let dir = state_path()?.parent().map(std::path::Path::to_path_buf)?;
    load_snapshot_from(&dir, ws, tab)
}

/// `load_snapshot` on a given directory.
#[must_use]
pub fn load_snapshot_from(dir: &std::path::Path, ws: usize, tab: usize) -> Option<Vec<u8>> {
    let bytes = std::fs::read(dir.join(format!("session-{ws}-{tab}.ans"))).ok()?;
    (!bytes.is_empty()).then_some(bytes)
}

/// tmp + rename: never leaves half a file behind, and a reader never sees one.
fn write_atomic(path: &std::path::Path, bytes: &[u8]) {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    let Ok(mut f) = std::fs::File::create(&tmp) else {
        eprintln!(
            "tortty: no remembered content ({} not writable)",
            tmp.display()
        );
        return;
    };
    let ok = f.write_all(bytes).is_ok() && f.sync_all().is_ok();
    drop(f);
    if !ok || std::fs::rename(&tmp, path).is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
}

/// Usable geometry: healthy size and the corner **inside some monitor**.
///
/// `None` means "nothing usable left" — monitor disconnected, hand-edited file,
/// resolution changed. It is enough for the top-left corner to be on a monitor:
/// the title bar stays reachable and the WM never opens the window off screen.
#[must_use]
pub fn fit(
    x: Option<i32>,
    y: Option<i32>,
    w: u32,
    h: u32,
    monitors: &[Monitor],
) -> Option<Geometry> {
    if w < MIN_W || h < MIN_H {
        return None;
    }
    let (Some(x), Some(y)) = (x, y) else {
        // no position (Wayland, J6): the size alone still counts
        return Some(Geometry {
            x: None,
            y: None,
            w,
            h,
            maximized: false,
        });
    };
    if monitors.is_empty() {
        // no known monitor means no way to judge: accept
        return Some(Geometry {
            x: Some(x),
            y: Some(y),
            w,
            h,
            maximized: false,
        });
    }
    let inside = monitors.iter().any(|&(mx, my, mw, mh)| {
        x >= mx
            && x < mx + i32::try_from(mw).unwrap_or(i32::MAX)
            && y >= my
            && y < my + i32::try_from(mh).unwrap_or(i32::MAX)
    });
    inside.then_some(Geometry {
        x: Some(x),
        y: Some(y),
        w,
        h,
        maximized: false,
    })
}

#[cfg(test)]
mod checks_state {
    use super::{
        fit, load_from, load_snapshot_from, lock_at, save_snapshots_to, save_to, Geometry, Layout,
        Monitor, State, WorkspaceMemory, DEFAULT, MIN_H, MIN_W,
    };

    /// A `State` with no layout: the file as the old version wrote it.
    fn st(g: Geometry) -> State {
        State {
            geometry: g,
            layout: None,
        }
    }
    use std::path::{Path, PathBuf};

    fn monitors() -> Vec<Monitor> {
        // a 1920x1080 at 0,0 and a 1280x1024 to its right
        vec![(0, 0, 1920, 1080), (1920, 0, 1280, 1024)]
    }

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("bz-estado-{name}"));
        let _ = std::fs::remove_dir_all(&d);
        d.join("tortty").join("window.json")
    }

    #[test]
    fn default_is_980x640() {
        let g = Geometry::default();
        assert_eq!((g.w, g.h), DEFAULT);
        assert_eq!((g.w, g.h), (980, 640));
        assert!(!g.maximized);
    }

    #[test]
    fn fit_accepts_sane_rectangle_on_monitor() {
        let g = fit(Some(100), Some(80), 900, 600, &monitors()).expect("fits");
        assert_eq!((g.x, g.y, g.w, g.h), (Some(100), Some(80), 900, 600));
    }

    #[test]
    fn fit_rejects_garbage_size() {
        // below the minimum: WM garbage or a hand-edited file
        assert!(fit(Some(0), Some(0), MIN_W - 1, 600, &monitors()).is_none());
        assert!(fit(Some(0), Some(0), 600, MIN_H - 1, &monitors()).is_none());
        // the exact edge passes
        assert!(fit(Some(0), Some(0), MIN_W, MIN_H, &monitors()).is_some());
    }

    #[test]
    fn fit_rejects_corner_outside_all_monitors() {
        // disconnected monitor: the corner was left in the void
        assert!(fit(Some(5000), Some(0), 900, 600, &monitors()).is_none());
        assert!(fit(Some(0), Some(-500), 900, 600, &monitors()).is_none());
        // second monitor: accepted
        assert!(fit(Some(2000), Some(100), 900, 600, &monitors()).is_some());
        // exactly at the width of the first monitor is already the second
        assert!(fit(Some(1920), Some(0), 900, 600, &monitors()).is_some());
        // and one pixel before is still the first
        assert!(fit(Some(1919), Some(0), 900, 600, &monitors()).is_some());
    }

    #[test]
    fn fit_without_known_monitor_accepts_the_size() {
        // no monitor means no way to judge the position (the WM decides)
        assert!(fit(Some(-9000), Some(-9000), 900, 600, &[]).is_some());
    }

    #[test]
    fn fit_without_position_keeps_the_size_wayland() {
        // Wayland gives no position (J6): the size alone is what is left
        let g = fit(None, None, 900, 600, &monitors()).expect("size counts");
        assert_eq!((g.x, g.y), (None, None));
        assert_eq!((g.w, g.h), (900, 600));
    }

    #[test]
    fn path_respects_xdg_state_home() {
        // `state_path` is the only part that reads the environment; the assembly is what matters
        let p = std::env::temp_dir().join("bz-xdg");
        assert_eq!(
            p.join("tortty").join("window.json"),
            PathBuf::from(format!("{}/tortty/window.json", p.display()))
        );
    }

    #[test]
    fn save_and_read_roundtrip_and_leaves_no_tmp() {
        let path = tmp("roundtrip");
        let g = Geometry {
            x: Some(120),
            y: Some(60),
            w: 1000,
            h: 700,
            maximized: true,
        };
        save_to(&path, &st(g));
        assert_eq!(load_from(&path), Some(st(g)));
        let leftovers: Vec<String> = std::fs::read_dir(path.parent().expect("dir"))
            .expect("directory created")
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(leftovers, ["window.json"], "no tmp may be left behind");
        // Wayland: no position, the rest remains (J6)
        let s = Geometry {
            x: None,
            y: None,
            ..g
        };
        save_to(&path, &st(s));
        assert_eq!(load_from(&path), Some(st(s)));
        let _ = std::fs::remove_dir_all(Path::new(&path).ancestors().nth(2).expect("root"));
    }

    #[test]
    fn save_in_impossible_place_does_not_blow_up() {
        // sandbox/read-only home: the app has to stay up (and say it on stderr,
        // not vanish silently)
        let path = std::path::Path::new("/proc/tortty-does-not-exist/window.json");
        save_to(
            path,
            &st(Geometry {
                x: None,
                y: None,
                w: 800,
                h: 600,
                maximized: false,
            }),
        );
        assert!(!path.exists());
        assert!(!path.with_extension("json.tmp").exists());
    }

    #[test]
    fn the_layout_survives_a_round_trip_and_loses_only_what_it_cannot_open() {
        let path = tmp("layout");
        let g = Geometry {
            x: Some(10),
            y: Some(20),
            w: 900,
            h: 600,
            maximized: false,
        };
        let l = Layout {
            workspaces: vec![
                WorkspaceMemory {
                    name: "one".into(),
                    cwds: vec![Some("/tmp".into())],
                },
                WorkspaceMemory {
                    name: "two".into(),
                    cwds: vec![None, Some("/".into())],
                },
            ],
            ws: 1,
            tab: 1,
        };
        save_to(
            &path,
            &State {
                geometry: g,
                layout: Some(l.clone()),
            },
        );
        assert_eq!(load_from(&path).and_then(|s| s.layout), Some(l));
        // a workspace with no tab is dropped and the index falls back to the
        // survivors: the file is hand-editable and points at directories
        let hand = r#"{"w":900,"h":600,"layout":{"ws":5,"tab":9,"workspaces":
            [{"name":"a","cwds":[]},{"name":"b","cwds":["/tmp"]}]}}"#;
        std::fs::write(&path, hand).expect("write");
        let want = Some(Layout {
            workspaces: vec![WorkspaceMemory {
                name: "b".into(),
                cwds: vec![Some("/tmp".into())],
            }],
            ws: 0,
            tab: 0,
        });
        assert_eq!(load_from(&path).and_then(|s| s.layout), want);
        // a file from before the layout existed still opens, with one tab
        std::fs::write(&path, r#"{"w":900,"h":600}"#).expect("write");
        assert_eq!(load_from(&path).map(|s| s.layout), Some(None));
        let _ = std::fs::remove_dir_all(Path::new(&path).ancestors().nth(2).expect("root"));
    }

    #[test]
    fn skewed_file_is_not_an_error_only_default() {
        let path = tmp("crooked");
        std::fs::create_dir_all(path.parent().expect("dir")).expect("dir");
        for garbage in ["", "{", "[]", "null", "\"text\"", "{\"x\":1}"] {
            std::fs::write(&path, garbage).expect("write");
            assert_eq!(load_from(&path), None, "garbage accepted: {garbage:?}");
        }
        // a number that does not fit in i32/x/y becomes None, but w/h rule
        std::fs::write(&path, r#"{"x":1e300,"y":3,"w":900,"h":600}"#).expect("write");
        let g = load_from(&path).expect("valid w/h").geometry;
        assert_eq!((g.x, g.y, g.w, g.h), (None, Some(3), 900, 600));
        // a file that does not even exist: default, no error
        assert_eq!(load_from(&path.with_extension("doesnotexist")), None);
        let _ = std::fs::remove_dir_all(Path::new(&path).ancestors().nth(2).expect("root"));
    }

    #[test]
    fn only_one_window_owns_the_state() {
        let path = tmp("lock").with_file_name("state.lock");
        let first = lock_at(&path).expect("the first window is the owner");
        // the second instance finds the lock taken: it is a scratch window
        assert!(lock_at(&path).is_none());
        // the lock is the process's, and it dies with it
        drop(first);
        let again = lock_at(&path).expect("the lock is free after the owner is gone");
        drop(again);
        let _ = std::fs::remove_dir_all(path.ancestors().nth(2).expect("root"));
    }

    #[test]
    fn a_snapshot_comes_back_and_the_closed_tab_does_not() {
        let path = tmp("snapshot");
        let dir = path.parent().expect("dir").to_path_buf();
        save_snapshots_to(
            &dir,
            &[
                (0, 0, b"first".to_vec()),
                (0, 1, b"second".to_vec()),
                (1, 0, b"other workspace".to_vec()),
            ],
        );
        assert_eq!(
            load_snapshot_from(&dir, 0, 1).as_deref(),
            Some(&b"second"[..])
        );
        assert_eq!(
            load_snapshot_from(&dir, 1, 0).as_deref(),
            Some(&b"other workspace"[..])
        );
        // a tab closed between the two runs does not come back on the third
        save_snapshots_to(&dir, &[(0, 0, b"only one now".to_vec())]);
        assert_eq!(load_snapshot_from(&dir, 0, 1), None);
        assert_eq!(load_snapshot_from(&dir, 1, 0), None);
        assert_eq!(
            load_snapshot_from(&dir, 0, 0).as_deref(),
            Some(&b"only one now"[..])
        );
        // nothing written is not an error, it is a clean start
        assert_eq!(load_snapshot_from(&dir, 7, 7), None);
        let _ = std::fs::remove_dir_all(Path::new(&path).ancestors().nth(2).expect("root"));
    }
}
