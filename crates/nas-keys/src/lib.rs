//! Máquina de estados modal (Insert, Visual, Sequential), seguindo a
//! semântica do nas-keys (https://github.com/2gab/nas-keys). Separado da
//! interface de terminal — testável sem abrir um terminal de verdade.
//!
//! A tradução de layout físico de teclado (qwerty → qwerty-abnt) é escopo
//! da etapa 4, não desta.

mod key;
mod machine;

pub use key::{Key, KeyCode};
pub use machine::{Action, Machine, ModalState};
