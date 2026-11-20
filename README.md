# rmp

A terminal media player for Linux. It has a library browser, a directory browser,
playlists, a play queue and listening history, with playback handled by `mpv`.

## Features

- Library indexed from configured folders into SQLite, with background, incremental rescans
- Views: Library (artists / albums / tracks), Files (directory browser), Playlists,
  Queue, History
- Audio plays in the terminal; video opens in a separate `mpv` window
- Repeat (Off / All / One) and shuffle, with queue editing during playback
- Persistent playlists, queue and history
- Live search in the Library view
- MPRIS support, so desktop media keys and tools can control playback
- Keyboard-only, with configurable keys and theme colors

## Requirements

- Linux with a terminal that supports the alternate screen (Fedora and NixOS are the tested targets)
- Rust 1.85 or newer (the crate uses edition 2024)
- `mpv` on `PATH` (required for playback)
- `ffprobe` (optional; it ships with `ffmpeg` and fills in missing metadata for video)
- A D-Bus session bus (optional; only needed for MPRIS)

### Install dependencies on Fedora

```sh
sudo dnf install mpv ffmpeg rust cargo
```

`ffprobe` comes with `ffmpeg`. For full codec support, enable RPM Fusion (free) and use
its `mpv` and `ffmpeg` builds.

### Install dependencies on NixOS

For a temporary shell:

```sh
nix-shell -p mpv ffmpeg cargo rustc
```

Or add them to your configuration:

```nix
environment.systemPackages = with pkgs; [ mpv ffmpeg cargo rustc ];
```

## Build and run

```sh
cargo build --release
./target/release/rmp
```

Quit with `q`. The database and config are created on first use, so no setup is needed
to start.

## Default keys

Keys are shown as the defaults. Several keys can map to one action (for example `j` and
the Down arrow).

| Key | Action |
| --- | --- |
| `1`–`5` | Switch to Library, Files, Playlists, Queue, History |
| `j` / `↓`, `k` / `↑` | Move down / up |
| `h`, `l` | Move between panes or into / out of directories |
| `g`, `G` | Top / bottom of list |
| `ctrl-d` / `PgDn`, `ctrl-u` / `PgUp` | Page down / up |
| `Enter` | Open, play or replay the selected item |
| `Tab` | Focus next pane |
| `Backspace` | Parent directory (Files view) |
| `Space` | Pause / resume |
| `x` | Stop |
| `n` / `p` | Next / previous track |
| `←` / `→` | Seek back / forward 5 seconds |
| `-` / `+` (or `=`) | Volume down / up 5 |
| `r` | Cycle repeat: Off → All → One → Off |
| `s` | Toggle shuffle |
| `a` | Add to queue |
| `A` | Add to playlist |
| `d` | Remove selected queue or playlist item |
| `J` / `K` | Move queue or playlist item down / up |
| `N` | New playlist |
| `R` | Rename playlist |
| `D` | Delete playlist, or clear history (with confirmation); `C` clears the queue (with confirmation) |
| `C` | Clear queue (stops playback) |
| `/` | Search (Library view) |
| `u` | Rescan the library |
| `?` | Help |
| `q` / `ctrl-c` | Quit |

## Configuration

The config file is `$XDG_CONFIG_HOME/rmp/config.toml`, or `~/.config/rmp/config.toml`
when `XDG_CONFIG_HOME` is not set. A missing file uses the defaults.

```sh
mkdir -p ~/.config/rmp
cp config.example.toml ~/.config/rmp/config.toml
```

`config.example.toml` lists every option with its default. A short example:

```toml
media_dirs = ["~/Music", "/srv/media/videos"]
volume = 70
start_view = "queue"        # library, files, playlists, queue, history
rescan_on_start = true

[theme]
accent = "#51D6D0"

[keys]
quit = "Q"
view_library = "y"
```

- Invalid values and unknown keys do not stop the app. The status line reports them and
  the default is used.
- A `[keys]` entry replaces all default keys for that action. Key forms: single characters
  (`j`, `J`, `+`), names (`enter`, `esc`, `tab`, `space`, `backspace`, arrows, `pgup`,
  `pgdown`, `home`, `end`), and `ctrl-` prefixed keys.
- Theme colors are `#RRGGBB` values for the tokens listed in `config.example.toml`.

## Library

- `media_dirs` lists the folders to index. A leading `~` is expanded. If it is unset,
  `~/Music` and `~/Videos` are used when they exist.
- With `rescan_on_start = true` (the default), the library is scanned at startup.
  Press `u` to rescan at any time.
- Scans run on a background thread, so the interface stays responsive. Progress and
  results appear in the status line.
- Scans are incremental: a file is re-read only when its modification time or size
  changes. Files that have disappeared are removed from the library. Overlapping folders
  are indexed once.
- Unreadable folders are reported and their existing entries are kept.
- Metadata comes from the file's tags. The title falls back to the file name. Anything
  missing is left empty and never stops playback.

## Playback

- Repeat cycles exactly Off → All → One → Off. Off plays the queue in order and stops at
  the end. All wraps around forever. One loops the current file until you skip, stop or
  change the mode.
- Shuffle is independent of repeat. Repeat One repeats the current file even when
  shuffle is on. `n` and `p` skip even in Repeat One.
- Removing the playing item from the queue lets it finish, then playback continues.
- Playback position and duration come from `mpv`. Plays shorter than 5 seconds that were
  not completed are not recorded in history.

## Supported formats

- Audio: mp3, flac, wav, ogg, oga, opus, aac, m4a, wma, aiff, aif, ape, wv, mka, mpc
- Video: mp4, mkv, webm, avi, mov, m4v, wmv, flv, mpg, mpeg, ts, ogv, 3gp

Only files with these extensions are indexed. The Files view can open other files, but
they are shown muted as unsupported. Whether a file actually plays depends on your `mpv`
and `ffmpeg` builds.

Limitations:

- Video plays in a separate `mpv` window. Closing that window stops playback.
- Only one `mpv` process runs at a time. It is restarted on the next play after a crash.
- History timestamps are shown in local time. The UTC offset is read once at startup
  with `date +%z` (falling back to UTC), so restart after a daylight-saving change.
- Linux only.

## MPRIS

rmp registers `org.mpris.MediaPlayer2.rmp` on the session bus. Play/pause, next,
previous, stop, seek, position, volume, shuffle and loop status can be controlled from
desktop media keys and tools such as `playerctl`. `mpv` is started with its own scripts
disabled, so only rmp appears on MPRIS.

## Files

- Config: `$XDG_CONFIG_HOME/rmp/config.toml` (default `~/.config/rmp/config.toml`)
- Library database (library, playlists, queue and history): `$XDG_DATA_HOME/rmp/library.db`
  (default `~/.local/share/rmp/library.db`)
- `mpv` IPC socket: `$XDG_RUNTIME_DIR/rmp/` (falls back to `/tmp/rmp-$UID/`), mode 0700

## Troubleshooting

- **"mpv is not installed"**: install `mpv` (see Requirements) and make sure it is on
  `PATH`. Check with `command -v mpv`.
- **No MPRIS, or a "D-Bus" warning**: MPRIS needs a session bus. Check that
  `DBUS_SESSION_BUS_ADDRESS` is set. Playback works without it.
- **"Terminal too small — resize"**: the window is below 40×12. Resize it. 80×24 or
  larger gives the full layout.
- **Library is empty**: check that the folder is in `media_dirs`, that the file
  extension is in the supported list, and press `u` to rescan.
- **Config warnings in the status line**: the named keys or values were ignored and
  defaults are used. Fix the line shown in the warning.
- **Resetting the library database**: quit rmp and delete `library.db`. This also deletes
  your playlists, queue and history, so back the file up first. On the next start the
  library is rebuilt by scanning.
