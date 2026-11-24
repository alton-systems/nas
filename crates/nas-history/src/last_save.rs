//! O hash do conteúdo da última vez que alguém salvou um arquivo no
//! disco — issue #28. Sem isto, uma escrita do `nas --mcp`
//! (`save_file`/`create_file`/restaurar um checkpoint) aparece de
//! novo no journal como `Author::External` quando um `nas-term`
//! aberto no mesmo arquivo a detecta (docs/decisions/0005 §1),
//! duplicando a autoria de uma única edição. `nas-term` consulta
//! [`get_last_save`] antes de journalar como `External`: bate o
//! hash? então já é uma mudança conhecida, de outro autor — não é
//! "externa" de verdade.
//!
//! Ao contrário de `originals` (que só captura na primeira vez),
//! este registro é **sobrescrito a cada gravação** — é sempre "a
//! última", nunca "a primeira".

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::author::Author;
use crate::checkpoint::hash_content;

/// O que se sabia sobre a última gravação de um arquivo — o hash do
/// conteúdo gravado e quem gravou.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LastSave {
    pub hash: String,
    pub author: Author,
}

fn last_saves_dir(history_dir: &Path) -> PathBuf {
    history_dir.join("last_saves")
}

/// Mesmo esquema de `crate::originals::entry_path`: o nome do
/// registro é um hash do caminho, não o caminho em si.
fn entry_path(history_dir: &Path, file: &str) -> PathBuf {
    let mut hasher = Sha256::new();
    hasher.update(file.as_bytes());
    let key: String = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    last_saves_dir(history_dir).join(format!("{key}.toml"))
}

/// Registra o hash de `content` como a última gravação conhecida de
/// `file`, por `author` — sobrescreve o que já houvesse. Travado como
/// todo escritor do histórico (ADR 0006, revisão da etapa 3c).
pub fn record_last_save(
    history_dir: &Path,
    file: &str,
    content: &[u8],
    author: Author,
) -> io::Result<()> {
    let _lock = crate::lock::HistoryLock::acquire(history_dir)?;
    fs::create_dir_all(last_saves_dir(history_dir))?;
    let entry = LastSave {
        hash: hash_content(content),
        author,
    };
    let toml = toml::to_string_pretty(&entry)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    crate::atomic::write_atomic(&entry_path(history_dir, file), toml.as_bytes())
}

/// A última gravação conhecida de `file` — `None` se nunca foi
/// gravado por aqui (ADR 0006 §6/§5: `edit` sozinho, sem
/// `save_file`, nunca grava disco, então nunca passa por aqui).
pub fn get_last_save(history_dir: &Path, file: &str) -> io::Result<Option<LastSave>> {
    match fs::read_to_string(entry_path(history_dir, file)) {
        Ok(content) => {
            let entry = toml::from_str(&content)
                .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
            Ok(Some(entry))
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sem_nenhuma_gravacao_e_none() {
        let dir = tempfile::tempdir().unwrap();
        assert!(
            get_last_save(dir.path(), "nunca-salvo.txt")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn grava_e_le_de_volta() {
        let dir = tempfile::tempdir().unwrap();
        let author = Author::Ai {
            agent: "claude-code".to_string(),
            session: "s1".to_string(),
        };
        record_last_save(dir.path(), "a.txt", b"conteudo", author.clone()).unwrap();

        let last = get_last_save(dir.path(), "a.txt").unwrap().unwrap();
        assert_eq!(last.hash, hash_content(b"conteudo"));
        assert_eq!(last.author, author);
    }

    #[test]
    fn segunda_gravacao_sobrescreve_a_primeira() {
        let dir = tempfile::tempdir().unwrap();
        record_last_save(dir.path(), "a.txt", b"v1", Author::User).unwrap();
        record_last_save(
            dir.path(),
            "a.txt",
            b"v2",
            Author::Ai {
                agent: "claude-code".to_string(),
                session: "s1".to_string(),
            },
        )
        .unwrap();

        let last = get_last_save(dir.path(), "a.txt").unwrap().unwrap();
        assert_eq!(last.hash, hash_content(b"v2"));
        assert_eq!(
            last.author,
            Author::Ai {
                agent: "claude-code".to_string(),
                session: "s1".to_string()
            }
        );
    }
}
