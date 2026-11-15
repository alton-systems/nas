//! Binário do nas: interface ratatui/crossterm e loop de eventos.

mod app;
mod logging;
mod ui;

use std::io;

use nas_editor::Document;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::event::{self, Event, KeyEventKind, KeyboardEnhancementFlags};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
    supports_keyboard_enhancement,
};

use app::App;

type Backend = CrosstermBackend<io::Stdout>;

#[tokio::main(flavor = "current_thread")]
async fn main() -> io::Result<()> {
    let log_path = logging::init()?;
    tracing::info!(path = %log_path.display(), "nas iniciado");

    let document = match std::env::args().nth(1) {
        Some(path) => Document::open(path)?,
        None => Document::empty(),
    };

    let mut terminal = setup_terminal()?;
    let result = run(&mut terminal, App::new(document)).await;
    restore_terminal(&mut terminal)?;

    if let Err(err) = &result {
        tracing::error!(error = %err, "nas terminou com erro");
    }
    result
}

async fn run(terminal: &mut Terminal<Backend>, mut app: App) -> io::Result<()> {
    loop {
        terminal.draw(|frame| ui::draw(frame, &mut app))?;

        if let Event::Key(key) = event::read()? {
            if key.kind != KeyEventKind::Release {
                app.handle_key(key);
            }
        }

        if app.should_quit {
            return Ok(());
        }
    }
}

fn setup_terminal() -> io::Result<Terminal<Backend>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    if supports_keyboard_enhancement().unwrap_or(false) {
        execute!(
            stdout,
            event::PushKeyboardEnhancementFlags(
                KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
            )
        )?;
    }
    Terminal::new(CrosstermBackend::new(stdout))
}

fn restore_terminal(terminal: &mut Terminal<Backend>) -> io::Result<()> {
    if supports_keyboard_enhancement().unwrap_or(false) {
        execute!(terminal.backend_mut(), event::PopKeyboardEnhancementFlags)?;
    }
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    disable_raw_mode()?;
    Ok(())
}
