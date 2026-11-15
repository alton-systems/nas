//! Máquina de estados modal (Insert, Visual, Sequential), seguindo a
//! semântica do nas-keys (https://github.com/2gab/nas-keys). Separado da
//! interface de terminal — testável sem abrir um terminal de verdade.
//!
//! Os atalhos vêm de `keymap/default.toml` (ver [`keymap`]), nunca
//! espalhados pelo código Rust — ver
//! docs/decisions/0003-nas-keys-como-keymap.md. A tradução de layout
//! físico de teclado (qwerty → qwerty-abnt) mora em [`layout`].

mod key;
mod keymap;
mod layout;
mod machine;

pub use key::{Key, KeyCode};
pub use keymap::{Binding, Dispatch, Keymap, KeymapError};
pub use layout::Layout;
pub use machine::{Action, Machine, ModalState};
