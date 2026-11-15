//! Binário do nas: interface ratatui/crossterm e loop de eventos.

mod app;
mod logging;
mod ui;

use std::io;

use nas_editor::{Document, OpenError};
use nas_keys::{Layout, ModalState};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::cursor::{self, SetCursorStyle};
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
    install_panic_hook();

    let log_path = logging::init()?;
    tracing::info!(path = %log_path.display(), "nas iniciado");

    let mut app = App::with_layout(open_document_from_args()?, keyboard_layout());

    let mut terminal = setup_terminal()?;
    let result = run(&mut terminal, &mut app).await;
    restore_terminal();

    if let Err(err) = &result {
        tracing::error!(error = %err, "nas terminou com erro");
    }
    result
}

/// Abre o documento passado na linha de comando. Um arquivo que não é
/// UTF-8 válido não é carregado: a aplicação abre com um buffer vazio e
/// uma mensagem na barra de status, para nunca sobrescrever o original.
fn open_document_from_args() -> io::Result<Document> {
    let Some(path) = std::env::args().nth(1) else {
        return Ok(Document::empty());
    };
    match Document::open(&path) {
        Ok(document) => Ok(document),
        Err(OpenError::NotUtf8) => {
            tracing::warn!(path, "arquivo não é UTF-8 válido; recusado");
            Ok(Document::empty())
        }
        Err(err) => Err(err.into()),
    }
}

/// Layout físico de teclado, escolhido por `NAS_KEYBOARD_LAYOUT`
/// ("qwerty" ou "qwerty-abnt"). Sem a variável, ou com um nome
/// desconhecido, usa qwerty (o padrão do próprio nas-keys).
fn keyboard_layout() -> Layout {
    let Ok(name) = std::env::var("NAS_KEYBOARD_LAYOUT") else {
        return Layout::Qwerty;
    };
    Layout::from_name(&name).unwrap_or_else(|| {
        tracing::warn!(name, "NAS_KEYBOARD_LAYOUT desconhecida; usando qwerty");
        Layout::Qwerty
    })
}

async fn run(terminal: &mut Terminal<Backend>, app: &mut App) -> io::Result<()> {
    loop {
        execute!(io::stdout(), cursor_style_for(app.modal.state()))?;
        terminal.draw(|frame| ui::draw(frame, app))?;

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

/// Forma do cursor por estado modal: barra em Insert, bloco em Visual, e
/// sublinhado em Sequential — o terminal não tem um cursor "oco" como o
/// do Emacs, sublinhado é o mais distinto dos outros dois disponível.
fn cursor_style_for(state: ModalState) -> SetCursorStyle {
    match state {
        ModalState::Insert => SetCursorStyle::SteadyBar,
        ModalState::Visual => SetCursorStyle::SteadyBlock,
        ModalState::Sequential => SetCursorStyle::SteadyUnderScore,
    }
}

fn setup_terminal() -> io::Result<Terminal<Backend>> {
    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen)?;
    if supports_keyboard_enhancement().unwrap_or(false) {
        execute!(
            io::stdout(),
            event::PushKeyboardEnhancementFlags(
                KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
            )
        )?;
    }
    Terminal::new(CrosstermBackend::new(io::stdout()))
}

/// Restaura o terminal ao estado normal. Best-effort: nunca propaga erro,
/// porque também é chamada do panic hook, onde já estamos no meio de um
/// erro fatal e uma falha aqui não deve impedir a mensagem de pânico de
/// aparecer.
fn restore_terminal() {
    if supports_keyboard_enhancement().unwrap_or(false) {
        let _ = execute!(io::stdout(), event::PopKeyboardEnhancementFlags);
    }
    let _ = execute!(io::stdout(), cursor::Show);
    let _ = execute!(io::stdout(), SetCursorStyle::DefaultUserShape);
    let _ = execute!(io::stdout(), LeaveAlternateScreen);
    let _ = disable_raw_mode();
}

/// Garante que, mesmo num pânico, o terminal do usuário volte ao normal
/// antes da mensagem de erro ser impressa — sem isso, um pânico em modo
/// raw/tela alternada deixa o terminal num estado ilegível.
fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_terminal();
        default_hook(info);
    }));
}
