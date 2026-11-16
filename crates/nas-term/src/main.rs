//! Binário do nas: interface ratatui/crossterm e loop de eventos.

mod app;
mod logging;
mod ui;

use std::io::{self, Write as _};

use base64::Engine as _;
use nas_editor::{Document, OpenError};
use nas_keys::{Layout, ModalState};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::cursor::{self, SetCursorStyle};
use ratatui::crossterm::event::{
    self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyEventKind,
    KeyboardEnhancementFlags,
};
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
    // Uma só instância, viva pelo resto do processo: se criarmos e
    // derrubarmos um `arboard::Clipboard` a cada cópia, o X11 não tem
    // tempo de entregar o conteúdo a um clipboard manager e o arboard
    // imprime um aviso direto no stderr — o que corrompe a tela do TUI,
    // já que stderr continua ligado ao terminal.
    let mut clipboard = arboard::Clipboard::new().ok();

    loop {
        execute!(io::stdout(), cursor_style_for(app))?;
        terminal.draw(|frame| ui::draw(frame, app))?;

        match event::read()? {
            Event::Key(key) if key.kind != KeyEventKind::Release => app.handle_key(key),
            // Texto colado via bracketed paste nunca passa pela máquina
            // modal — sem isso, colar algo como "dd" seria interpretado
            // tecla por tecla como comandos do estado Visual, em vez de
            // simplesmente inserido como texto.
            Event::Paste(text) => app.handle_paste(text),
            _ => {}
        }

        if let Some(text) = app.pending_clipboard_write.take() {
            write_system_clipboard(&mut clipboard, &text);
        }

        if app.pending_paste {
            // Lê a área de transferência do sistema primeiro, pra colar
            // o que foi copiado em outro programa; se não der (sem
            // display, terminal sem suporte), o que já está em
            // `app.clipboard` (o último copiar/recortar dentro do nas)
            // continua valendo.
            if let Some(text) = read_system_clipboard(&mut clipboard) {
                app.clipboard = text;
            }
            app.apply_pending_paste();
        }

        if app.should_quit {
            return Ok(());
        }
    }
}

/// Escreve `text` na área de transferência do sistema por dois
/// caminhos independentes: OSC 52 (sequência de escape que o próprio
/// terminal intercepta — funciona no Kitty e, por passar pelo próprio
/// fluxo do terminal, também via SSH) e `arboard` (fala direto com o
/// clipboard do sistema operacional — alternativa para quando o
/// terminal não suporta OSC 52, mas não funciona via SSH sem um
/// display local). Melhor esforço: nenhuma falha aqui é fatal — o
/// registro interno (`App::clipboard`) já garante colar dentro do nas
/// mesmo se os dois falharem.
fn write_system_clipboard(clipboard: &mut Option<arboard::Clipboard>, text: &str) {
    let encoded = base64::engine::general_purpose::STANDARD.encode(text);
    let mut stdout = io::stdout();
    let _ = write!(stdout, "\x1b]52;c;{encoded}\x07");
    let _ = stdout.flush();

    if let Some(clipboard) = clipboard {
        let _ = clipboard.set_text(text);
    }
}

/// Lê a área de transferência do sistema via `arboard`. `None` se não
/// houver acesso a uma de verdade (sem display gráfico, por exemplo —
/// o caso comum numa sessão SSH) ou se estiver vazia; quem chama volta
/// pro registro interno (`App::clipboard`) nesse caso. OSC 52 não
/// participa daqui: ler a área de transferência por OSC 52 depende do
/// terminal responder à consulta, o que a maioria não faz por padrão
/// (questão de segurança) — só `arboard` é confiável pra leitura.
fn read_system_clipboard(clipboard: &mut Option<arboard::Clipboard>) -> Option<String> {
    clipboard.as_mut()?.get_text().ok()
}

/// Forma do cursor: barra em Insert (e numa linha de entrada, que
/// também é edição de texto), bloco em Visual, sublinhado em Sequential
/// — o terminal não tem um cursor "oco" como o do Emacs, sublinhado é o
/// mais distinto dos outros dois disponível.
fn cursor_style_for(app: &App) -> SetCursorStyle {
    if app.prompt.is_some() {
        return SetCursorStyle::SteadyBar;
    }
    match app.modal.state() {
        ModalState::Insert => SetCursorStyle::SteadyBar,
        ModalState::Visual => SetCursorStyle::SteadyBlock,
        ModalState::Sequential => SetCursorStyle::SteadyUnderScore,
    }
}

fn setup_terminal() -> io::Result<Terminal<Backend>> {
    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen)?;
    execute!(io::stdout(), EnableBracketedPaste)?;
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
    let _ = execute!(io::stdout(), DisableBracketedPaste);
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
