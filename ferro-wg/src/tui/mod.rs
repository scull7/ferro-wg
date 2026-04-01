//! Interactive TUI — terminal setup, event loop, and teardown.

pub mod app;
pub mod event;
pub mod ui;

use std::io;
use std::time::Duration;

use crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use app::App;
use event::{AppEvent, EventHandler};
use ferro_wg_core::config::WgConfig;

/// Run the interactive TUI.
///
/// # Errors
///
/// Returns an error if terminal setup, event handling, or teardown fails.
pub async fn run(wg_config: WgConfig) -> Result<(), Box<dyn std::error::Error>> {
    let mut app = App::new(wg_config);

    // Setup terminal.
    crossterm::terminal::enable_raw_mode()?;
    let mut stdout = io::stdout();
    crossterm::execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    // Run the event loop, then restore terminal regardless of outcome.
    let result = event_loop(&mut terminal, &mut app).await;

    let _ = crossterm::terminal::disable_raw_mode();
    let _ = crossterm::execute!(terminal.backend_mut(), LeaveAlternateScreen);
    let _ = terminal.show_cursor();

    result
}

/// Drive the TUI event loop until the user quits.
async fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut events = EventHandler::new(Duration::from_millis(250));

    while app.running {
        terminal.draw(|frame| ui::draw(frame, app))?;

        match events.next().await {
            Some(AppEvent::Key(key)) => app.handle_key(key),
            Some(AppEvent::Tick) => {
                // Future: refresh stats from TunnelManager here.
            }
            None => break,
        }
    }

    Ok(())
}
