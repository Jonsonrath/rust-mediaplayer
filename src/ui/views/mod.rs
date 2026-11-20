//! One module per view; each exposes `draw(f, app, area)`.
pub mod browser;
pub mod history;
pub mod library;
pub mod playlists;
pub mod queue;

use ratatui::Frame;
use ratatui::layout::Rect;

use crate::app::App;
use crate::ui::layout::Breakpoint;

pub fn draw(f: &mut Frame, app: &App, area: Rect, bp: Breakpoint) {
    match app.view {
        0 => library::draw(f, app, area, bp),
        1 => browser::draw(f, app, area, bp),
        2 => playlists::draw(f, app, area, bp),
        3 => queue::draw(f, app, area, bp),
        _ => history::draw(f, app, area, bp),
    }
}
