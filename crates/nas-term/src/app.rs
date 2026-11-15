//! Estado da aplicação: liga o teclado (crossterm) à máquina de estados
//! do nas-keys e suas ações ao documento. Atalhos com Ctrl (salvar,
//! sair) não fazem parte do vocabulário do nas-keys — são tratados aqui,
//! antes de chegar à máquina de estados.

use std::path::{Path, PathBuf};

use nas_editor::{Document, OpenError};
use nas_keys::{Action, Key, KeyCode as NasKeyCode, Layout, Machine};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// O que uma linha de entrada está pedindo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptKind {
    OpenFile,
    Search,
}

/// Uma linha de entrada ativa na barra de status — equivalente ao
/// minibuffer do Emacs que `o` (find-file) e `SPC f` (isearch-forward)
/// abrem no nas-keys real. Enquanto ativa, nenhuma tecla chega à máquina
/// modal: o texto digitado é só o conteúdo do prompt, nunca interpretado
/// como comando do estado Visual.
pub struct Prompt {
    pub kind: PromptKind,
    pub input: String,
    /// Posição do cursor antes da busca começar, pra restaurar se o
    /// usuário cancelar. `None` fora de uma busca (ex.: abrir arquivo).
    search_origin: Option<usize>,
}

pub struct App {
    pub document: Document,
    pub modal: Machine,
    pub scroll_top: usize,
    pub scroll_left: usize,
    pub should_quit: bool,
    pub quit_confirm_pending: bool,
    pub status_message: Option<String>,
    pub prompt: Option<Prompt>,
}

impl App {
    /// Traduz teclas do layout físico de teclado dado antes de repassá-las
    /// à máquina modal (ver `NAS_KEYBOARD_LAYOUT` em `main.rs`).
    pub fn with_layout(document: Document, layout: Layout) -> Self {
        Self {
            document,
            modal: Machine::with_layout(layout),
            scroll_top: 0,
            scroll_left: 0,
            should_quit: false,
            quit_confirm_pending: false,
            status_message: None,
            prompt: None,
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        if self.prompt.is_some() {
            self.handle_prompt_key(key);
            return;
        }

        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

        if ctrl && key.code == KeyCode::Char('q') {
            self.handle_quit_request();
            return;
        }

        self.quit_confirm_pending = false;
        self.status_message = None;

        if ctrl {
            if key.code == KeyCode::Char('s') {
                self.save();
            }
            // Outros atalhos com Ctrl não existem ainda; ignorados, nunca
            // reinterpretados como a tecla sem o Ctrl (nas-keys não tem
            // vocabulário de Ctrl, então não cai em nenhuma ação dele).
            return;
        }

        let Some(nas_key) = translate_key(key) else {
            return;
        };

        for action in self.modal.handle_key(nas_key) {
            self.apply_action(action);
        }
    }

    fn apply_action(&mut self, action: Action) {
        match action {
            Action::MoveLeft => self.document.move_left(),
            Action::MoveRight => self.document.move_right(),
            Action::MoveUp => self.document.move_up(),
            Action::MoveDown => self.document.move_down(),
            Action::MoveLineStart => self.document.move_to_line_start(),
            Action::MoveLineEnd => self.document.move_to_line_end(),
            Action::MoveBufferStart => self.document.move_to_buffer_start(),
            Action::MoveBufferEnd => self.document.move_to_buffer_end(),
            Action::DeleteForward => self.document.delete_forward(),
            Action::DeleteBackward => self.document.delete_backward(),
            Action::Undo => self.undo(),
            Action::InsertChar(c) => self.document.insert_char(c),
            Action::InsertNewline => self.document.insert_newline(),
            Action::PromptOpenFile => {
                self.prompt = Some(Prompt {
                    kind: PromptKind::OpenFile,
                    input: String::new(),
                    search_origin: None,
                });
            }
            Action::PromptSearch => {
                self.prompt = Some(Prompt {
                    kind: PromptKind::Search,
                    input: String::new(),
                    search_origin: Some(self.document.cursor_offset()),
                });
            }
        }
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
        if let Some(pos) = self.document.find_from(origin, &prompt.input) {
            self.document.set_cursor(pos);
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
        }
    }

    fn cancel_prompt(&mut self) {
        let Some(prompt) = self.prompt.take() else {
            return;
        };
        if let Some(origin) = prompt.search_origin {
            self.document.set_cursor(origin);
        }
    }

    fn open_file(&mut self, path: &str) {
        let path = path.trim();
        if path.is_empty() {
            self.status_message = Some("Nenhum arquivo informado.".to_string());
            return;
        }
        if self.document.is_modified() {
            self.status_message = Some(
                "Há alterações não salvas; salve (Ctrl+S) antes de abrir outro arquivo."
                    .to_string(),
            );
            return;
        }
        match Document::open(expand_path(path)) {
            Ok(document) => {
                self.document = document;
                self.scroll_top = 0;
                self.scroll_left = 0;
            }
            Err(OpenError::NotUtf8) => {
                self.status_message = Some(format!("'{path}' não é UTF-8 válido."));
            }
            Err(err) => {
                self.status_message = Some(format!("Erro ao abrir '{path}': {err}"));
            }
        }
    }

    /// Ajusta `scroll_top`/`scroll_left` para que o cursor fique visível
    /// numa área de `width` x `height` células.
    pub fn ensure_cursor_visible(&mut self, width: usize, height: usize) {
        let position = self.document.cursor_position();
        if position.line < self.scroll_top {
            self.scroll_top = position.line;
        } else if position.line >= self.scroll_top + height {
            self.scroll_top = position.line + 1 - height;
        }

        let visual_column = self.document.cursor_visual_column();
        if visual_column < self.scroll_left {
            self.scroll_left = visual_column;
        } else if width > 0 && visual_column >= self.scroll_left + width {
            self.scroll_left = visual_column + 1 - width;
        }
    }

    fn handle_quit_request(&mut self) {
        if !self.document.is_modified() || self.quit_confirm_pending {
            self.should_quit = true;
            return;
        }
        self.quit_confirm_pending = true;
        self.status_message = Some(
            "Alterações não salvas — Ctrl+Q de novo para saír sem salvar, Ctrl+S para salvar"
                .to_string(),
        );
    }

    fn save(&mut self) {
        self.status_message = Some(match self.document.save() {
            Ok(true) => "Salvo.".to_string(),
            Ok(false) => "Sem nome de arquivo; nada para salvar.".to_string(),
            Err(err) => format!("Erro ao salvar: {err}"),
        });
    }

    fn undo(&mut self) {
        if !self.document.undo() {
            self.status_message = Some("Nada para desfazer.".to_string());
        }
    }
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
        (0..app.document.len_lines())
            .map(|i| app.document.line(i).into_owned())
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
    fn despacho_de_apagar_e_desfazer_com_u_pelo_estado_visual() {
        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        type_text(&mut app, "abc");
        app.handle_key(key(KeyCode::Char('g')));
        app.handle_key(key(KeyCode::Char('g'))); // início do buffer
        assert_eq!(text(&app), "abc");

        app.handle_key(key(KeyCode::Char('d')));
        app.handle_key(key(KeyCode::Char('d'))); // d d: apaga o 'a'
        assert_eq!(text(&app), "bc");

        app.handle_key(key(KeyCode::Char('u'))); // u: desfaz
        assert_eq!(text(&app), "abc");
    }

    #[test]
    fn ctrl_mais_tecla_nao_mapeada_nao_e_reinterpretado_sem_o_ctrl() {
        // Ctrl+j não é parte do vocabulário do nas-keys; não deve mover o
        // cursor como se fosse só 'j' sem o Ctrl.
        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        type_text(&mut app, "abc");
        let position_before = app.document.cursor_position();

        app.handle_key(ctrl_key(KeyCode::Char('j')));

        assert_eq!(app.document.cursor_position(), position_before);
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
        let origin = app.document.cursor_offset();
        assert_eq!(origin, 4);

        app.handle_key(key(KeyCode::Char(' '))); // líder
        app.handle_key(key(KeyCode::Char('f'))); // SPC f: busca
        assert!(app.prompt.is_some());

        for c in "ola".chars() {
            app.handle_key(key(KeyCode::Char(c)));
        }
        // achou a segunda ocorrência de "ola", a partir da origem
        assert_eq!(app.document.cursor_offset(), 10);

        app.handle_key(key(KeyCode::Esc));
        assert!(app.prompt.is_none());
        assert_eq!(app.document.cursor_offset(), origin);
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
        assert_eq!(app.document.cursor_offset(), 4); // "ola " tem 4 chars
    }

    #[test]
    fn durante_o_prompt_teclas_nunca_chegam_na_maquina_modal() {
        // 'j' durante o prompt deve digitar 'j' no prompt, nunca mover o
        // cursor do documento como comando visual.
        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        type_text(&mut app, "abc");
        app.handle_key(key(KeyCode::Char('g')));
        app.handle_key(key(KeyCode::Char('g')));
        let position_before = app.document.cursor_position();

        app.handle_key(key(KeyCode::Char('o'))); // abre o prompt de arquivo
        app.handle_key(key(KeyCode::Char('j')));
        app.handle_key(key(KeyCode::Char('k')));

        assert_eq!(app.prompt.as_ref().unwrap().input, "jk");
        assert_eq!(app.document.cursor_position(), position_before);

        app.handle_key(key(KeyCode::Esc));
        assert!(app.prompt.is_none());
    }

    #[test]
    fn abrir_arquivo_recusa_se_houver_alteracoes_nao_salvas() {
        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        type_text(&mut app, "abc");
        assert!(app.document.is_modified());

        app.handle_key(key(KeyCode::Char('o')));
        for c in "/tmp/qualquer-coisa.txt".chars() {
            app.handle_key(key(KeyCode::Char(c)));
        }
        app.handle_key(key(KeyCode::Enter));

        assert!(app.prompt.is_none());
        assert_eq!(
            text(&app),
            "abc",
            "o documento modificado não deve ter sido substituído"
        );
        assert!(app.status_message.is_some());
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
}
