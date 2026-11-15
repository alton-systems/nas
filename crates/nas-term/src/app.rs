//! Estado da aplicação e tratamento de teclas. Ainda sem modal (nas-keys
//! entra na etapa 3) — por ora, atalhos fixos ao estilo dos editores comuns.

use nas_editor::Document;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

pub struct App {
    pub document: Document,
    pub scroll_top: usize,
    pub scroll_left: usize,
    pub should_quit: bool,
    pub quit_confirm_pending: bool,
    pub status_message: Option<String>,
}

impl App {
    pub fn new(document: Document) -> Self {
        Self {
            document,
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

        match key.code {
            KeyCode::Left => self.document.move_left(),
            KeyCode::Right => self.document.move_right(),
            KeyCode::Up => self.document.move_up(),
            KeyCode::Down => self.document.move_down(),
            KeyCode::Backspace => self.document.delete_backward(),
            KeyCode::Delete => self.document.delete_forward(),
            KeyCode::Enter => self.document.insert_newline(),
            KeyCode::Tab => self.document.insert_char('\t'),
            KeyCode::Char('s') if ctrl => self.save(),
            KeyCode::Char('z') if ctrl => self.undo(),
            KeyCode::Char(c) if !ctrl && !key.modifiers.contains(KeyModifiers::ALT) => {
                self.document.insert_char(c);
            }
            _ => {}
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
        self.quit_confirm_pending = false;
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
