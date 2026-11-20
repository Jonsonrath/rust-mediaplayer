# rmp — Rust Terminal Media Player: Specification

Binary/crate name: `rmp`. XDG app dir name: `rmp`.
Linux only (Fedora, NixOS). KISS, YAGNI, clean modular Rust. Real working features, proper error handling, tests.

## Stack
- Rust stable, Ratatui + Crossterm.
- Playback: external `mpv` process controlled via JSON IPC (unix socket). No manual decoding.
- Concurrency: std threads + `std::sync::mpsc` channels (Tokio only if genuinely required).
- SQLite via rusqlite (bundled). Metadata via lofty; ffprobe/mpv for video info when lofty can't.
- Config: TOML + serde. MPRIS via zbus (blocking API is fine).
- `std::process::Command` with separate args (never shell strings). IPC socket in `$XDG_RUNTIME_DIR/rmp/` (fallback: a 0700 dir under /tmp/rmp-$UID), cleaned up on exit.

## Playback engine
- Audio: MP3, FLAC, WAV, OGG, Opus, AAC, M4A, etc. Video: MP4, MKV, WebM, AVI, MOV, etc.
- Audio plays with no video window (`--no-video`/`vid=no` for audio). Playback continues across view switches.
- Pause/resume/stop/seek/next/prev/volume. Accurate position + duration.
- Video opens in a separate native mpv window; TUI stays the controller. Closing the video window handled safely (treat as stop / move on, no crash, no zombie). Playing audio after video must not leave a video window. Never spawn duplicate mpv processes: ONE mpv process + ONE IPC connection, one playback controller.
- Handle mpv not installed, mpv crash, IPC disconnect (respawn on next play), missing files, unsupported media.
- mpv must not expose its own MPRIS (pass `--load-scripts=no` or disable the mpris script) to avoid competing instances.

## Repeat / shuffle
- `r` cycles exactly: Off → All → One → Off.
- Off: play in queue order, stop at end. All: wrap around forever. One: loop current file forever (use mpv `loop-file=inf`) until skip/stop/mode change; works for audio and video.
- `s` toggles shuffle, independent of repeat. No immediate replay of the same track when others exist. Sensible prev/next (prev goes back through actual play order). Turning shuffle off restores queue order (continue from current entry's queue position). Repeat All + Shuffle supported (reshuffle each cycle). Repeat One repeats current regardless of shuffle. Explicit `n`/`p` skips even in Repeat One.
- Queue changes during playback: define + test (e.g. removing the current entry keeps playing it until it ends, then continues from where it was; added entries get included in the shuffle order).
- Status bar always shows repeat and shuffle state.

## Views (keys 1–5)
1. Library: artists | albums | tracks panes (tracks widest); filter, sort, add to queue. Collapses to 2/1 panes when narrow.
2. Directory browser: real filesystem, enter dirs, parent dir, play directly without indexing, add file or whole dir (recursive) to queue, unsupported files clearly indicated (muted).
3. Playlists: two panes (playlists | contents). Create/rename/delete (modal dialogs), add/remove/reorder items, load or append into queue.
4. Queue: reorder (J/K), remove (d), clear, play selected immediately. Repeated occurrences of the same file are distinct entries (own entry id).
5. History: played media with human-readable timestamps + metadata; replay entry, clear history (with confirmation).

Persistent bottom player panel on all views: title, artist, state, elapsed/total, progress bar, volume, repeat, shuffle.

## Keys (defaults; configurable in TOML)
1–5 views · j/k down/up · h/l panels or dirs · Enter open/play · Space pause · n next · p prev · r repeat · s shuffle · a add to queue · d remove queue item · J/K move queue item · / search · Esc cancel · Left/Right seek ∓5s · -/+ volume · Tab focus next panel · ? help · q quit.
Shortcuts must not trigger while typing in text inputs.

## Library
SQLite library: configured dirs, recursive indexing in a background thread (UI never freezes), manual rescan, incremental (mtime+size), detect removed files, audio+video together, metadata, unique path, stable integer ids, sort/filter, corrupt files handled. No decoding whole files. No FS scanning during redraws. Simple schema migrations (`PRAGMA user_version`).

## Metadata
title, artist, album, album artist, track no, genre, year, duration, format, size, video resolution. Filename fallback for title. All optional — never break playback.

## Search
By title/artist/album/filename/path, live as you type, SQLite LIKE with indexes is fine (or in-memory filter of already-loaded tracks if faster/simpler).

## Playlists & queue persistence
Playlists in SQLite, ordered, add/remove/reorder, load/append to queue, survive restart. Persist the active queue (simple table) if straightforward. Deleted files don't crash.

## History
media id/path, start timestamp, played duration, completed flag, title/artist snapshot. Write on track end/change only (not every second). Very short plays (< ~5s or seek noise) not counted.

## MPRIS
org.mpris.MediaPlayer2 + .Player: PlayPause/Play/Pause/Next/Previous/Stop/Seek/SetPosition/Volume/PlaybackStatus/Metadata/Position/Shuffle/LoopStatus (Off=None, All=Playlist, One=Track). MPRIS actions → internal commands → state/TUI. TUI changes → MPRIS PropertiesChanged. D-Bus unavailable → log/notify and continue.

## Architecture principles
Single authoritative playback state; playback logic independent of rendering; UI renders state and emits Commands; explicit commands/events; nothing blocks terminal input; no global mutable state; no giant files; no needless generics; no unsafe; no `unwrap()`/`expect()` on routine failure paths; always restore the terminal (incl. on panic via hook).

## Config
`~/.config/rmp/config.toml`, DB `~/.local/share/rmp/library.db` (respect XDG_* env vars). Sample config: media dirs, default volume, theme colors, start view, keybindings, rescan-on-start. First launch works with no config. No hardcoded usernames/paths.

## Errors to handle
mpv missing/exits, IPC fail, invalid media, missing dir, permission denied, deleted queued file, corrupt metadata, empty playlist/queue, invalid config (report, use defaults), SQLite errors, tiny terminal, resize during playback, video window closed.

## Testing
Repeat cycling, Repeat One/All/Off behavior, shuffle, queue insert/remove/reorder, prev/next, state transitions, playlist persistence, indexing + duplicate prevention, migrations, metadata fallbacks, search, IPC message serialization/parsing. mpv integration tests skip cleanly when mpv is absent.
Must pass: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, `cargo build --release`.

## Performance
Instant navigation, no busy-wait, no per-frame DB/FS/metadata access, no constant disk writes, thousands of tracks fine, redraw only when state changed (or on a ~250–500ms tick while playing for the progress bar).

---

# UI design (premium, minimalist cyber-industrial — modern cmus / lazygit; restrained GitS feel)

## Palette (semantic theme tokens, configurable, no hardcoded colors in widgets)
bg #0B0F14 · surface #121922 · raised #19232D · text #E6EDF3 · text_secondary #81909F · text_muted #52606E · accent #51D6D0 (cyan) · accent_alt #6096E8 (blue) · playing #79D8A0 · warning #E4B46A · error #E4777C · border #293540

## Composition
- Slim header: app identity + tabs, e.g. `RMP / 01   01 LIBRARY  02 FILES  03 PLAYLISTS  04 QUEUE  05 HISTORY` then a thin `─` separator. Active tab in accent + bold.
- Main content area per view. Avoid boxing everything; prefer separators, padding, grouping; occasional stronger panel borders.
- Persistent bottom playback panel (the centrepiece):
  ```
  NOW PLAYING
  Deftones — Digital Bath                      ▶ PLAYING
  01:24  ━━━━━━━━━━━━━━━━●━━━━━━━━━━━━━━━━━━  04:15
  VOL  75%       ↻ REPEAT ONE      SHUFFLE OFF
  ```
  Title prominent, artist/album secondary, bounded proportional progress bar, fixed-width times (no layout jumping), clear ▶/⏸/■ state. Repeat Off subdued, All accented, One accented with visible ONE. Unknown duration → no fake progress.
- Compact context-sensitive shortcut footer (one line).
- Inline transient status notifications (e.g. "Added 12 tracks to queue"), errors in error color, scan progress. Not modal for ordinary success.

## States (must all look different)
Selected: subtle raised-surface highlight + cursor marker `›`. Focused panel: accent-colored header/edge. Playing: persistent `▶` symbol + playing color, independent of selection. Queued: compact marker/position. Unavailable: muted + explanation.

## Typography
Unicode box drawing, thin separators, ━ ● progress, few caps, little bold, no emoji spam. Terminal-cell-aware widths (unicode-width); never byte-slice UTF-8. Must not depend on Nerd Fonts.

## Responsive breakpoints
120×35 full; 100×30; 80×24 compact (library → 2 panes, drop optional columns); 60×20 constrained (1 pane); smaller → minimal "terminal too small" fallback. Never panic, never overdraw neighbours.

## Dialogs
Centered modal dialogs with title, padding, confirm/cancel hints, keyboard-only: playlist create/rename, playlist item ops (add-to-playlist picker), destructive confirmations, error details, help. Search is an inline bar (not a full-screen popup), live filtering.

## Rendering
No DB fetch in render fns, no big per-frame allocations, keep scroll/selection state, event-driven redraws.
