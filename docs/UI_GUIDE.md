# UI guide for view agents

Each view agent owns exactly `src/ui/views/<view>.rs` + `src/app/views/<view>.rs`
(library, browser, playlists, queue, history). Do not edit app.rs, views/mod.rs, ui/mod.rs, theme/widgets/layout.

## Draw entry
`pub fn draw(f: &mut Frame, app: &App, area: Rect, bp: Breakpoint)` — `area` is the body only; header, player, footer
and dialogs are drawn for you. Read state only; no DB/FS access, no big allocations (build only visible rows).
Breakpoint (`ui::layout`): `Full>=120x35, Standard>=100x30, Compact>=80x24, Narrow>=60x20`;
`bp.library_panes() -> usize` (Full=3, Standard/Compact=2, Narrow=1), `bp.optional_columns() -> bool` (Standard+).

## Helpers (`ui::widgets`, `ui::theme::Theme` via `app.theme`)
- `truncate(&str, w) -> Cow<str>`; `fit(&str, w, right: bool) -> String` (exact cell width); `tail(&str, w)`
- `fmt_time(secs) -> String`, `fmt_thousands(usize) -> String`
- `pane_title(&Theme, title, focused) -> Line` (focused: accent bar + accent bold; same width either way)
- `separator(&Theme, width) -> Line`
- `track_row(&Theme, width, cursor: bool, mark: Mark, muted: bool, cols: &[Col], duration: Option<Option<f64>>) -> Line`
  `Mark::{None, Playing, Queued(Option<usize>)}`; `Col::new(text, width).right().style(s).weight(n)` (width 0 = flexible; `.weight(n)` sets its share of leftover space, default 1);
  `duration`: `None` = no duration cell, `Some(None)` = "--:--". Marker column: `›` cursor, ` ▶ ` playing, `·n` queued.
- `draw_rows(f, &Theme, area, selected: Option<usize>, len, |i, is_cursor| -> Line)` — windows the list so the
  selection stays visible, only builds visible rows, applies the raised highlight to the cursor row.
- `empty_state(f, &Theme, area, title, hint)` centered message for empty panes.
- `progress_bar(&Theme, width, Option<f64>) -> Line`.
- Theme styles: `base selected focused_title muted secondary border playing warning error accent queued panel modal tab(active) key strong text_style`.

## Conventions
- Selected = raised bg + `›`; playing = `▶` + playing color (independent of cursor); queued = `·n`;
  unsupported/unavailable = `muted: true` plus an explanation column. Focused pane = `pane_title(.., true)`.
- Use `ListState` from `app.<view>_ui` (`list.selected()`); the draw is stateless (no scroll offset to maintain).
- Never byte-slice strings; never hardcode colors; never depend on Nerd Fonts.

## App API for key handling (app/views/<view>.rs)
`impl App { pub(super) fn <view>_action(&mut self, a: Action) }`, dispatched from `views/mod.rs` (nav keys j/k/g/G/page
are handled there). Use `self.notify(msg)` / `self.notify_error(msg)` (3s), `self.enqueue(tracks)`,
`self.open_picker(tracks)` (add-to-playlist dialog), `self.command(Command::..)`, `self.dialog = Some(Dialog::..)`.
Shortcuts never reach views while a dialog or search is open. State read by views: `app.tracks`, `app.library.{list,filter,filtered}`,
`app.browser.{dir,entries,list}`, `app.playlists`, `app.pl_ui.{list,items,items_list,focus_items}`,
`app.player.queue().entries()` + `current_index()`, `app.queue_ui.list`, `app.history`, `app.hist_ui.list`,
`app.player.snapshot().current` (to mark the playing row), `app.scanning`.

## Footer
The footer shows the toast (notify/notify_error) on the left and the shortcut hints right-aligned in the remaining room.
