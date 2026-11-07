# Externals — bugs of other software

A trap or a protocol violation that is **not ours**, kept so the investigation is
not paid twice. An entry enters **when the owner asks**: a measured case with the
proof attached, never a suspicion. It is not a queue ([`BACKLOG.md`](BACKLOG.md)):
nothing here is work for us. When the other side fixes it, the row leaves and the
removal is a line in [`HISTORY.md`](HISTORY.md).

| when | app | symptom | proof | Workaround | retest on |
|---|---|---|---|---|---|
| 2026-11-07 | `pi` 1.4.0 | leaves and the tab stays on its title, no longer following the `cd` | one `process.stdout.write("\x1B]0;${title}\x07")` in the whole `dist` (`dist/bundle/chunks/chunk-T726RZZU.js:107`), **zero** `CSI 22;2t`/`23;2t`, and `restoreOnExit` restores paste/kitty-keyboard/cursor but not the title; in tortty, without pi: socket `write` of `printf '\033]0;x\007'`, then `cd /tmp` → frozen, then `printf '\033c'` → the cwd is back | `reset`, or `OSC 0` in the `precmd` (`B39`) | pi > 1.4.0: open, quit, `cd /tmp` |
