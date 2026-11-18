//! Histórico com autoria: quem mudou o quê, quando, e por qual
//! comando — docs/decisions/0005-historico-com-autoria.md.
//!
//! Não depende de `nas-editor` nem de `nas-keys` — só de tipos
//! simples (`String`, `PathBuf`, timestamps, intervalos de
//! caracteres). `nas-term` é quem chama esta crate, do mesmo jeito
//! que já chama as outras duas (ADR 0005 §9).
//!
//! ## Escopo desta etapa (2a)
//!
//! Autoria (`Author`), o journal (agrupamento de digitação contínua e
//! gravação em lotes, ADR §2), persistência e permissão `0700`
//! (`ProjectId`, ADR §5/§6). Checkpoints, armazenamento por hash,
//! poda, diff e restauração ainda não existem — vêm nas etapas 2b e
//! 2c. Nada daqui está ligado a `nas-term` ainda.

mod author;
mod journal;
mod project;

pub use author::Author;
pub use journal::{Journal, JournalEntry, journal_path, now_unix, read_entries};
pub use project::{ProjectId, state_home};
