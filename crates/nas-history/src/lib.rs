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
//!
//! **Correção** (restaurar precisa voltar o projeto inteiro, não só
//! os arquivos de um checkpoint): `originals` guarda o conteúdo
//! original de cada arquivo rastreado, capturado na primeira edição
//! — por isso `checkpoint::create` passou a registrar todo arquivo
//! rastreado, nunca só os que mudaram desde o checkpoint anterior.
//! `trash` move pra `~/.local/share/Trash` (nunca apaga de vez) um
//! arquivo que um checkpoint sendo restaurado não conhece.

mod author;
mod checkpoint;
mod journal;
mod originals;
mod project;
mod restore;
mod trash;

pub use author::Author;
pub use checkpoint::{
    CheckpointFile, CheckpointManifest, CreatedCheckpoint, ExcludedFile, ExcludedReason,
    FileChange, FileDiff, MAX_AUTOMATIC_CHECKPOINTS, MAX_FILE_SIZE, MAX_TOTAL_SIZE_BYTES,
    PruneReport, create as create_checkpoint, diff, list as list_checkpoints, prune,
    prune_with_limits, read_object,
};
pub use journal::{Journal, JournalEntry, file_history, journal_path, now_unix, read_entries};
pub use originals::{OriginalEntry, ensure_tracked, get as get_tracked, list as list_tracked};
pub use project::{ProjectId, delete_project_history, state_home};
pub use restore::{RestoreReport, RestoreTarget, restore};
pub use trash::move_to_trash;
