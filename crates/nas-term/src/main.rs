//! Binário do nas: interface ratatui/crossterm e loop de eventos.

mod app;
mod logging;
mod ui;

use std::io::{self, Write as _};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine as _;
use nas_editor::{Document, OpenError};
use nas_keys::{Layout, ModalState};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::cursor::{self, SetCursorStyle};
use ratatui::crossterm::event::{
    self, DisableBracketedPaste, DisableFocusChange, EnableBracketedPaste, EnableFocusChange,
    Event, KeyEventKind, KeyboardEnhancementFlags,
};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
    supports_keyboard_enhancement,
};

use app::{App, SharedJournal};

type Backend = CrosstermBackend<io::Stdout>;

/// Intervalo da checagem leve de mudança externa — "a cada poucos
/// segundos", docs/decisions/0005 §1. Também é o timeout do `poll`
/// que mantém o loop de eventos vivo sem bloquear indefinidamente:
/// cada vez que `poll` retorna sem evento nenhum, já se passou esse
/// tempo desde a última volta, então é a hora certa de conferir —
/// sem precisar de um temporizador separado. Se o usuário está
/// digitando sem pausa (eventos chegando mais rápido que isto), a
/// checagem simplesmente espera a próxima pausa; os outros três
/// momentos (trocar de buffer, salvar, foco) continuam cobrindo esse
/// caso.
const EXTERNAL_CHANGE_CHECK_INTERVAL: Duration = Duration::from_secs(3);

#[tokio::main(flavor = "current_thread")]
async fn main() -> io::Result<()> {
    // Verificado antes de qualquer outra coisa — `--check-config`
    // nunca abre terminal nenhum (docs/decisions/0007 §3, Parte 2).
    if std::env::args().any(|arg| arg == "--check-config") {
        return check_config();
    }

    // `--mcp` também sai antes de qualquer coisa que toque o
    // terminal: sem isso, o panic hook normal (que restaura o
    // terminal escrevendo códigos ANSI na stdout) e o próprio
    // `setup_terminal` quebrariam a regra de nunca escrever nada na
    // stdout além do protocolo MCP (ADR 0006-ia-trabalha-pelo-nas.md
    // §3, etapa 3c).
    if std::env::args().any(|arg| arg == "--mcp") {
        return run_mcp().await;
    }

    // Criado antes de qualquer outra coisa (até do log) pra que o
    // panic hook, instalado logo abaixo, já tenha o mesmo `Arc` que
    // `app` vai usar — um pânico em qualquer ponto depois daqui força
    // o flush do journal, mesmo que o fio normal (Ctrl+Q, Ctrl+K,
    // Ctrl+S) nunca seja alcançado.
    let history: SharedJournal = Arc::new(Mutex::new(None));
    install_panic_hook(history.clone());

    let log_path = logging::init()?;
    tracing::info!(path = %log_path.display(), "nas iniciado");

    let project = discover_project();
    if let Some((project, state_home)) = &project {
        *history.lock().unwrap() = open_project_journal(project, state_home);
    }

    // Configuração em camadas (docs/decisions/0007): lida e validada
    // por completo antes de qualquer coisa ser aplicada (§3) — um
    // erro aqui nunca impede o nas de abrir, só mantém o padrão
    // embutido e avisa na echo area assim que a tela existir.
    // `--no-config` ignora as duas camadas de propósito — a saída
    // pra quando a própria configuração deixa o nas inutilizável
    // (docs/config.md).
    let no_config = std::env::args().any(|arg| arg == "--no-config");
    let project_root = project.as_ref().map(|(p, _)| p.root().to_path_buf());
    let user_config_dir = nas_config::user_config_dir().ok();

    let (config, keymap, startup_message) = if no_config {
        (nas_config::Config::default(), None, None)
    } else {
        match nas_config::load(user_config_dir.as_deref(), project_root.as_deref()) {
            Ok(loaded) => {
                let message = (!loaded.warnings.is_empty()).then(|| {
                    format!(
                        "Aviso(s) na configuração do projeto, ignorada — {}",
                        nas_config::format_errors(&loaded.warnings)
                    )
                });
                (loaded.config, Some(loaded.keymap), message)
            }
            Err(errors) => (
                nas_config::Config::default(),
                None,
                Some(format!(
                    "Configuração inválida, usando o padrão — {}",
                    nas_config::format_errors(&errors)
                )),
            ),
        }
    };

    let layout = keyboard_layout(config.keyboard_layout);
    let mut app = App::with_layout(open_document_from_args()?, layout);
    app.attach_history(history);
    if no_config {
        app.attach_config(config, None, None);
    } else {
        app.attach_config(config, user_config_dir, project_root);
    }
    if let Some(keymap) = keymap {
        app.modal.set_keymap(keymap);
    }
    if let Some((project, state_home)) = project {
        app.attach_project(project, state_home);
    }
    if let Some(message) = startup_message {
        app.show_startup_message(message);
    }

    let mut terminal = setup_terminal()?;
    let result = run(&mut terminal, &mut app).await;
    restore_terminal();

    if let Err(err) = &result {
        tracing::error!(error = %err, "nas terminou com erro");
    }
    result
}

/// `nas --check-config`: valida as duas camadas de dados
/// (docs/decisions/0007) sem abrir nenhum terminal — imprime os
/// erros (se houver) e sai com código diferente de zero.
fn check_config() -> io::Result<()> {
    let project_root = discover_project().map(|(project, _)| project.root().to_path_buf());
    let user_config_dir = nas_config::user_config_dir().ok();
    match nas_config::load(user_config_dir.as_deref(), project_root.as_deref()) {
        Ok(_) => {
            println!("Configuração válida.");
            Ok(())
        }
        Err(errors) => {
            eprintln!("{}", nas_config::format_errors(&errors));
            Err(io::Error::other("configuração inválida"))
        }
    }
}

/// `nas --mcp`: sobe o servidor MCP (`nas-mcp`) sobre stdio — nunca
/// abre terminal, nunca instala o panic hook que restaura o
/// terminal (ADR 0006-ia-trabalha-pelo-nas.md §3, etapa 3c). Logging
/// continua só em arquivo (`logging::init`, igual ao modo normal); um
/// pânico aqui só vai pro log, nunca pra stdout.
///
/// `--approve-writes` (temporário, só pra testar antes da aprovação
/// de verdade na etapa 3d — ver a ADR) libera `save_file`
/// (`Effect::WriteFile`); nunca `Shell`/`Network`, que não têm
/// comando nenhum ainda.
async fn run_mcp() -> io::Result<()> {
    let log_path = logging::init()?;
    tracing::info!(path = %log_path.display(), "nas --mcp iniciado");

    std::panic::set_hook(Box::new(|info| {
        tracing::error!(%info, "nas --mcp entrou em pânico");
    }));

    let approve_writes = std::env::args().any(|arg| arg == "--approve-writes");
    let project = discover_project();
    let project_root = match &project {
        Some((project, _)) => project.root().to_path_buf(),
        None => std::env::current_dir()?,
    };
    let history_dir = project
        .as_ref()
        .and_then(|(project, state_home)| project.ensure_history_dir(state_home).ok());

    let options = nas_mcp::McpOptions {
        project_root,
        history_dir,
        approve_writes,
    };
    nas_mcp::run_stdio(options).await
}

/// Abre o documento passado na linha de comando — o primeiro
/// argumento que não comece com `--` (pra `--check-config`/
/// `--no-config` nunca serem lidos como nome de arquivo). Um arquivo
/// que não é UTF-8 válido não é carregado: a aplicação abre com um
/// buffer vazio e uma mensagem na barra de status, para nunca
/// sobrescrever o original.
fn open_document_from_args() -> io::Result<Document> {
    let Some(path) = std::env::args().skip(1).find(|arg| !arg.starts_with("--")) else {
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

/// Identifica o projeto (raiz do Git ou diretório atual) e onde fica
/// `$XDG_STATE_HOME` — ver docs/decisions/0005-historico-com-autoria.md
/// §5/§6. `None` se não for possível (sem `HOME`, por exemplo): o nas
/// continua funcionando normalmente, só sem histórico nem checkpoints
/// — nunca um motivo para recusar abrir.
fn discover_project() -> Option<(nas_history::ProjectId, std::path::PathBuf)> {
    let cwd = std::env::current_dir().ok()?;
    let project = nas_history::ProjectId::discover(&cwd).ok()?;
    let state_home = nas_history::state_home().ok()?;
    Some((project, state_home))
}

/// Abre o journal de histórico do projeto já identificado por
/// [`discover_project`]. `None` se a pasta de histórico ou o journal
/// não puderem ser criados/abertos (sem permissão de escrita, etc.).
fn open_project_journal(
    project: &nas_history::ProjectId,
    state_home: &std::path::Path,
) -> Option<nas_history::Journal> {
    let history_dir = project.ensure_history_dir(state_home).ok()?;
    match nas_history::Journal::open(&history_dir) {
        Ok(journal) => Some(journal),
        Err(err) => {
            tracing::warn!(error = %err, "não foi possível abrir o journal de histórico");
            None
        }
    }
}

/// Layout físico de teclado: `NAS_KEYBOARD_LAYOUT` ("qwerty" ou
/// "qwerty-abnt"), se definida e válida — vence por ser uma troca
/// rápida, de uma sessão só, sem precisar editar um arquivo; senão
/// `keyboard_layout` de `config.toml` (docs/decisions/0007), que já
/// chega resolvido em `config_layout`.
pub(crate) fn keyboard_layout(config_layout: Layout) -> Layout {
    let Ok(name) = std::env::var("NAS_KEYBOARD_LAYOUT") else {
        return config_layout;
    };
    Layout::from_name(&name).unwrap_or_else(|| {
        tracing::warn!(
            name,
            "NAS_KEYBOARD_LAYOUT desconhecida; usando a configuração"
        );
        config_layout
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

        // `poll` com um timeout em vez de `read` bloqueando sem fim:
        // quando ele retorna `false` (nenhum evento chegou dentro do
        // intervalo), é a vez da checagem leve de mudança externa —
        // ver `EXTERNAL_CHANGE_CHECK_INTERVAL`.
        if !event::poll(EXTERNAL_CHANGE_CHECK_INTERVAL)? {
            app.check_external_changes();
        } else {
            match event::read()? {
                Event::Key(key) if key.kind != KeyEventKind::Release => app.handle_key(key),
                // Texto colado via bracketed paste nunca passa pela
                // máquina modal — sem isso, colar algo como "dd" seria
                // interpretado tecla por tecla como comandos do estado
                // Visual, em vez de simplesmente inserido como texto.
                Event::Paste(text) => app.handle_paste(text),
                Event::FocusGained => app.handle_focus_gained(),
                _ => {}
            }
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
    // `Event::FocusGained` é um dos quatro momentos em que o nas
    // confere mudança externa (docs/decisions/0005 §1) — sem
    // habilitar isto, o terminal nunca reporta foco.
    execute!(io::stdout(), EnableFocusChange)?;
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
    let _ = execute!(io::stdout(), DisableFocusChange);
    let _ = execute!(io::stdout(), LeaveAlternateScreen);
    let _ = disable_raw_mode();
}

/// Garante que, mesmo num pânico, o terminal do usuário volte ao normal
/// antes da mensagem de erro ser impressa — sem isso, um pânico em modo
/// raw/tela alternada deixa o terminal num estado ilegível. Também
/// tenta o flush do journal de histórico (`history`, o mesmo `Arc` que
/// `app` usa), pra não perder edições feitas pouco antes do pânico.
fn install_panic_hook(history: SharedJournal) {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_terminal();
        try_flush_history(&history);
        default_hook(info);
    }));
}

/// Flush de melhor esforço no journal compartilhado — **nunca
/// bloqueia**. Usa `try_lock` em vez de `lock`: se o lock estiver
/// ocupado (por exemplo, o pânico aconteceu no meio de uma gravação,
/// com o lock preso no fio que entrou em pânico), desiste do flush e
/// segue adiante — um panic hook travado deixaria o terminal do
/// usuário sem ser restaurado, o que é pior que perder o flush.
fn try_flush_history(history: &SharedJournal) {
    if let Ok(mut guard) = history.try_lock()
        && let Some(journal) = guard.as_mut()
    {
        let _ = journal.flush(nas_history::now_unix());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn try_flush_history_desiste_sem_travar_se_o_lock_estiver_ocupado() {
        let dir = tempfile::tempdir().expect("diretório temporário");
        let journal = nas_history::Journal::open(dir.path()).expect("abrir journal");
        let history: SharedJournal = Arc::new(Mutex::new(Some(journal)));

        // Simula o pânico acontecendo no meio de uma gravação: o lock
        // já está preso neste mesmo fio. Se `try_flush_history` usasse
        // `lock()` em vez de `try_lock()`, esta chamada travaria pra
        // sempre (deadlock no mesmo fio) e o teste nunca terminaria.
        let _guard = history.lock().unwrap();
        try_flush_history(&history);
    }
}
