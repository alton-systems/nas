//! Histórico com autoria: quem mudou o quê, quando, e por qual
//! comando — docs/decisions/0005-historico-com-autoria.md.
//!
//! Não depende de `nas-editor` nem de `nas-keys` — só de tipos
//! simples (`String`, `PathBuf`, timestamps, intervalos de
//! caracteres). `nas-term` é quem chama esta crate, do mesmo jeito
//! que já chama as outras duas (ADR 0005 §9).
//!
//! ## Escopo desta etapa (2b)
//!
//! Autoria (`Author`) e o journal vêm da etapa 2a, já ligados ao
//! `nas-editor`/`nas-term`. Esta etapa acrescenta checkpoints do
//! projeto: armazenamento por conteúdo (`objects/`, deduplicado),
//! limite de tamanho por arquivo com aviso dos excluídos, poda (por
//! quantidade e por tamanho total, nunca removendo um nomeado) e
//! apagar o histórico do projeto, com confirmação. Diff e restauração
//! (e as teclas `Ctrl+t`) ainda não existem — vêm na etapa 2c.

mod author;
mod checkpoint;
mod journal;
mod project;

pub use author::Author;
pub use checkpoint::{
    CheckpointFile, CheckpointManifest, CreatedCheckpoint, ExcludedFile, ExcludedReason,
    MAX_AUTOMATIC_CHECKPOINTS, MAX_FILE_SIZE, MAX_TOTAL_SIZE_BYTES, PruneReport,
    changed_files_since_last_checkpoint, create as create_checkpoint, list as list_checkpoints,
    prune, prune_with_limits,
};
pub use journal::{Journal, JournalEntry, journal_path, now_unix, read_entries};
pub use project::{ProjectId, delete_project_history, state_home};
