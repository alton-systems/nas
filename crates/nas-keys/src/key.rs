//! Uma tecla neutra, independente de terminal — nas-keys não depende de
//! crossterm nem de nenhuma outra coisa de interface, para ser testável
//! sem abrir um terminal de verdade.
//!
//! Teclas com Ctrl (Ctrl+S, Ctrl+Q, ...) não fazem parte do vocabulário do
//! nas-keys — o nas-map.el real não usa `C-` em lugar nenhum, só `M-` (em
//! `M-SPC` e nos atalhos de janela). Atalhos com Ctrl são decisão do nas
//! (nas-term), tratados antes de chegar à máquina de estados.

use std::fmt;

/// Uma tecla, com o modificador Alt (`M-` no Emacs) marcado separadamente.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Key {
    pub code: KeyCode,
    pub alt: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyCode {
    Char(char),
    Enter,
    Escape,
    Backspace,
    Delete,
    Tab,
    Left,
    Right,
    Up,
    Down,
}

impl Key {
    pub const fn new(code: KeyCode) -> Self {
        Key { code, alt: false }
    }

    pub const fn with_alt(code: KeyCode) -> Self {
        Key { code, alt: true }
    }

    pub const fn char(c: char) -> Self {
        Key::new(KeyCode::Char(c))
    }

    pub const fn alt_char(c: char) -> Self {
        Key::with_alt(KeyCode::Char(c))
    }

    pub const fn enter() -> Self {
        Key::new(KeyCode::Enter)
    }

    pub const fn escape() -> Self {
        Key::new(KeyCode::Escape)
    }

    pub const fn backspace() -> Self {
        Key::new(KeyCode::Backspace)
    }
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.alt {
            write!(f, "M-")?;
        }
        match self.code {
            KeyCode::Char(' ') => write!(f, "SPC"),
            KeyCode::Char(c) => write!(f, "{c}"),
            KeyCode::Enter => write!(f, "RET"),
            KeyCode::Escape => write!(f, "ESC"),
            KeyCode::Backspace => write!(f, "DEL"),
            KeyCode::Delete => write!(f, "<delete>"),
            KeyCode::Tab => write!(f, "TAB"),
            KeyCode::Left => write!(f, "<left>"),
            KeyCode::Right => write!(f, "<right>"),
            KeyCode::Up => write!(f, "<up>"),
            KeyCode::Down => write!(f, "<down>"),
        }
    }
}
