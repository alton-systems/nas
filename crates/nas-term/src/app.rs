//! Estado da aplicação: liga o teclado (crossterm) à máquina de estados
//! do nas-keys e suas ações aos documentos abertos. Atalhos com Ctrl
//! (classe sistema: salvar, sair, abrir arquivo, trocar e fechar
//! buffer) não fazem parte do vocabulário do nas-keys — são tratados
//! aqui, antes de chegar à máquina de estados. Ver docs/keys.md.

use std::path::{Path, PathBuf};

use nas_editor::{Document, OpenError, RegionObject};
use nas_keys::{Action, Key, KeyCode as NasKeyCode, Layout, Machine, Object, SystemAction, Verb};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Um buffer aberto: o documento mais o estado de rolagem que é dele,
/// não da aplicação como um todo (voltar a um buffer deveria mostrar
/// onde você o deixou). O cursor, a seleção e o histórico de desfazer
/// já são do próprio `Document`.
pub struct Buffer {
    pub document: Document,
    pub scroll_top: usize,
    pub scroll_left: usize,
}

impl Buffer {
    fn new(document: Document) -> Self {
        Self {
            document,
            scroll_top: 0,
            scroll_left: 0,
        }
    }
}

/// O que uma linha de entrada está pedindo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptKind {
    OpenFile,
    Search,
    SwitchBuffer,
    GotoLine,
}

/// Uma linha de entrada ativa na barra de status — equivalente ao
/// minibuffer do Emacs que Ctrl+o (find-file), líder `f`
/// (isearch-forward), Ctrl+b (switch-to-buffer) e `g l` (goto-line)
/// abrem no nas-keys real. Enquanto ativa, nenhuma tecla chega à
/// máquina modal: o texto digitado é só o conteúdo do prompt, nunca
/// interpretado como comando do estado Visual.
pub struct Prompt {
    pub kind: PromptKind,
    pub input: String,
    /// Posição do cursor antes da busca começar, pra restaurar se o
    /// usuário cancelar. `None` fora de uma busca.
    search_origin: Option<usize>,
}

pub struct App {
    pub buffers: Vec<Buffer>,
    pub active: usize,
    pub modal: Machine,
    pub should_quit: bool,
    pub quit_confirm_pending: bool,
    /// Ctrl+k (fechar o buffer atual) foi pressionado uma vez num
    /// buffer com alterações não salvas; a mesma tecla de novo
    /// confirma. Como `quit_confirm_pending`, não é limpo
    /// incondicionalmente — só por outra tecla qualquer, não por
    /// Ctrl+k de novo.
    close_confirm_pending: bool,
    /// Um verbo que apaga ou recorta (`d a`/`x a`) o arquivo inteiro
    /// pediu confirmação — guarda qual verbo, pra só confirmar se a
    /// mesma combinação repetir. Limpo por qualquer ação que não seja
    /// essa mesma repetição.
    whole_buffer_confirm_pending: Option<Verb>,
    pub status_message: Option<String>,
    pub prompt: Option<Prompt>,
    /// Registro interno da área de transferência — sempre atualizado em
    /// copiar/recortar. É o que `y` cola quando a área de transferência
    /// do sistema não está acessível (ex.: SSH sem terminal compatível,
    /// sessão sem display gráfico); quando está, `y` lê de lá primeiro,
    /// pra colar o que foi copiado em outro programa.
    pub clipboard: String,
    /// Texto que `main.rs` deve escrever na área de transferência do
    /// sistema (OSC 52 + `arboard`) depois desta tecla — `App` não tem
    /// acesso a E/S real, pra ficar testável sem terminal nem display.
    pub pending_clipboard_write: Option<String>,
    /// `y` foi pressionado: `main.rs` deve tentar atualizar `clipboard`
    /// a partir da área de transferência do sistema (`arboard`) e então
    /// chamar `apply_pending_paste`. Mesmo motivo do campo acima — ler
    /// a área de transferência do sistema é E/S de verdade, que `App`
    /// não faz sozinho.
    pub pending_paste: bool,
}

impl App {
    /// Traduz teclas do layout físico de teclado dado antes de repassá-las
    /// à máquina modal (ver `NAS_KEYBOARD_LAYOUT` em `main.rs`).
    pub fn with_layout(document: Document, layout: Layout) -> Self {
        Self {
            buffers: vec![Buffer::new(document)],
            active: 0,
            modal: Machine::with_layout(layout),
            should_quit: false,
            quit_confirm_pending: false,
            close_confirm_pending: false,
            whole_buffer_confirm_pending: None,
            status_message: None,
            prompt: None,
            clipboard: String::new(),
            pending_clipboard_write: None,
            pending_paste: false,
        }
    }

    pub fn document(&self) -> &Document {
        &self.buffers[self.active].document
    }

    pub fn document_mut(&mut self) -> &mut Document {
        &mut self.buffers[self.active].document
    }

    pub fn scroll_top(&self) -> usize {
        self.buffers[self.active].scroll_top
    }

    pub fn scroll_left(&self) -> usize {
        self.buffers[self.active].scroll_left
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        if self.prompt.is_some() {
            self.handle_prompt_key(key);
            return;
        }

        if key.modifiers.contains(KeyModifiers::CONTROL) {
            self.handle_system_key(key);
            return;
        }

        let Some(nas_key) = translate_key(key) else {
            return;
        };

        let has_selection = self.document().selection_range().is_some();
        for action in self.modal.handle_key(nas_key, has_selection) {
            self.apply_action(action);
        }
    }

    /// Ctrl + tecla: a classe sistema (docs/keys.md §5.7), resolvida
    /// pelo keymap (`[system]` em `default.toml`), não mais escrita
    /// direto aqui. `Quit` e `CloseBuffer` cuidam da própria
    /// confirmação (gerenciam seus próprios campos de pendência, que
    /// não devem ser limpos pela repetição da mesma tecla); os demais
    /// limpam qualquer mensagem/confirmação pendente antes de agir.
    fn handle_system_key(&mut self, key: KeyEvent) {
        let Some(nas_key) = translate_key(key) else {
            return;
        };
        let Some(system_action) = self.modal.system_action(nas_key) else {
            return;
        };

        if system_action == SystemAction::Quit {
            self.handle_quit_request();
            return;
        }
        if system_action == SystemAction::CloseBuffer {
            self.close_buffer();
            return;
        }

        self.quit_confirm_pending = false;
        self.close_confirm_pending = false;
        self.status_message = None;

        match system_action {
            SystemAction::Save => self.save(),
            SystemAction::OpenFile => self.open_prompt(PromptKind::OpenFile),
            SystemAction::SwitchBuffer => self.open_prompt(PromptKind::SwitchBuffer),
            SystemAction::Quit | SystemAction::CloseBuffer => unreachable!(),
        }
    }

    fn apply_action(&mut self, action: Action) {
        let repeats_whole_buffer_confirm = matches!(
            action,
            Action::VerbObject(verb, Object::WholeBuffer)
                if Some(verb) == self.whole_buffer_confirm_pending
        );
        if !repeats_whole_buffer_confirm {
            self.whole_buffer_confirm_pending = None;
        }

        match action {
            Action::MoveLeft => self.document_mut().move_left(),
            Action::MoveRight => self.document_mut().move_right(),
            Action::MoveUp => self.document_mut().move_up(),
            Action::MoveDown => self.document_mut().move_down(),
            Action::MoveLineStart => self.document_mut().move_to_line_start(),
            Action::MoveLineEnd => self.document_mut().move_to_line_end(),
            Action::MoveBufferStart => self.document_mut().move_to_buffer_start(),
            Action::MoveBufferEnd => self.document_mut().move_to_buffer_end(),
            Action::MoveWordForward => self.document_mut().move_word_forward(),
            Action::MoveWordBackward => self.document_mut().move_word_backward(),
            Action::MoveParagraphForward => self.document_mut().move_paragraph_forward(),
            Action::MoveParagraphBackward => self.document_mut().move_paragraph_backward(),
            Action::DeleteForward => self.document_mut().delete_forward(),
            Action::DeleteBackward => self.document_mut().delete_backward(),
            Action::Undo => self.undo(),
            Action::InsertChar(c) => self.document_mut().insert_char(c),
            Action::InsertNewline => self.document_mut().insert_newline(),
            Action::PromptSearch => {
                let origin = self.document().cursor_offset();
                self.prompt = Some(Prompt {
                    kind: PromptKind::Search,
                    input: String::new(),
                    search_origin: Some(origin),
                });
            }
            Action::PromptGotoLine => self.open_prompt(PromptKind::GotoLine),
            Action::StartSelection => self.document_mut().start_selection(),
            Action::SelectAll => self.document_mut().select_all(),
            Action::Copy => self.copy(),
            Action::Cut => self.cut(),
            Action::DeleteSelection => self.document_mut().delete_selection(),
            Action::Paste => self.pending_paste = true,
            Action::VerbObject(verb, object) => self.apply_verb_object(verb, object),
        }
    }

    fn open_prompt(&mut self, kind: PromptKind) {
        self.prompt = Some(Prompt {
            kind,
            input: String::new(),
            search_origin: None,
        });
    }

    /// Um verbo agindo sobre um objeto (docs/keys.md §4.3). `d a`/`x a`
    /// (apagar/recortar o arquivo inteiro) pedem confirmação antes de
    /// executar — a mesma combinação repetida confirma.
    fn apply_verb_object(&mut self, verb: Verb, object: Object) {
        if object == Object::WholeBuffer && matches!(verb, Verb::Delete | Verb::Cut) {
            if self.whole_buffer_confirm_pending == Some(verb) {
                self.whole_buffer_confirm_pending = None;
            } else {
                self.whole_buffer_confirm_pending = Some(verb);
                self.status_message = Some(format!(
                    "{} o arquivo inteiro? repita pra confirmar.",
                    verb_label(verb)
                ));
                return;
            }
        }

        let region = to_region_object(object);
        let (from, to) = self.document().region_for(region);
        match verb {
            Verb::Copy => {
                let text = self.document().text_in(from, to);
                self.write_clipboard_if_nonempty(text);
            }
            Verb::Delete | Verb::Cut => {
                let text = self.document_mut().delete_region(from, to);
                self.write_clipboard_if_nonempty(text);
            }
        }
    }

    fn write_clipboard_if_nonempty(&mut self, text: String) {
        if !text.is_empty() {
            self.clipboard = text.clone();
            self.pending_clipboard_write = Some(text);
        }
    }

    /// Insere o conteúdo atual de `clipboard` no documento, e desarma
    /// `pending_paste`. Chamado por `main.rs` depois de tentar atualizar
    /// `clipboard` a partir da área de transferência do sistema — se
    /// essa tentativa falhar (sem display, terminal sem suporte), o
    /// registro interno (o que já estava em `clipboard`) é quem cola.
    /// Com uma seleção ativa, substitui a seleção (docs/keys.md §4.2).
    /// Não faz nada se não houver um `y` pendente.
    pub fn apply_pending_paste(&mut self) {
        if !std::mem::take(&mut self.pending_paste) {
            return;
        }
        if self.document().selection_range().is_some() {
            self.document_mut().delete_selection();
        }
        let text = self.clipboard.clone();
        self.paste_text(&text);
    }

    fn copy(&mut self) {
        if let Some(text) = self.document().selected_text() {
            self.write_clipboard_if_nonempty(text);
        }
        self.document_mut().clear_selection();
    }

    fn cut(&mut self) {
        if let Some(text) = self.document().selected_text() {
            self.write_clipboard_if_nonempty(text);
        }
        self.document_mut().delete_selection();
    }

    /// Insere `text` no documento, caractere a caractere — exceto
    /// quebras de linha (`"\n"`, `"\r\n"` ou `"\r"` sozinho, tratadas
    /// como uma única quebra), que passam por `insert_newline` pra
    /// respeitar o estilo de quebra de linha do documento atual, em vez
    /// de reproduzir literalmente o que veio colado.
    fn paste_text(&mut self, text: &str) {
        let mut chars = text.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '\n' => self.document_mut().insert_newline(),
                '\r' => {
                    if chars.peek() == Some(&'\n') {
                        chars.next();
                    }
                    self.document_mut().insert_newline();
                }
                _ => self.document_mut().insert_char(c),
            }
        }
    }

    /// Texto colado pelo terminal via bracketed paste
    /// (`Event::Paste` do crossterm) — nunca passa pela máquina modal,
    /// então nunca é interpretado como comando do estado Visual, só
    /// como texto. Vai pro prompt ativo, se houver um; senão, substitui
    /// uma seleção ativa (como `y`) e insere direto no documento,
    /// independente do estado modal.
    pub fn handle_paste(&mut self, text: String) {
        if self.prompt.is_some() {
            if let Some(prompt) = &mut self.prompt {
                prompt.input.push_str(&text);
            }
            self.update_incremental_search();
            return;
        }
        if self.document().selection_range().is_some() {
            self.document_mut().delete_selection();
        }
        self.paste_text(&text);
    }

    /// Enquanto uma linha de entrada está ativa, só ela recebe teclas —
    /// nada chega à máquina modal (equivalente ao `minibuffer-setup-hook`
    /// do nas-keys real, que troca pra Insert, mas aqui é mais simples
    /// nem repassar pra máquina nenhuma).
    fn handle_prompt_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => self.cancel_prompt(),
            KeyCode::Enter => self.confirm_prompt(),
            KeyCode::Backspace => {
                if let Some(prompt) = &mut self.prompt {
                    prompt.input.pop();
                }
                self.update_incremental_search();
            }
            KeyCode::Char(c)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                if let Some(prompt) = &mut self.prompt {
                    prompt.input.push(c);
                }
                self.update_incremental_search();
            }
            _ => {}
        }
    }

    /// A cada tecla da busca, refaz a busca inteira a partir de onde ela
    /// começou (não de onde o cursor está agora, que já pode ter saltado
    /// pro match anterior) — assim apagar um caractere da consulta pode
    /// achar uma ocorrência mais cedo no documento.
    fn update_incremental_search(&mut self) {
        let Some(prompt) = &self.prompt else { return };
        let (PromptKind::Search, Some(origin)) = (prompt.kind, prompt.search_origin) else {
            return;
        };
        let query = prompt.input.clone();
        if let Some(pos) = self.document().find_from(origin, &query) {
            self.document_mut().set_cursor(pos);
        }
    }

    fn confirm_prompt(&mut self) {
        let Some(prompt) = self.prompt.take() else {
            return;
        };
        match prompt.kind {
            PromptKind::OpenFile => self.open_file(&prompt.input),
            // O cursor já está no último match encontrado (ou de volta
            // na posição original, se nada bateu) — só sair do prompt.
            PromptKind::Search => {}
            PromptKind::SwitchBuffer => self.switch_buffer(&prompt.input),
            PromptKind::GotoLine => self.goto_line(&prompt.input),
        }
    }

    fn cancel_prompt(&mut self) {
        let Some(prompt) = self.prompt.take() else {
            return;
        };
        if let Some(origin) = prompt.search_origin {
            self.document_mut().set_cursor(origin);
        }
    }

    /// Abre `path` num buffer novo — ou troca pro buffer já aberto com
    /// esse caminho, se houver um. Não recusa por causa de alterações
    /// não salvas: o buffer atual continua intacto em segundo plano.
    fn open_file(&mut self, path: &str) {
        let path = path.trim();
        if path.is_empty() {
            self.status_message = Some("Nenhum arquivo informado.".to_string());
            return;
        }
        let resolved = expand_path(path);

        if let Some(index) = self.find_buffer_by_path(&resolved) {
            self.active = index;
            return;
        }

        match Document::open(&resolved) {
            Ok(document) => {
                self.buffers.push(Buffer::new(document));
                self.active = self.buffers.len() - 1;
            }
            Err(OpenError::NotUtf8) => {
                self.status_message = Some(format!("'{path}' não é UTF-8 válido."));
            }
            Err(err) => {
                self.status_message = Some(format!("Erro ao abrir '{path}': {err}"));
            }
        }
    }

    fn find_buffer_by_path(&self, path: &Path) -> Option<usize> {
        self.buffers
            .iter()
            .position(|b| b.document.path() == Some(path))
    }

    /// Troca pro primeiro buffer aberto cujo nome contém `query`. Com
    /// `query` vazia, ou sem nenhum buffer batendo, mostra uma mensagem
    /// em vez de trocar.
    fn switch_buffer(&mut self, query: &str) {
        let query = query.trim();
        if query.is_empty() {
            self.status_message = Some("Nenhum buffer informado.".to_string());
            return;
        }
        match self
            .buffers
            .iter()
            .position(|b| buffer_name(&b.document).contains(query))
        {
            Some(index) => self.active = index,
            None => {
                self.status_message = Some(format!("Nenhum buffer encontrado para {query:?}."));
            }
        }
    }

    fn goto_line(&mut self, input: &str) {
        match input.trim().parse::<usize>() {
            Ok(line) if line >= 1 => self.document_mut().goto_line(line),
            _ => self.status_message = Some("Número de linha inválido.".to_string()),
        }
    }

    /// Fecha o buffer atual — Ctrl+k. Pede confirmação se ele tiver
    /// alterações não salvas (a mesma tecla de novo confirma). Fechar
    /// o último buffer aberto deixa um `[sem nome]` vazio no lugar: o
    /// nas sempre tem pelo menos um buffer.
    fn close_buffer(&mut self) {
        if self.document().is_modified() && !self.close_confirm_pending {
            self.close_confirm_pending = true;
            self.status_message = Some(format!(
                "{} tem alterações não salvas — Ctrl+K de novo fecha sem salvar.",
                buffer_name(self.document())
            ));
            return;
        }
        self.close_confirm_pending = false;
        self.status_message = None;

        self.buffers.remove(self.active);
        if self.buffers.is_empty() {
            self.buffers.push(Buffer::new(Document::empty()));
            self.active = 0;
        } else if self.active >= self.buffers.len() {
            self.active = self.buffers.len() - 1;
        }
    }

    /// Ajusta a rolagem do buffer ativo para que o cursor fique visível
    /// numa área de `width` x `height` células.
    pub fn ensure_cursor_visible(&mut self, width: usize, height: usize) {
        let position = self.document().cursor_position();
        let visual_column = self.document().cursor_visual_column();
        let buffer = &mut self.buffers[self.active];

        if position.line < buffer.scroll_top {
            buffer.scroll_top = position.line;
        } else if position.line >= buffer.scroll_top + height {
            buffer.scroll_top = position.line + 1 - height;
        }

        if visual_column < buffer.scroll_left {
            buffer.scroll_left = visual_column;
        } else if width > 0 && visual_column >= buffer.scroll_left + width {
            buffer.scroll_left = visual_column + 1 - width;
        }
    }

    /// Pergunta só se algum buffer tiver alterações não salvas,
    /// dizendo quais — Ctrl+S só salva o buffer atual, então a mensagem
    /// deixa isso claro quando há mais de um.
    fn handle_quit_request(&mut self) {
        let modified: Vec<String> = self
            .buffers
            .iter()
            .filter(|b| b.document.is_modified())
            .map(|b| buffer_name(&b.document))
            .collect();

        if modified.is_empty() || self.quit_confirm_pending {
            self.should_quit = true;
            return;
        }
        self.quit_confirm_pending = true;
        self.status_message = Some(format!(
            "Alterações não salvas em: {} — Ctrl+Q de novo sai sem salvar, Ctrl+S salva o buffer atual",
            modified.join(", ")
        ));
    }

    fn save(&mut self) {
        self.status_message = Some(match self.document_mut().save() {
            Ok(true) => "Salvo.".to_string(),
            Ok(false) => "Sem nome de arquivo; nada para salvar.".to_string(),
            Err(err) => format!("Erro ao salvar: {err}"),
        });
    }

    fn undo(&mut self) {
        if !self.document_mut().undo() {
            self.status_message = Some("Nada para desfazer.".to_string());
        }
    }
}

fn verb_label(verb: Verb) -> &'static str {
    match verb {
        Verb::Delete => "Apagar",
        Verb::Copy => "Copiar",
        Verb::Cut => "Recortar",
    }
}

fn to_region_object(object: Object) -> RegionObject {
    match object {
        Object::CharLeft => RegionObject::CharLeft,
        Object::CharRight => RegionObject::CharRight,
        Object::LineStart => RegionObject::LineStart,
        Object::LineEnd => RegionObject::LineEnd,
        Object::LineDown => RegionObject::LineDown,
        Object::LineUp => RegionObject::LineUp,
        Object::BufferEnd => RegionObject::BufferEnd,
        Object::BufferStart => RegionObject::BufferStart,
        Object::WordBackward => RegionObject::WordBackward,
        Object::WordForward => RegionObject::WordForward,
        Object::ParagraphForward => RegionObject::ParagraphForward,
        Object::ParagraphBackward => RegionObject::ParagraphBackward,
        Object::CurrentLine => RegionObject::CurrentLine,
        Object::WholeBuffer => RegionObject::WholeBuffer,
    }
}

/// Nome de exibição de um buffer: o caminho do arquivo, ou
/// `[sem nome]` se ainda não tiver um — usado na barra de status, no
/// prompt de trocar de buffer e nas mensagens de confirmação.
pub fn buffer_name(document: &Document) -> String {
    document
        .path()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "[sem nome]".to_string())
}

/// Traduz um `KeyEvent` do crossterm para a tecla neutra do nas-keys.
/// `None` para teclas que o nas-keys não precisa conhecer (F-keys,
/// PageUp/Down, modificadores sozinhos, etc.).
fn translate_key(key: KeyEvent) -> Option<Key> {
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let code = match key.code {
        KeyCode::Char(c) => NasKeyCode::Char(c),
        KeyCode::Enter => NasKeyCode::Enter,
        KeyCode::Esc => NasKeyCode::Escape,
        KeyCode::Backspace => NasKeyCode::Backspace,
        KeyCode::Delete => NasKeyCode::Delete,
        KeyCode::Tab => NasKeyCode::Tab,
        KeyCode::Left => NasKeyCode::Left,
        KeyCode::Right => NasKeyCode::Right,
        KeyCode::Up => NasKeyCode::Up,
        KeyCode::Down => NasKeyCode::Down,
        _ => return None,
    };
    Some(Key { code, alt })
}

/// Resolve o caminho digitado no prompt de abrir arquivo: `~` (ou
/// `~/...`) expande pro diretório home; um caminho relativo é resolvido
/// a partir do diretório onde o nas foi iniciado — como o nas nunca
/// troca de diretório durante a execução, isso é só o diretório atual
/// do processo. Um caminho absoluto passa direto.
fn expand_path(input: &str) -> PathBuf {
    let path = Path::new(input);
    if let Ok(rest) = path.strip_prefix("~")
        && let Some(home) = std::env::var_os("HOME")
    {
        return PathBuf::from(home).join(rest);
    }
    if path.is_relative()
        && let Ok(cwd) = std::env::current_dir()
    {
        return cwd.join(path);
    }
    path.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;
    use nas_editor::Document;
    use ratatui::crossterm::event::KeyEvent;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn alt_key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::ALT)
    }

    fn ctrl_key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::CONTROL)
    }

    fn text(app: &App) -> String {
        (0..app.document().len_lines())
            .map(|i| app.document().line(i).into_owned())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn type_text(app: &mut App, s: &str) {
        app.handle_key(key(KeyCode::Enter)); // Visual -> Insert
        for c in s.chars() {
            app.handle_key(key(KeyCode::Char(c)));
        }
        app.handle_key(alt_key(KeyCode::Char(' '))); // Insert -> Visual
    }

    #[test]
    fn verbo_mais_objeto_e_desfazer_com_u_pelo_estado_visual() {
        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        type_text(&mut app, "abc");
        app.handle_key(key(KeyCode::Char('g')));
        app.handle_key(key(KeyCode::Char('g'))); // início do buffer
        assert_eq!(text(&app), "abc");

        app.handle_key(key(KeyCode::Char('d')));
        app.handle_key(key(KeyCode::Char(';'))); // d ; : apaga o 'a'
        assert_eq!(text(&app), "bc");
        assert_eq!(app.clipboard, "a");

        app.handle_key(key(KeyCode::Char('u'))); // u: desfaz
        assert_eq!(text(&app), "abc");
    }

    #[test]
    fn ctrl_mais_tecla_nao_mapeada_nao_e_reinterpretado_sem_o_ctrl() {
        // Ctrl+j não é parte do vocabulário do nas-keys; não deve mover o
        // cursor como se fosse só 'j' sem o Ctrl.
        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        type_text(&mut app, "abc");
        let position_before = app.document().cursor_position();

        app.handle_key(ctrl_key(KeyCode::Char('j')));

        assert_eq!(app.document().cursor_position(), position_before);
    }

    #[test]
    fn busca_incremental_pula_pro_match_e_cancelar_restaura_o_cursor() {
        // "ola mundo ola de novo": origem na posição 4 (início de
        // "mundo"), pra que o match de "ola" (posição 10, a segunda
        // ocorrência) seja claramente diferente da origem.
        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        type_text(&mut app, "ola mundo ola de novo");
        app.handle_key(key(KeyCode::Char('g')));
        app.handle_key(key(KeyCode::Char('g'))); // início do buffer
        for _ in 0..4 {
            app.handle_key(key(KeyCode::Char(';'))); // move pra posição 4
        }
        let origin = app.document().cursor_offset();
        assert_eq!(origin, 4);

        app.handle_key(key(KeyCode::Char(' '))); // líder
        app.handle_key(key(KeyCode::Char('f'))); // SPC f: busca
        assert!(app.prompt.is_some());

        for c in "ola".chars() {
            app.handle_key(key(KeyCode::Char(c)));
        }
        // achou a segunda ocorrência de "ola", a partir da origem
        assert_eq!(app.document().cursor_offset(), 10);

        app.handle_key(key(KeyCode::Esc));
        assert!(app.prompt.is_none());
        assert_eq!(app.document().cursor_offset(), origin);
    }

    #[test]
    fn busca_confirmada_com_enter_deixa_o_cursor_no_match() {
        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        type_text(&mut app, "ola mundo");
        app.handle_key(key(KeyCode::Char('g')));
        app.handle_key(key(KeyCode::Char('g')));

        app.handle_key(key(KeyCode::Char(' ')));
        app.handle_key(key(KeyCode::Char('f')));
        for c in "mundo".chars() {
            app.handle_key(key(KeyCode::Char(c)));
        }
        app.handle_key(key(KeyCode::Enter));

        assert!(app.prompt.is_none());
        assert_eq!(app.document().cursor_offset(), 4); // "ola " tem 4 chars
    }

    #[test]
    fn durante_o_prompt_teclas_nunca_chegam_na_maquina_modal() {
        // 'j' durante o prompt deve digitar 'j' no prompt, nunca mover o
        // cursor do documento como comando visual.
        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        type_text(&mut app, "abc");
        app.handle_key(key(KeyCode::Char('g')));
        app.handle_key(key(KeyCode::Char('g')));
        let position_before = app.document().cursor_position();

        app.handle_key(ctrl_key(KeyCode::Char('o'))); // Ctrl+o: abre o prompt de arquivo
        app.handle_key(key(KeyCode::Char('j')));
        app.handle_key(key(KeyCode::Char('k')));

        assert_eq!(app.prompt.as_ref().unwrap().input, "jk");
        assert_eq!(app.document().cursor_position(), position_before);

        app.handle_key(key(KeyCode::Esc));
        assert!(app.prompt.is_none());
    }

    #[test]
    fn ctrl_o_abre_arquivo_com_alteracoes_nao_salvas_em_buffer_novo() {
        let dir = std::env::temp_dir().join(format!("nas-term-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("outro.txt");
        let _ = std::fs::remove_file(&path);

        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        type_text(&mut app, "abc");
        assert!(app.document().is_modified());

        app.handle_key(ctrl_key(KeyCode::Char('o')));
        for c in path.to_str().unwrap().chars() {
            app.handle_key(key(KeyCode::Char(c)));
        }
        app.handle_key(key(KeyCode::Enter));

        assert!(app.prompt.is_none());
        assert_eq!(app.buffers.len(), 2);
        assert_eq!(app.active, 1);
        assert_eq!(text(&app), "", "o buffer novo começa vazio");

        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn ctrl_o_com_arquivo_ja_aberto_troca_pro_buffer_em_vez_de_duplicar() {
        let dir = std::env::temp_dir().join(format!("nas-term-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("existente.txt");
        std::fs::write(&path, "conteudo").unwrap();

        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        let open = |app: &mut App| {
            app.handle_key(ctrl_key(KeyCode::Char('o')));
            for c in path.to_str().unwrap().chars() {
                app.handle_key(key(KeyCode::Char(c)));
            }
            app.handle_key(key(KeyCode::Enter));
        };
        open(&mut app);
        assert_eq!(app.buffers.len(), 2);
        let first_open_index = app.active;

        app.handle_key(key(KeyCode::Char('g')));
        app.handle_key(key(KeyCode::Char('g')));
        open(&mut app); // abre o mesmo caminho de novo

        assert_eq!(app.buffers.len(), 2, "não deveria duplicar o buffer");
        assert_eq!(app.active, first_open_index);

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn ctrl_b_troca_de_buffer_pelo_nome() {
        let dir = std::env::temp_dir().join(format!("nas-term-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("segundo.txt");
        let _ = std::fs::remove_file(&path);

        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        app.handle_key(ctrl_key(KeyCode::Char('o')));
        for c in path.to_str().unwrap().chars() {
            app.handle_key(key(KeyCode::Char(c)));
        }
        app.handle_key(key(KeyCode::Enter));
        assert_eq!(app.active, 1);

        app.handle_key(ctrl_key(KeyCode::Char('b')));
        assert_eq!(app.prompt.as_ref().unwrap().kind, PromptKind::SwitchBuffer);
        for c in "sem nome".chars() {
            app.handle_key(key(KeyCode::Char(c)));
        }
        app.handle_key(key(KeyCode::Enter));

        assert_eq!(app.active, 0, "deveria ter voltado pro buffer [sem nome]");

        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn ctrl_b_sem_match_mostra_mensagem_e_nao_troca() {
        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        app.handle_key(ctrl_key(KeyCode::Char('b')));
        for c in "nao existe".chars() {
            app.handle_key(key(KeyCode::Char(c)));
        }
        app.handle_key(key(KeyCode::Enter));

        assert_eq!(app.active, 0);
        assert!(app.status_message.is_some());
    }

    #[test]
    fn o_e_b_nao_sao_mais_comandos_do_visual() {
        // o/b saíram do Visual na reforma de teclas — agora só existem
        // como Ctrl+o/Ctrl+b (classe sistema).
        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        app.handle_key(key(KeyCode::Char('o')));
        assert!(app.prompt.is_none());
        app.handle_key(key(KeyCode::Char('b')));
        assert!(app.prompt.is_none());
    }

    #[test]
    fn ctrl_q_lista_os_buffers_com_alteracoes_nao_salvas() {
        let dir = std::env::temp_dir().join(format!("nas-term-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("terceiro.txt");
        let _ = std::fs::remove_file(&path);

        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        type_text(&mut app, "abc"); // modifica o buffer [sem nome]

        app.handle_key(ctrl_key(KeyCode::Char('o')));
        for c in path.to_str().unwrap().chars() {
            app.handle_key(key(KeyCode::Char(c)));
        }
        app.handle_key(key(KeyCode::Enter));
        type_text(&mut app, "def"); // modifica o segundo buffer também

        app.handle_key(ctrl_key(KeyCode::Char('q')));

        assert!(!app.should_quit);
        let message = app.status_message.as_ref().unwrap();
        assert!(message.contains("[sem nome]"));
        assert!(message.contains(path.to_str().unwrap()));

        app.handle_key(ctrl_key(KeyCode::Char('q'))); // confirma
        assert!(app.should_quit);

        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn ctrl_q_sem_alteracoes_sai_direto() {
        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        app.handle_key(ctrl_key(KeyCode::Char('q')));
        assert!(app.should_quit);
    }

    #[test]
    fn ctrl_k_fecha_buffer_sem_alteracoes_direto() {
        let dir = std::env::temp_dir().join(format!("nas-term-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("fechar.txt");
        let _ = std::fs::remove_file(&path);

        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        app.handle_key(ctrl_key(KeyCode::Char('o')));
        for c in path.to_str().unwrap().chars() {
            app.handle_key(key(KeyCode::Char(c)));
        }
        app.handle_key(key(KeyCode::Enter));
        assert_eq!(app.buffers.len(), 2);

        app.handle_key(ctrl_key(KeyCode::Char('k')));
        assert_eq!(app.buffers.len(), 1);
        assert_eq!(app.active, 0);

        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn ctrl_k_com_alteracoes_pede_confirmacao() {
        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        type_text(&mut app, "abc");

        app.handle_key(ctrl_key(KeyCode::Char('k')));
        assert_eq!(app.buffers.len(), 1, "não deveria ter fechado ainda");
        assert!(app.status_message.is_some());

        app.handle_key(ctrl_key(KeyCode::Char('k'))); // confirma
        assert_eq!(app.buffers.len(), 1);
        assert_eq!(
            text(&app),
            "",
            "fechar o único buffer deixa um vazio no lugar"
        );
    }

    #[test]
    fn ctrl_k_no_ultimo_buffer_deixa_um_vazio_no_lugar() {
        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        app.handle_key(ctrl_key(KeyCode::Char('k')));
        assert_eq!(app.buffers.len(), 1);
        assert_eq!(app.active, 0);
        assert_eq!(app.document().path(), None);
    }

    #[test]
    fn expande_til_para_o_diretorio_home() {
        let home = std::env::var_os("HOME").expect("precisa de HOME pra este teste");
        assert_eq!(
            expand_path("~/notas/arquivo.txt"),
            PathBuf::from(&home).join("notas/arquivo.txt")
        );
        assert_eq!(expand_path("~"), PathBuf::from(&home));
    }

    #[test]
    fn resolve_caminho_relativo_a_partir_do_diretorio_atual() {
        let cwd = std::env::current_dir().expect("diretório atual deveria existir");
        assert_eq!(expand_path("arquivo.txt"), cwd.join("arquivo.txt"));
        assert_eq!(
            expand_path("sub/dir/arquivo.txt"),
            cwd.join("sub/dir/arquivo.txt")
        );
    }

    #[test]
    fn caminho_absoluto_fica_inalterado() {
        assert_eq!(
            expand_path("/tmp/arquivo.txt"),
            PathBuf::from("/tmp/arquivo.txt")
        );
    }

    #[test]
    fn selecionar_copiar_e_colar() {
        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        type_text(&mut app, "ola mundo");
        app.handle_key(key(KeyCode::Char('g')));
        app.handle_key(key(KeyCode::Char('g')));
        app.handle_key(key(KeyCode::Char('s'))); // marca a seleção
        for _ in 0..3 {
            app.handle_key(key(KeyCode::Char(';')));
        }
        assert_eq!(app.document().selection_range(), Some((0, 3)));

        app.handle_key(key(KeyCode::Char('c'))); // copia (com seleção, age na hora)
        assert_eq!(app.clipboard, "ola");
        assert_eq!(app.pending_clipboard_write.take(), Some("ola".to_string()));
        assert_eq!(
            app.document().selection_range(),
            None,
            "copiar deveria descartar a seleção"
        );

        app.handle_key(key(KeyCode::Char('K'))); // fim do buffer
        app.handle_key(key(KeyCode::Char('y'))); // pede pra colar
        assert!(app.pending_paste);
        app.apply_pending_paste(); // main.rs faria isso depois de tentar o clipboard do sistema

        assert_eq!(text(&app), "ola mundoola");
    }

    #[test]
    fn recortar_remove_o_trecho_e_pode_ser_colado_de_volta() {
        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        type_text(&mut app, "ola mundo");
        app.handle_key(key(KeyCode::Char('g')));
        app.handle_key(key(KeyCode::Char('g')));
        app.handle_key(key(KeyCode::Char('s')));
        for _ in 0..4 {
            app.handle_key(key(KeyCode::Char(';'))); // seleciona "ola "
        }

        app.handle_key(key(KeyCode::Char('x'))); // recorta (com seleção)
        assert_eq!(text(&app), "mundo");
        assert_eq!(app.clipboard, "ola ");

        app.handle_key(key(KeyCode::Char('y'))); // pede pra colar de volta
        app.apply_pending_paste();
        assert_eq!(text(&app), "ola mundo");
    }

    #[test]
    fn apagar_com_selecao_nao_copia_pra_area_de_transferencia() {
        // Diferente de x (recortar), d com seleção só apaga — não é
        // kill-style (docs/keys.md §4.2: a célula "com seleção" de d
        // não menciona área de transferência).
        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        type_text(&mut app, "ola mundo");
        app.handle_key(key(KeyCode::Char('g')));
        app.handle_key(key(KeyCode::Char('g')));
        app.handle_key(key(KeyCode::Char('s')));
        for _ in 0..3 {
            app.handle_key(key(KeyCode::Char(';')));
        }

        app.handle_key(key(KeyCode::Char('d')));
        assert_eq!(text(&app), " mundo");
        assert_eq!(
            app.clipboard, "",
            "d com seleção não deveria tocar o clipboard"
        );
    }

    #[test]
    fn colar_com_selecao_ativa_substitui_a_selecao() {
        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        type_text(&mut app, "ola mundo");
        app.handle_key(key(KeyCode::Char('g')));
        app.handle_key(key(KeyCode::Char('g')));
        app.handle_key(key(KeyCode::Char('s')));
        for _ in 0..3 {
            app.handle_key(key(KeyCode::Char(';'))); // seleciona "ola"
        }

        app.clipboard = "oi".to_string();
        app.handle_key(key(KeyCode::Char('y')));
        app.apply_pending_paste();

        assert_eq!(text(&app), "oi mundo");
    }

    #[test]
    fn y_marca_pending_paste_sem_inserir_nada_sozinho() {
        // A inserção de verdade só acontece em apply_pending_paste,
        // depois que main.rs tenta atualizar o clipboard a partir do
        // sistema — y por si só não insere nada.
        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        app.clipboard = "preexistente".to_string();
        app.handle_key(key(KeyCode::Char('y')));

        assert!(app.pending_paste);
        assert_eq!(text(&app), "");
    }

    #[test]
    fn apply_pending_paste_cola_o_que_estiver_em_clipboard_no_momento() {
        // Simula main.rs: aperta y, "atualiza" clipboard a partir da
        // área de transferência do sistema (aqui, só trocando o campo
        // direto), e só então aplica.
        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        app.handle_key(key(KeyCode::Char('y')));
        assert!(app.pending_paste);

        app.clipboard = "do sistema".to_string();
        app.apply_pending_paste();

        assert_eq!(text(&app), "do sistema");
        assert!(!app.pending_paste);
    }

    #[test]
    fn apply_pending_paste_sem_pedido_pendente_nao_faz_nada() {
        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        app.clipboard = "nao deveria colar".to_string();
        app.apply_pending_paste();
        assert_eq!(text(&app), "");
    }

    #[test]
    fn select_all_seleciona_tudo_antes_de_copiar() {
        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        type_text(&mut app, "abc");
        app.handle_key(key(KeyCode::Char('S')));
        app.handle_key(key(KeyCode::Char('c')));
        assert_eq!(app.clipboard, "abc");
    }

    #[test]
    fn copiar_sem_selecao_espera_um_objeto() {
        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        type_text(&mut app, "abc");
        app.handle_key(key(KeyCode::Char('g')));
        app.handle_key(key(KeyCode::Char('g')));
        app.handle_key(key(KeyCode::Char('c'))); // sem seleção: vira verbo pendente
        assert_eq!(app.clipboard, "");
        app.handle_key(key(KeyCode::Char(';'))); // objeto: 1 caractere à frente
        assert_eq!(app.clipboard, "a");
    }

    #[test]
    fn bracketed_paste_e_sempre_texto_nunca_comando_do_visual() {
        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        type_text(&mut app, "abc");
        app.handle_key(key(KeyCode::Char('g')));
        app.handle_key(key(KeyCode::Char('g'))); // cursor no início, estado Visual

        // "dd" seria o verbo repetido se passasse pela máquina modal
        // (apagaria a linha inteira); colado, deve só inserir o texto.
        app.handle_paste("dd".to_string());

        assert_eq!(text(&app), "ddabc");
        assert_eq!(app.document().cursor_position().column, 2);
    }

    #[test]
    fn bracketed_paste_vai_pro_prompt_quando_ativo() {
        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        type_text(&mut app, "abc");
        app.handle_key(ctrl_key(KeyCode::Char('o'))); // abre o prompt de abrir arquivo

        app.handle_paste("/tmp/colado.txt".to_string());

        assert_eq!(app.prompt.as_ref().unwrap().input, "/tmp/colado.txt");
        assert_eq!(text(&app), "abc", "o documento não deve ter sido tocado");
    }

    #[test]
    fn verbo_mais_objeto_copia_pra_area_de_transferencia() {
        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        type_text(&mut app, "ola mundo");
        app.handle_key(key(KeyCode::Char('g')));
        app.handle_key(key(KeyCode::Char('g')));

        app.handle_key(key(KeyCode::Char('d')));
        app.handle_key(key(KeyCode::Char('w'))); // d w ; : apaga "ola"
        app.handle_key(key(KeyCode::Char(';')));
        assert_eq!(text(&app), " mundo");
        assert_eq!(app.clipboard, "ola");

        app.handle_key(key(KeyCode::Char('d')));
        app.handle_key(key(KeyCode::Char(':'))); // d : apaga até o fim da linha
        assert_eq!(text(&app), "");
        assert_eq!(app.clipboard, " mundo");
    }

    #[test]
    fn apagar_arquivo_inteiro_precisa_de_confirmacao_e_copia_pro_clipboard() {
        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        type_text(&mut app, "ola mundo");

        app.handle_key(key(KeyCode::Char('d')));
        app.handle_key(key(KeyCode::Char('a'))); // primeiro "d a": só avisa
        assert_eq!(text(&app), "ola mundo", "não deveria ter apagado ainda");
        assert!(app.status_message.is_some());

        app.handle_key(key(KeyCode::Char('d')));
        app.handle_key(key(KeyCode::Char('a'))); // segundo "d a": confirma
        assert_eq!(text(&app), "");
        assert_eq!(
            app.clipboard, "ola mundo",
            "d a deveria copiar o conteúdo antes de apagar, como qualquer outro objeto"
        );
    }

    #[test]
    fn apagar_arquivo_inteiro_e_cancelado_por_qualquer_outra_acao() {
        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        type_text(&mut app, "ola mundo");

        app.handle_key(key(KeyCode::Char('d')));
        app.handle_key(key(KeyCode::Char('a'))); // primeiro "d a"

        app.handle_key(key(KeyCode::Char('g')));
        app.handle_key(key(KeyCode::Char('g'))); // qualquer outra ação cancela

        app.handle_key(key(KeyCode::Char('d')));
        app.handle_key(key(KeyCode::Char('a'))); // "d a" de novo: é um primeiro pedido, não confirma
        assert_eq!(
            text(&app),
            "ola mundo",
            "a confirmação deveria ter sido cancelada pelo 'g g'"
        );
    }

    #[test]
    fn recortar_arquivo_inteiro_tambem_pede_confirmacao() {
        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        type_text(&mut app, "ola mundo");

        app.handle_key(key(KeyCode::Char('x')));
        app.handle_key(key(KeyCode::Char('a')));
        assert_eq!(text(&app), "ola mundo");

        app.handle_key(key(KeyCode::Char('x')));
        app.handle_key(key(KeyCode::Char('a')));
        assert_eq!(text(&app), "");
        assert_eq!(app.clipboard, "ola mundo");
    }

    #[test]
    fn copiar_arquivo_inteiro_nao_precisa_de_confirmacao() {
        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        type_text(&mut app, "ola mundo");

        app.handle_key(key(KeyCode::Char('c')));
        app.handle_key(key(KeyCode::Char('a')));
        assert_eq!(text(&app), "ola mundo", "copiar nunca apaga nada");
        assert_eq!(app.clipboard, "ola mundo");
    }

    #[test]
    fn goto_line_vai_para_a_linha_pedida() {
        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        type_text(&mut app, "um\ndois\ntres");

        app.handle_key(key(KeyCode::Char('g')));
        app.handle_key(key(KeyCode::Char('l')));
        assert_eq!(app.prompt.as_ref().unwrap().kind, PromptKind::GotoLine);
        for c in "2".chars() {
            app.handle_key(key(KeyCode::Char(c)));
        }
        app.handle_key(key(KeyCode::Enter));

        assert_eq!(app.document().cursor_position().line, 1);
    }

    #[test]
    fn unidade_sozinha_move_por_palavra_e_paragrafo() {
        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        type_text(&mut app, "ola mundo\n\ndois");
        app.handle_key(key(KeyCode::Char('g')));
        app.handle_key(key(KeyCode::Char('g')));

        app.handle_key(key(KeyCode::Char('w')));
        app.handle_key(key(KeyCode::Char(';'))); // w ; : próxima palavra
        assert_eq!(app.document().cursor_offset(), 3);

        app.handle_key(key(KeyCode::Char('p')));
        app.handle_key(key(KeyCode::Char('k'))); // p k: próximo parágrafo
        assert_eq!(app.document().cursor_position().line, 2);

        app.handle_key(key(KeyCode::Char('p')));
        app.handle_key(key(KeyCode::Char('l'))); // p l: parágrafo anterior
        assert_eq!(app.document().cursor_position().line, 0);
    }

    #[test]
    fn repetir_o_ultimo_composto_na_posicao_atual_do_cursor() {
        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        type_text(&mut app, "ola mundo bom");
        app.handle_key(key(KeyCode::Char('g')));
        app.handle_key(key(KeyCode::Char('g')));

        app.handle_key(key(KeyCode::Char('d')));
        app.handle_key(key(KeyCode::Char('w')));
        app.handle_key(key(KeyCode::Char(';'))); // d w ; : apaga "ola"
        assert_eq!(text(&app), " mundo bom");

        // o cursor agora está no espaço antes de "mundo" — repetir "d w
        // ;" dali apaga esse espaço e "mundo" (o objeto é recalculado
        // na posição atual, não replica literalmente o que foi apagado
        // antes).
        app.handle_key(key(KeyCode::Char('.'))); // repete na posição atual
        assert_eq!(text(&app), " bom");
    }
}
