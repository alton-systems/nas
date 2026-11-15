//! Estado da aplicação: liga o teclado (crossterm) à máquina de estados
//! do nas-keys e suas ações ao documento. Atalhos com Ctrl (salvar,
//! sair) não fazem parte do vocabulário do nas-keys — são tratados aqui,
//! antes de chegar à máquina de estados.

use nas_editor::Document;
use nas_keys::{Action, Key, KeyCode as NasKeyCode, Layout, Machine};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

pub struct App {
    pub document: Document,
    pub modal: Machine,
    pub scroll_top: usize,
    pub scroll_left: usize,
    pub should_quit: bool,
    pub quit_confirm_pending: bool,
    pub status_message: Option<String>,
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
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
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

    #[test]
    fn despacho_de_apagar_e_desfazer_com_u_pelo_estado_visual() {
        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        app.handle_key(key(KeyCode::Enter)); // Visual -> Insert
        for c in "abc".chars() {
            app.handle_key(key(KeyCode::Char(c)));
        }
        app.handle_key(alt_key(KeyCode::Char(' '))); // Insert -> Visual
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
        app.handle_key(key(KeyCode::Enter));
        for c in "abc".chars() {
            app.handle_key(key(KeyCode::Char(c)));
        }
        app.handle_key(alt_key(KeyCode::Char(' ')));
        let position_before = app.document.cursor_position();

        app.handle_key(ctrl_key(KeyCode::Char('j')));

        assert_eq!(app.document.cursor_position(), position_before);
    }
}
