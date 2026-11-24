//! Máquina de estados modal (Insert, Visual, Sequential) e a gramática
//! de radicais e compostos (verbo + objeto, unidades) do nas. A
//! especificação completa das teclas vive em `docs/keys.md` — ver
//! docs/decisions/0004-especificacao-de-teclas-no-nas.md. Separado da
//! interface de terminal — testável sem abrir um terminal de verdade.
//!
//! Os atalhos vêm de `keymap/default.toml` (ver [`keymap`]), nunca
//! espalhados pelo código Rust. A tradução de layout físico de
//! teclado (qwerty → qwerty-abnt) mora em [`layout`].

mod key;
mod keymap;
mod layout;
mod machine;

pub use key::{Key, KeyCode};
pub use keymap::{Binding, DEFAULT_TOML, Keymap, KeymapError};
pub use layout::Layout;
pub use machine::{
    Action, CatalogEntry, Machine, ModalState, Object, PendingHint, SystemAction, Unit, Verb,
};
