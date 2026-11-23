//! O conteúdo original de cada arquivo rastreado — capturado uma
//! única vez, na primeira edição que o journal registra para aquele
//! arquivo, direto do disco (antes de qualquer transação nossa tocar
//! nele). É a base pra restaurar corretamente um checkpoint: um
//! checkpoint só registra os arquivos que já eram rastreados quando
//! foi criado (ver `crate::checkpoint`), então um arquivo tocado por
//! primeira vez *depois* dele não aparece no manifesto — sem isto,
//! restaurar esse checkpoint não saberia pra onde esse arquivo
//! deveria voltar. docs/decisions/0005-historico-com-autoria.md §4
//! (correção: restaurar o projeto inteiro, não só os arquivos do
//! checkpoint).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::checkpoint::{MAX_FILE_SIZE, store_object};

/// O que se sabia sobre `path` no instante em que foi rastreado por
/// primeira vez.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OriginalEntry {
    pub path: String,
    /// `false` quando o arquivo não existia no disco ainda — foi
    /// criado depois de começar a ser editado pelo nas. Restaurar um
    /// checkpoint anterior a essa criação joga o arquivo na lixeira
    /// (nunca apaga de vez) em vez de tentar reverter um conteúdo que
    /// nunca existiu.
    pub existed: bool,
    /// O hash do conteúdo original em `objects/` — `None` se o
    /// arquivo não existia (`existed == false`), ou se existia mas
    /// era grande demais/não-UTF8 (mesmo critério dos checkpoints).
    pub content_hash: Option<String>,
}

fn originals_dir(history_dir: &Path) -> PathBuf {
    history_dir.join("originals")
}

/// O nome do arquivo de registro de `file` em `originals/` — um hash
/// do próprio caminho, não o caminho em si (que pode ter `/` e outros
/// caracteres que um nome de arquivo não aceita direto).
fn entry_path(history_dir: &Path, file: &str) -> PathBuf {
    let mut hasher = Sha256::new();
    hasher.update(file.as_bytes());
    let key: String = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    originals_dir(history_dir).join(format!("{key}.toml"))
}

/// Garante que `file` esteja rastreado — se já estiver (uma chamada
/// anterior, nesta sessão ou numa anterior, já capturou), não faz
/// nada: a captura é só na primeira vez de verdade, nunca
/// sobrescrevendo um original já guardado. `disk_content` é o
/// conteúdo atual em disco (`None` se o arquivo não existe) — quem
/// chama precisa garantir que isto reflete o estado **antes** de
/// qualquer edição desta transação tocar o disco (nenhuma ação normal
/// de editar escreve em disco, só salvar/restaurar — então chamar
/// isto antes de journalar uma edição comum já é seguro).
pub fn ensure_tracked(
    history_dir: &Path,
    file: &str,
    disk_content: Option<&[u8]>,
) -> io::Result<()> {
    // Travada: "ainda não rastreado? então capture" precisa ser
    // atômico entre processos (ADR 0006, revisão da etapa 3c) — sem
    // a trava, `nas-term` e `nas --mcp` editando o mesmo arquivo por
    // primeira vez quase ao mesmo tempo poderiam capturar dois
    // "originais" diferentes, cada um vencendo a corrida do outro.
    let _lock = crate::lock::HistoryLock::acquire(history_dir)?;

    let path = entry_path(history_dir, file);
    if path.exists() {
        return Ok(());
    }
    fs::create_dir_all(originals_dir(history_dir))?;

    let entry = match disk_content {
        None => OriginalEntry {
            path: file.to_string(),
            existed: false,
            content_hash: None,
        },
        Some(content) if content.len() > MAX_FILE_SIZE || std::str::from_utf8(content).is_err() => {
            OriginalEntry {
                path: file.to_string(),
                existed: true,
                content_hash: None,
            }
        }
        Some(content) => OriginalEntry {
            path: file.to_string(),
            existed: true,
            content_hash: Some(store_object(history_dir, content)?),
        },
    };

    let toml = toml::to_string_pretty(&entry)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    crate::atomic::write_atomic(&path, toml.as_bytes())
}

/// O registro original de `file`, se já estiver rastreado.
pub fn get(history_dir: &Path, file: &str) -> io::Result<Option<OriginalEntry>> {
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

/// Todo arquivo já rastreado (teve ao menos uma edição, nesta sessão
/// ou numa anterior) — a lista completa de arquivos que um checkpoint
/// novo deveria registrar.
pub fn list(history_dir: &Path) -> io::Result<Vec<OriginalEntry>> {
    let dir = originals_dir(history_dir);
    let entries = match fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err),
    };
    let mut out = Vec::new();
    for entry in entries {
        let content = fs::read_to_string(entry?.path())?;
        let parsed = toml::from_str(&content)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        out.push(parsed);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primeira_chamada_grava_o_conteudo_existente() {
        let dir = tempfile::tempdir().unwrap();
        ensure_tracked(dir.path(), "a.txt", Some(b"conteudo original")).unwrap();

        let entry = get(dir.path(), "a.txt")
            .unwrap()
            .expect("deveria estar rastreado");
        assert!(entry.existed);
        let hash = entry.content_hash.expect("deveria ter hash");
        assert_eq!(
            fs::read(dir.path().join("objects").join(&hash)).unwrap(),
            b"conteudo original"
        );
    }

    #[test]
    fn chamadas_seguintes_nao_sobrescrevem_o_original() {
        let dir = tempfile::tempdir().unwrap();
        ensure_tracked(dir.path(), "a.txt", Some(b"primeira versao")).unwrap();
        ensure_tracked(dir.path(), "a.txt", Some(b"versao bem depois")).unwrap();

        let entry = get(dir.path(), "a.txt").unwrap().unwrap();
        let hash = entry.content_hash.unwrap();
        assert_eq!(
            fs::read(dir.path().join("objects").join(&hash)).unwrap(),
            b"primeira versao",
            "a segunda chamada não deveria ter sobrescrito o original"
        );
    }

    #[test]
    fn arquivo_que_nao_existia_fica_marcado_sem_hash() {
        let dir = tempfile::tempdir().unwrap();
        ensure_tracked(dir.path(), "novo.txt", None).unwrap();

        let entry = get(dir.path(), "novo.txt").unwrap().unwrap();
        assert!(!entry.existed);
        assert!(entry.content_hash.is_none());
    }

    #[test]
    fn arquivo_grande_demais_fica_marcado_como_existente_sem_hash() {
        let dir = tempfile::tempdir().unwrap();
        let grande = vec![b'x'; MAX_FILE_SIZE + 1];
        ensure_tracked(dir.path(), "grande.txt", Some(&grande)).unwrap();

        let entry = get(dir.path(), "grande.txt").unwrap().unwrap();
        assert!(entry.existed);
        assert!(entry.content_hash.is_none());
    }

    #[test]
    fn get_sem_nenhum_registro_e_none() {
        let dir = tempfile::tempdir().unwrap();
        assert!(get(dir.path(), "nunca-tocado.txt").unwrap().is_none());
    }

    #[test]
    fn list_devolve_todo_arquivo_rastreado() {
        let dir = tempfile::tempdir().unwrap();
        ensure_tracked(dir.path(), "a.txt", Some(b"a")).unwrap();
        ensure_tracked(dir.path(), "b.txt", None).unwrap();

        let mut paths: Vec<String> = list(dir.path())
            .unwrap()
            .into_iter()
            .map(|e| e.path)
            .collect();
        paths.sort();
        assert_eq!(paths, vec!["a.txt".to_string(), "b.txt".to_string()]);
    }

    #[test]
    fn list_sem_nenhum_arquivo_rastreado_e_vazia() {
        let dir = tempfile::tempdir().unwrap();
        assert!(list(dir.path()).unwrap().is_empty());
    }
}
