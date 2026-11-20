//! Renders every view at several sizes against seeded data, checks invariants
//! and dumps plain-text snapshots to `target/ui-snapshots/` for design review.
use std::path::PathBuf;
use std::sync::mpsc;

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use unicode_width::UnicodeWidthStr;

use super::draw;
use crate::app::App;
use crate::config::keys::KeyMap;
use crate::event::{Command, MpvEvent};
use crate::library::Track;
use crate::library::database::Database;
use crate::playback::Player;
use crate::playback::modes::RepeatMode;
use crate::ui::theme::Theme;

const SIZES: [(u16, u16); 7] = [
    (120, 35),
    (100, 30),
    (80, 24),
    (60, 20),
    (45, 14),
    (20, 6),
    (0, 0),
];
const VIEWS: [&str; 5] = ["library", "files", "playlists", "queue", "history"];

fn trk(i: usize) -> Track {
    let artists = [
        "Deftones",
        "Boards of Canada",
        "宇多田ヒカル",
        "Björk",
        "Daft Punk",
        "Мумий Тролль",
    ];
    let long =
        "An Exceedingly Long Track Title That Keeps Going And Going Past Any Sane Column Width";
    let title = match i {
        0 => long.to_string(),
        1 => "夜に駆ける 🎧 night ride".into(),
        2 => "Untitled".into(),
        _ => format!("Track number {i:02}"),
    };
    let missing = i == 2;
    Track {
        id: 0,
        path: PathBuf::from(format!("/music/a{}/t{i}.flac", i % 6)),
        title,
        artist: (!missing).then(|| artists[i % 6].to_string()),
        album: (!missing).then(|| {
            format!(
                "Album {} {}",
                i % 6,
                if i.is_multiple_of(4) {
                    "(Deluxe Anniversary Remastered Edition)"
                } else {
                    ""
                }
            )
        }),
        album_artist: None,
        genre: None,
        track_no: (!missing).then_some(i as u32 % 12 + 1),
        year: None,
        duration: (i != 5).then_some(120.0 + i as f64 * 7.0),
        format: Some("FLAC".into()),
        size: 5_000_000,
        width: (i == 7).then_some(1920),
        height: (i == 7).then_some(1080),
        is_video: i == 7,
    }
}

fn seeded() -> App {
    let (tx, _rx) = mpsc::channel();
    let db = Database::open_in_memory().unwrap();
    let mut w = Vec::new();
    let keys = KeyMap::from_config(&Default::default(), &mut w);
    let mut a = App::new(tx.clone(), Player::new(tx, 75), db, keys, Theme::default());
    for i in 0..60 {
        a.db.upsert_track(&trk(i), 1).unwrap();
    }
    a.reload_library();
    let mut all = a.db.all_tracks().unwrap();
    for n in ["Road Trip", "Focus 集中", "Empty"] {
        a.db.create_playlist(n).unwrap();
    }
    let pls = a.db.playlists().unwrap();
    a.db.add_to_playlist(pls[0].id, &all[..8]).unwrap();
    a.db.add_to_playlist(pls[1].id, &all[10..13]).unwrap();
    let now = 1_700_000_000;
    for (k, t) in all.iter().take(9).enumerate() {
        let ago = [
            0, 3_000, 90_000, 95_000, 100_000, 180_000, 185_000, 190_000, 200_000,
        ][k];
        a.db.add_history(t, now - ago, 60.0 + k as f64 * 20.0, k % 3 != 0)
            .unwrap();
    }
    a.reload_playlists();
    a.reload_history();
    // Queue of 10, one pointing at a file that does not exist.
    let real = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    for (k, t) in all.iter_mut().take(10).enumerate() {
        if k != 4 {
            t.path = real.clone();
        }
    }
    a.command(Command::PlayTracks {
        tracks: all[..10].to_vec(),
        start: 0,
    });
    a.player.handle_mpv_event(&MpvEvent::PauseChanged(false));
    a.player
        .handle_mpv_event(&MpvEvent::DurationChanged(Some(255.0)));
    a.player.handle_mpv_event(&MpvEvent::PositionChanged(84.0));
    a.command(Command::SetRepeat(RepeatMode::One));
    a.command(Command::SetShuffle(true));
    a.notify("Added 12 tracks to queue");
    a
}

fn dump(term: &Terminal<TestBackend>) -> Vec<String> {
    let b = term.backend().buffer();
    let w = b.area.width as usize;
    let cells: Vec<&str> = b.content().iter().map(|c| c.symbol()).collect();
    if w == 0 {
        return vec![];
    }
    cells
        .chunks(w)
        .map(|r| {
            let mut out = String::new();
            let mut skip = 0;
            for c in r {
                if skip > 0 {
                    skip -= 1; // trailing cells of a wide glyph
                    continue;
                }
                skip = c.width().saturating_sub(1);
                out.push_str(c);
            }
            out.trim_end().to_string()
        })
        .collect()
}

#[test]
fn snapshots_all_views() {
    let mut a = seeded();
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/ui-snapshots");
    std::fs::create_dir_all(&dir).unwrap();
    let tabs = ["LIBRARY", "FILES", "PLAYLISTS", "QUEUE", "HISTORY"];
    for (v, name) in VIEWS.iter().enumerate() {
        a.view = v;
        for (w, h) in SIZES {
            let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
            term.draw(|f| draw(f, &a)).unwrap();
            let lines = dump(&term);
            std::fs::write(dir.join(format!("{name}-{w}x{h}.txt")), lines.join("\n")).unwrap();
            let text = lines.join("\n");
            for l in &lines {
                assert!(l.width() <= w as usize, "{name} {w}x{h}: wide line {l:?}");
            }
            if w >= 60 && h >= 20 {
                for s in ["REPEAT ONE", "SHUFFLE", "Added 12 tracks"] {
                    assert!(text.contains(s), "{name} {w}x{h}: missing {s}");
                }
                assert!(lines[0].contains("RMP"), "{name} {w}x{h}: header");
                assert!(
                    lines[0].contains(&tabs[v][..3]),
                    "{name} {w}x{h}: active tab"
                );
                assert!(text.contains("01:24"), "{name} {w}x{h}: position");
            }
        }
    }
}
