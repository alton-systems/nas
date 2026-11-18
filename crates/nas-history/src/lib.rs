//! Histórico com autoria: quem mudou o quê, quando, e por qual
//! comando — docs/decisions/0005-historico-com-autoria.md.
//!
//! Não depende de `nas-editor` nem de `nas-keys` — só de tipos
//! simples (`String`, `PathBuf`, timestamps, intervalos de
//! caracteres). `nas-term` é quem chama esta crate, do mesmo jeito
//! que já chama as outras duas (ADR 0005 §9).
//!
//! ## Escopo desta etapa (2c)
//!
//! Autoria, journal (2a) e checkpoints com poda (2b) já existem.
//! Esta etapa acrescenta diff entre checkpoints (via a crate
//! `similar`) e uma listagem de autoria por arquivo
//! (`file_history`) — ligados ao `nas-term` pelas teclas `Ctrl+t`
//! (prefixo "tempo") e pela restauração de um checkpoint, via
//! `nas_editor::Document::replace_all`. Detecção de mudança externa
//! (`Author::External`) fica pra etapa 2d.

mod author;
mod checkpoint;
mod journal;
mod project;

pub use author::Author;
pub use checkpoint::{
    CheckpointFile, CheckpointManifest, CreatedCheckpoint, ExcludedFile, ExcludedReason,
    FileChange, FileDiff, MAX_AUTOMATIC_CHECKPOINTS, MAX_FILE_SIZE, MAX_TOTAL_SIZE_BYTES,
    PruneReport, changed_files_since_last_checkpoint, create as create_checkpoint, diff,
    list as list_checkpoints, prune, prune_with_limits, read_object,
};
pub use journal::{Journal, JournalEntry, file_history, journal_path, now_unix, read_entries};
pub use project::{ProjectId, delete_project_history, state_home};
