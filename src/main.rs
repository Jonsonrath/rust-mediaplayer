// Narrow to nothing once the views phase uses Theme/key_label/search/widgets helpers.

mod app;
mod config;
mod event;
mod library;
mod mpris;
mod playback;
mod ui;

use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use config::Config;
use config::keys::KeyMap;
use event::AppEvent;
use library::database::Database;
use playback::Player;

const TICK: Duration = Duration::from_millis(250);

fn main() -> anyhow::Result<()> {
    // ratatui::init enables raw mode + alt screen and installs a panic hook that restores them.
    let mut terminal = ratatui::init();
    let result = run(&mut terminal);
    ratatui::restore();
    result
}

fn run(terminal: &mut ratatui::DefaultTerminal) -> anyhow::Result<()> {
    let (tx, rx) = mpsc::channel::<AppEvent>();
    let (cfg, mut warnings) = Config::load();
    let theme = cfg.theme(&mut warnings);
    let keys = KeyMap::from_config(&cfg.keys, &mut warnings);

    let path = config::db_path();
    let (db, db_error) = match Database::open(&path) {
        Ok(db) => (db, None),
        Err(e) => (
            Database::open_in_memory()?,
            Some(format!("Library database unavailable: {e}")),
        ),
    };
    let player = Player::new(tx.clone(), cfg.volume);
    let mut app = app::App::new(tx.clone(), player, db, keys, theme);
    app.media_dirs = cfg.media_dirs.clone();
    app.view = cfg.start_view_index();
    app.db_path = db_error.is_none().then_some(path);
    app.init_browser();
    app.restore_queue();
    match mpris::spawn(tx.clone()) {
        Ok(m) => app.mpris = Some(m),
        Err(e) => warnings.push(e),
    }
    if let Some(e) = db_error {
        app.set_status(e, true);
    } else if let Some(w) = warnings.first() {
        let more = warnings.len() - 1;
        app.set_status(
            if more > 0 {
                format!("{w} (+{more} more)")
            } else {
                w.clone()
            },
            true,
        );
    }
    if cfg.rescan_on_start && !cfg.media_dirs.is_empty() {
        app.start_scan();
    }

    spawn_input(tx);
    let result = event_loop(terminal, &mut app, &rx);
    app.shutdown();
    result
}

fn event_loop(
    terminal: &mut ratatui::DefaultTerminal,
    app: &mut app::App,
    rx: &mpsc::Receiver<AppEvent>,
) -> anyhow::Result<()> {
    while !app.quit {
        if app.dirty {
            terminal.draw(|f| ui::draw(f, app))?;
            app.dirty = false;
        }
        let ev = rx.recv_timeout(TICK).unwrap_or(AppEvent::Tick);
        app.handle(ev);
    }
    Ok(())
}

/// Blocks on crossterm and forwards key/resize events; exits when the receiver is gone.
fn spawn_input(tx: mpsc::Sender<AppEvent>) {
    thread::spawn(move || {
        while let Ok(ev) = crossterm::event::read() {
            let ev = match ev {
                crossterm::event::Event::Key(k)
                    if k.kind != crossterm::event::KeyEventKind::Release =>
                {
                    AppEvent::Key(k)
                }
                crossterm::event::Event::Resize(..) => AppEvent::Resize,
                _ => continue,
            };
            if tx.send(ev).is_err() {
                break;
            }
        }
    });
}
