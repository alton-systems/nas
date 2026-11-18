//! Checkpoints nomeados do projeto: um snapshot, num só momento, do
//! conteúdo de todo arquivo que mudou desde o checkpoint anterior —
//! docs/decisions/0005-historico-com-autoria.md §4 e §5.

use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::author::Author;
use crate::journal::{journal_path, read_entries};

/// Arquivos maiores que isto não entram em `objects/` — mesmo
/// critério de tamanho que `Document::open` usa, só que aqui é sobre
/// não guardar binários grandes, não sobre recusar abrir.
pub const MAX_FILE_SIZE: usize = 2 * 1024 * 1024;

/// Quantos checkpoints automáticos reter, por padrão, antes de podar
/// os mais antigos — nomeados nunca entram nessa conta.
pub const MAX_AUTOMATIC_CHECKPOINTS: usize = 20;

/// Limite de tamanho total em disco (journal excluído, só
/// `objects/` + `checkpoints/`) por projeto, antes de podar
/// automáticos por tamanho.
pub const MAX_TOTAL_SIZE_BYTES: u64 = 500 * 1024 * 1024;

/// Por que um arquivo não entrou em `objects/` — pra avisar o usuário
/// quais ficaram de fora (docs/decisions/0005 §5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExcludedReason {
    TooLarge,
    NotUtf8,
}

#[derive(Debug, Clone)]
pub struct ExcludedFile {
    pub path: String,
    pub reason: ExcludedReason,
}

/// Um arquivo dentro de um checkpoint. `content_hash` é `None` quando
/// o arquivo ficou de fora de `objects/` (grande demais ou não-UTF8)
/// — o caminho e o tamanho continuam registrados mesmo assim.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointFile {
    pub path: String,
    pub size: usize,
    pub content_hash: Option<String>,
}

/// `checkpoints/<id>/manifest.toml` — docs/decisions/0005 §5. `id` é
/// `<timestamp>-<nome-normalizado>`, ordenável só listando o
/// diretório.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointManifest {
    pub id: String,
    pub name: String,
    pub created_at: u64,
    pub author: Author,
    pub automatic: bool,
    pub files: Vec<CheckpointFile>,
}

pub struct CreatedCheckpoint {
    pub manifest: CheckpointManifest,
    pub excluded: Vec<ExcludedFile>,
}

/// O que uma poda removeu — pra uma futura mensagem de status.
#[derive(Debug, Clone, Default)]
pub struct PruneReport {
    pub removed_checkpoints: Vec<String>,
    pub removed_objects: usize,
    /// `true` se o limite de tamanho total ainda estava excedido
    /// depois de remover todos os automáticos — nenhum nomeado é
    /// removido só por pressão de espaço (docs/decisions/0005 §5).
    pub stopped_due_to_named_only: bool,
}

fn objects_dir(history_dir: &Path) -> PathBuf {
    history_dir.join("objects")
}

fn checkpoints_dir(history_dir: &Path) -> PathBuf {
    history_dir.join("checkpoints")
}

fn normalize_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect()
}

fn hash_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Cria um checkpoint com o conteúdo atual de `files` (caminho,
/// bytes) — normalmente os arquivos que mudaram desde o anterior (ver
/// [`changed_files_since_last_checkpoint`]), mas este módulo só guarda
/// o que recebe; decidir quais arquivos passar é de quem chama.
/// Arquivos grandes demais ou não-UTF8 ainda entram no manifesto (nome
/// e tamanho), só não em `objects/` — devolvidos em `excluded`.
pub fn create(
    history_dir: &Path,
    name: &str,
    author: Author,
    now: u64,
    automatic: bool,
    files: &[(String, Vec<u8>)],
) -> io::Result<CreatedCheckpoint> {
    let objects = objects_dir(history_dir);
    fs::create_dir_all(&objects)?;

    let mut checkpoint_files = Vec::with_capacity(files.len());
    let mut excluded = Vec::new();

    for (path, content) in files {
        let size = content.len();
        if size > MAX_FILE_SIZE {
            excluded.push(ExcludedFile {
                path: path.clone(),
                reason: ExcludedReason::TooLarge,
            });
            checkpoint_files.push(CheckpointFile {
                path: path.clone(),
                size,
                content_hash: None,
            });
            continue;
        }
        if std::str::from_utf8(content).is_err() {
            excluded.push(ExcludedFile {
                path: path.clone(),
                reason: ExcludedReason::NotUtf8,
            });
            checkpoint_files.push(CheckpointFile {
                path: path.clone(),
                size,
                content_hash: None,
            });
            continue;
        }

        let hash = hash_bytes(content);
        let object_path = objects.join(&hash);
        if !object_path.exists() {
            fs::write(&object_path, content)?;
        }
        checkpoint_files.push(CheckpointFile {
            path: path.clone(),
            size,
            content_hash: Some(hash),
        });
    }

    let id = format!("{now}-{}", normalize_name(name));
    let dir = checkpoints_dir(history_dir).join(&id);
    fs::create_dir_all(&dir)?;

    let manifest = CheckpointManifest {
        id: id.clone(),
        name: name.to_string(),
        created_at: now,
        author,
        automatic,
        files: checkpoint_files,
    };
    let toml = toml::to_string_pretty(&manifest)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    fs::write(dir.join("manifest.toml"), toml)?;

    Ok(CreatedCheckpoint { manifest, excluded })
}

/// Lê todos os manifestos em `checkpoints/`, em ordem cronológica
/// (`id` começa pelo timestamp) — uma pasta inexistente (nenhum
/// checkpoint ainda) devolve uma lista vazia.
pub fn list(history_dir: &Path) -> io::Result<Vec<CheckpointManifest>> {
    let dir = checkpoints_dir(history_dir);
    let mut manifests = Vec::new();
    let entries = match fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err),
    };
    for entry in entries {
        let manifest_path = entry?.path().join("manifest.toml");
        let content = fs::read_to_string(&manifest_path)?;
        let manifest: CheckpointManifest = toml::from_str(&content)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        manifests.push(manifest);
    }
    manifests.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(manifests)
}

/// Os caminhos distintos com ao menos uma entrada no journal desde o
/// checkpoint mais recente (ou desde o início, se não houver nenhum
/// ainda) — docs/decisions/0005 §4: um checkpoint só inclui arquivos
/// que mudaram.
pub fn changed_files_since_last_checkpoint(history_dir: &Path) -> io::Result<Vec<String>> {
    let since = list(history_dir)?.last().map_or(0, |c| c.created_at);
    let entries = read_entries(&journal_path(history_dir))?;
    let mut seen = HashSet::new();
    let mut files = Vec::new();
    for entry in entries {
        if entry.ts_end >= since && seen.insert(entry.file.clone()) {
            files.push(entry.file);
        }
    }
    Ok(files)
}

/// Lê o conteúdo de um objeto pelo hash — o mesmo conteúdo gravado
/// por [`create`], endereçado em `objects/<hash>`.
pub fn read_object(history_dir: &Path, hash: &str) -> io::Result<Vec<u8>> {
    fs::read(objects_dir(history_dir).join(hash))
}

/// Como um arquivo mudou entre dois checkpoints — docs/decisions/0005
/// §7.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileChange {
    Added,
    Removed,
    Modified,
    Unchanged,
}

/// O diff de um arquivo entre dois checkpoints. `unified` é o diff de
/// linhas no formato unificado comum (via a crate `similar`); vazio
/// quando `change` é `Unchanged`, ou quando o conteúdo de um dos lados
/// não está disponível (arquivo excluído de `objects/` por ser grande
/// demais ou não-UTF8 — docs/decisions/0005 §5).
#[derive(Debug, Clone)]
pub struct FileDiff {
    pub path: String,
    pub change: FileChange,
    pub unified: String,
}

/// Diff entre dois checkpoints (`from_id`, `to_id`, como em
/// [`CheckpointManifest::id`]): para cada caminho presente em
/// qualquer um dos dois, compara o conteúdo guardado em `objects/` —
/// ou a ausência, se o arquivo só existe num dos dois. Erro se
/// `from_id` ou `to_id` não existir entre os checkpoints do projeto.
pub fn diff(history_dir: &Path, from_id: &str, to_id: &str) -> io::Result<Vec<FileDiff>> {
    let manifests = list(history_dir)?;
    let find = |id: &str| -> io::Result<CheckpointManifest> {
        manifests
            .iter()
            .find(|m| m.id == id)
            .cloned()
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("checkpoint não encontrado: {id}"),
                )
            })
    };
    let from = find(from_id)?;
    let to = find(to_id)?;

    let mut paths = Vec::new();
    let mut seen = HashSet::new();
    for f in from.files.iter().chain(to.files.iter()) {
        if seen.insert(f.path.clone()) {
            paths.push(f.path.clone());
        }
    }
    paths.sort();

    paths
        .into_iter()
        .map(|path| {
            let from_file = from.files.iter().find(|f| f.path == path);
            let to_file = to.files.iter().find(|f| f.path == path);
            diff_file(history_dir, &path, from_file, to_file)
        })
        .collect()
}

fn diff_file(
    history_dir: &Path,
    path: &str,
    from: Option<&CheckpointFile>,
    to: Option<&CheckpointFile>,
) -> io::Result<FileDiff> {
    match (from, to) {
        (None, Some(t)) => {
            let content = read_file_text(history_dir, t)?;
            Ok(FileDiff {
                path: path.to_string(),
                change: FileChange::Added,
                unified: unified_text(path, "", &content),
            })
        }
        (Some(f), None) => {
            let content = read_file_text(history_dir, f)?;
            Ok(FileDiff {
                path: path.to_string(),
                change: FileChange::Removed,
                unified: unified_text(path, &content, ""),
            })
        }
        (Some(f), Some(t)) => {
            if f.content_hash == t.content_hash {
                return Ok(FileDiff {
                    path: path.to_string(),
                    change: FileChange::Unchanged,
                    unified: String::new(),
                });
            }
            let old = read_file_text(history_dir, f)?;
            let new = read_file_text(history_dir, t)?;
            Ok(FileDiff {
                path: path.to_string(),
                change: FileChange::Modified,
                unified: unified_text(path, &old, &new),
            })
        }
        (None, None) => unreachable!("todo caminho vem de um dos dois manifestos"),
    }
}

/// O conteúdo de um `CheckpointFile` como texto — vazio se ele não
/// tem `content_hash` (excluído de `objects/`, grande demais ou
/// não-UTF8): não há o que comparar nesse caso, então o diff mostra o
/// arquivo como tendo mudado, sem linhas de conteúdo.
fn read_file_text(history_dir: &Path, file: &CheckpointFile) -> io::Result<String> {
    match &file.content_hash {
        Some(hash) => {
            let bytes = read_object(history_dir, hash)?;
            Ok(String::from_utf8_lossy(&bytes).into_owned())
        }
        None => Ok(String::new()),
    }
}

fn unified_text(path: &str, old: &str, new: &str) -> String {
    similar::TextDiff::from_lines(old, new)
        .unified_diff()
        .header(&format!("a/{path}"), &format!("b/{path}"))
        .to_string()
}

/// Poda com os limites padrão (`MAX_AUTOMATIC_CHECKPOINTS`,
/// `MAX_TOTAL_SIZE_BYTES`) — ver [`prune_with_limits`].
pub fn prune(history_dir: &Path) -> io::Result<PruneReport> {
    prune_with_limits(history_dir, MAX_AUTOMATIC_CHECKPOINTS, MAX_TOTAL_SIZE_BYTES)
}

/// Poda por dois limites, nunca removendo um checkpoint nomeado —
/// docs/decisions/0005 §5. Limites explícitos (em vez de usar sempre
/// as constantes) pra ficar testável sem precisar de 500 MiB de
/// verdade em disco.
pub fn prune_with_limits(
    history_dir: &Path,
    max_automatic: usize,
    max_total_bytes: u64,
) -> io::Result<PruneReport> {
    let mut manifests = list(history_dir)?;
    let mut removed_checkpoints = Vec::new();

    // 1. Retenção por quantidade: os automáticos mais antigos, além
    //    dos `max_automatic` mais recentes, são os primeiros candidatos.
    let automatic_indices: Vec<usize> = manifests
        .iter()
        .enumerate()
        .filter(|(_, m)| m.automatic)
        .map(|(i, _)| i)
        .collect();
    if automatic_indices.len() > max_automatic {
        let excess = automatic_indices.len() - max_automatic;
        for &idx in &automatic_indices[..excess] {
            removed_checkpoints.push(manifests[idx].id.clone());
        }
    }
    remove_checkpoints(history_dir, &mut manifests, &removed_checkpoints)?;

    // 2. Limite de tamanho total: enquanto exceder, remove o
    //    automático mais antigo restante; para e avisa se só
    //    restarem nomeados.
    let mut stopped_due_to_named_only = false;
    loop {
        if dir_size(history_dir)? <= max_total_bytes {
            break;
        }
        let Some(idx) = manifests.iter().position(|m| m.automatic) else {
            stopped_due_to_named_only = true;
            break;
        };
        let id = manifests[idx].id.clone();
        remove_checkpoints(history_dir, &mut manifests, std::slice::from_ref(&id))?;
        removed_checkpoints.push(id);
    }

    // 3. Objetos órfãos: nada referenciado por nenhum manifesto
    //    sobrevivente.
    let removed_objects = garbage_collect_objects(history_dir, &manifests)?;

    // 4. Journal: só entradas mais novas que o checkpoint mais antigo
    //    ainda vivo (nomeado ou automático).
    if let Some(oldest) = manifests.iter().map(|m| m.created_at).min() {
        prune_journal(history_dir, oldest)?;
    }

    Ok(PruneReport {
        removed_checkpoints,
        removed_objects,
        stopped_due_to_named_only,
    })
}

fn remove_checkpoints(
    history_dir: &Path,
    manifests: &mut Vec<CheckpointManifest>,
    ids: &[String],
) -> io::Result<()> {
    let to_remove: HashSet<&String> = ids.iter().collect();
    manifests.retain(|m| !to_remove.contains(&m.id));
    for id in ids {
        let dir = checkpoints_dir(history_dir).join(id);
        match fs::remove_dir_all(&dir) {
            Ok(()) => {}
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => return Err(err),
        }
    }
    Ok(())
}

fn garbage_collect_objects(
    history_dir: &Path,
    surviving: &[CheckpointManifest],
) -> io::Result<usize> {
    let referenced: HashSet<&str> = surviving
        .iter()
        .flat_map(|m| m.files.iter().filter_map(|f| f.content_hash.as_deref()))
        .collect();

    let objects = objects_dir(history_dir);
    let entries = match fs::read_dir(&objects) {
        Ok(entries) => entries,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(err) => return Err(err),
    };

    let mut removed = 0;
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name();
        if !referenced.contains(name.to_string_lossy().as_ref()) {
            fs::remove_file(entry.path())?;
            removed += 1;
        }
    }
    Ok(removed)
}

fn prune_journal(history_dir: &Path, cutoff: u64) -> io::Result<()> {
    let path = journal_path(history_dir);
    let entries = read_entries(&path)?;
    let mut content = String::new();
    for entry in entries.iter().filter(|e| e.ts_end >= cutoff) {
        let line = serde_json::to_string(entry)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        content.push_str(&line);
        content.push('\n');
    }
    fs::write(&path, content)
}

fn dir_size(history_dir: &Path) -> io::Result<u64> {
    fn walk(dir: &Path) -> io::Result<u64> {
        let entries = match fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(0),
            Err(err) => return Err(err),
        };
        let mut total = 0u64;
        for entry in entries {
            let entry = entry?;
            let metadata = entry.metadata()?;
            total += if metadata.is_dir() {
                walk(&entry.path())?
            } else {
                metadata.len()
            };
        }
        Ok(total)
    }
    Ok(walk(&objects_dir(history_dir))? + walk(&checkpoints_dir(history_dir))?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::journal::Journal;

    fn file(path: &str, content: &str) -> (String, Vec<u8>) {
        (path.to_string(), content.as_bytes().to_vec())
    }

    #[test]
    fn cria_checkpoint_e_grava_o_conteudo_em_objects() {
        let dir = tempfile::tempdir().unwrap();
        let files = [file("a.txt", "conteudo a"), file("b.txt", "conteudo b")];

        let created = create(dir.path(), "primeiro", Author::User, 100, false, &files).unwrap();

        assert_eq!(created.manifest.files.len(), 2);
        assert!(created.excluded.is_empty());
        for f in &created.manifest.files {
            let hash = f.content_hash.as_ref().expect("deveria ter hash");
            assert!(dir.path().join("objects").join(hash).exists());
        }
        assert_eq!(created.manifest.id, "100-primeiro");
        assert!(!created.manifest.automatic);
    }

    #[test]
    fn conteudo_identico_entre_checkpoints_nao_duplica_objeto() {
        let dir = tempfile::tempdir().unwrap();
        let files = [file("a.txt", "mesmo conteudo")];

        create(dir.path(), "um", Author::User, 100, false, &files).unwrap();
        create(dir.path(), "dois", Author::User, 200, false, &files).unwrap();

        let objects: Vec<_> = fs::read_dir(dir.path().join("objects")).unwrap().collect();
        assert_eq!(
            objects.len(),
            1,
            "o mesmo conteúdo não deveria duplicar o objeto"
        );
    }

    #[test]
    fn arquivo_grande_demais_fica_de_fora_mas_registrado() {
        let dir = tempfile::tempdir().unwrap();
        let grande = "x".repeat(MAX_FILE_SIZE + 1);
        let files = [file("grande.txt", &grande)];

        let created = create(dir.path(), "cp", Author::User, 100, false, &files).unwrap();

        assert_eq!(created.excluded.len(), 1);
        assert_eq!(created.excluded[0].path, "grande.txt");
        assert_eq!(created.excluded[0].reason, ExcludedReason::TooLarge);
        assert_eq!(created.manifest.files[0].size, grande.len());
        assert!(created.manifest.files[0].content_hash.is_none());
        assert!(
            fs::read_dir(dir.path().join("objects"))
                .unwrap()
                .next()
                .is_none(),
            "não deveria ter gravado nada em objects/"
        );
    }

    #[test]
    fn arquivo_nao_utf8_fica_de_fora_mas_registrado() {
        let dir = tempfile::tempdir().unwrap();
        let files = [("binario.bin".to_string(), vec![0xFF, 0xFE, 0x00])];

        let created = create(dir.path(), "cp", Author::User, 100, false, &files).unwrap();

        assert_eq!(created.excluded.len(), 1);
        assert_eq!(created.excluded[0].reason, ExcludedReason::NotUtf8);
        assert!(created.manifest.files[0].content_hash.is_none());
    }

    #[test]
    fn list_devolve_em_ordem_cronologica() {
        let dir = tempfile::tempdir().unwrap();
        create(dir.path(), "terceiro", Author::User, 300, false, &[]).unwrap();
        create(dir.path(), "primeiro", Author::User, 100, false, &[]).unwrap();
        create(dir.path(), "segundo", Author::User, 200, false, &[]).unwrap();

        let names: Vec<String> = list(dir.path())
            .unwrap()
            .into_iter()
            .map(|m| m.name)
            .collect();
        assert_eq!(names, vec!["primeiro", "segundo", "terceiro"]);
    }

    #[test]
    fn list_sem_nenhum_checkpoint_e_vazia() {
        let dir = tempfile::tempdir().unwrap();
        assert!(list(dir.path()).unwrap().is_empty());
    }

    #[test]
    fn diff_detecta_adicionado_removido_modificado_e_inalterado() {
        let dir = tempfile::tempdir().unwrap();
        let before = [
            file("modificado.txt", "linha um\nlinha dois\n"),
            file("removido.txt", "vai sair\n"),
            file("igual.txt", "nunca muda\n"),
        ];
        let after = [
            file("modificado.txt", "linha um\nlinha dois mudou\n"),
            file("adicionado.txt", "chegou agora\n"),
            file("igual.txt", "nunca muda\n"),
        ];
        let from = create(dir.path(), "antes", Author::User, 100, false, &before).unwrap();
        let to = create(dir.path(), "depois", Author::User, 200, false, &after).unwrap();

        let diffs = diff(dir.path(), &from.manifest.id, &to.manifest.id).unwrap();
        let find = |path: &str| diffs.iter().find(|d| d.path == path).unwrap();

        assert_eq!(find("adicionado.txt").change, FileChange::Added);
        assert!(find("adicionado.txt").unified.contains("chegou agora"));

        assert_eq!(find("removido.txt").change, FileChange::Removed);
        assert!(find("removido.txt").unified.contains("vai sair"));

        assert_eq!(find("modificado.txt").change, FileChange::Modified);
        assert!(find("modificado.txt").unified.contains("linha dois mudou"));

        assert_eq!(find("igual.txt").change, FileChange::Unchanged);
        assert!(find("igual.txt").unified.is_empty());
    }

    #[test]
    fn diff_com_checkpoint_inexistente_e_erro() {
        let dir = tempfile::tempdir().unwrap();
        let cp = create(dir.path(), "cp", Author::User, 100, false, &[]).unwrap();

        let err = diff(dir.path(), &cp.manifest.id, "999-fantasma").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn read_object_le_o_conteudo_gravado_por_create() {
        let dir = tempfile::tempdir().unwrap();
        let created = create(
            dir.path(),
            "cp",
            Author::User,
            100,
            false,
            &[file("a.txt", "conteudo")],
        )
        .unwrap();
        let hash = created.manifest.files[0].content_hash.as_ref().unwrap();

        let bytes = read_object(dir.path(), hash).unwrap();
        assert_eq!(bytes, b"conteudo");
    }

    fn record(journal: &mut Journal, now: u64, file: &str, pos: usize) {
        journal
            .record(now, Author::User, "insert", file, pos, pos, "", "x")
            .unwrap();
    }

    #[test]
    fn changed_files_sem_checkpoint_pega_tudo_que_ja_foi_journalado() {
        let dir = tempfile::tempdir().unwrap();
        let mut journal = Journal::open(dir.path()).unwrap();
        record(&mut journal, 50, "a.txt", 0);
        record(&mut journal, 60, "b.txt", 0);
        journal.flush(60).unwrap();

        let mut changed = changed_files_since_last_checkpoint(dir.path()).unwrap();
        changed.sort();
        assert_eq!(changed, vec!["a.txt".to_string(), "b.txt".to_string()]);
    }

    #[test]
    fn changed_files_so_pega_o_que_mudou_depois_do_ultimo_checkpoint() {
        let dir = tempfile::tempdir().unwrap();
        let mut journal = Journal::open(dir.path()).unwrap();
        record(&mut journal, 50, "a.txt", 0);
        journal.flush(50).unwrap();

        create(dir.path(), "cp", Author::User, 100, false, &[]).unwrap();

        record(&mut journal, 150, "b.txt", 0);
        journal.flush(150).unwrap();

        let changed = changed_files_since_last_checkpoint(dir.path()).unwrap();
        assert_eq!(changed, vec!["b.txt".to_string()]);
    }

    fn automatic_checkpoint(dir: &Path, name: &str, now: u64) {
        create(dir, name, Author::User, now, true, &[]).unwrap();
    }

    fn named_checkpoint(dir: &Path, name: &str, now: u64) {
        create(dir, name, Author::User, now, false, &[]).unwrap();
    }

    #[test]
    fn poda_mantem_so_os_automaticos_mais_recentes() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..25 {
            automatic_checkpoint(dir.path(), &format!("auto-{i}"), 1000 + i);
        }

        let report = prune_with_limits(dir.path(), 20, MAX_TOTAL_SIZE_BYTES).unwrap();

        assert_eq!(report.removed_checkpoints.len(), 5);
        let remaining = list(dir.path()).unwrap();
        assert_eq!(remaining.len(), 20);
        assert_eq!(
            remaining[0].name, "auto-5",
            "deveria ter removido os 5 mais antigos"
        );
    }

    #[test]
    fn poda_nunca_remove_um_checkpoint_nomeado_por_quantidade() {
        let dir = tempfile::tempdir().unwrap();
        named_checkpoint(dir.path(), "importante", 500);
        for i in 0..25 {
            automatic_checkpoint(dir.path(), &format!("auto-{i}"), 1000 + i);
        }

        prune_with_limits(dir.path(), 20, MAX_TOTAL_SIZE_BYTES).unwrap();

        let remaining = list(dir.path()).unwrap();
        assert!(remaining.iter().any(|m| m.name == "importante"));
        assert_eq!(remaining.iter().filter(|m| m.automatic).count(), 20);
    }

    #[test]
    fn poda_por_tamanho_remove_automaticos_mais_antigos_primeiro() {
        let dir = tempfile::tempdir().unwrap();
        let conteudo = [file("a.txt", &"x".repeat(100))];
        automatic_checkpoint_com_conteudo(dir.path(), "velho", 100, &conteudo);
        automatic_checkpoint_com_conteudo(dir.path(), "novo", 200, &conteudo);

        // cada checkpoint tem ~100 bytes de conteúdo (deduplicado, então só
        // 1 objeto de ~100 bytes no total) mais o manifesto; um limite de
        // 1 byte força a poda por tamanho a agir.
        let report = prune_with_limits(dir.path(), MAX_AUTOMATIC_CHECKPOINTS, 1).unwrap();

        let remaining = list(dir.path()).unwrap();
        assert!(
            remaining.len() < 2,
            "deveria ter removido pelo menos um checkpoint por tamanho"
        );
        if !remaining.is_empty() {
            assert_eq!(
                remaining[0].name, "novo",
                "o mais antigo deveria ter sido removido primeiro"
            );
        }
        assert!(!report.removed_checkpoints.is_empty());
    }

    fn automatic_checkpoint_com_conteudo(
        dir: &Path,
        name: &str,
        now: u64,
        files: &[(String, Vec<u8>)],
    ) {
        create(dir, name, Author::User, now, true, files).unwrap();
    }

    #[test]
    fn poda_para_e_avisa_se_so_restarem_nomeados_e_ainda_exceder() {
        let dir = tempfile::tempdir().unwrap();
        let conteudo = [file("a.txt", &"x".repeat(100))];
        named_checkpoint_com_conteudo(dir.path(), "nomeado", 100, &conteudo);

        let report = prune_with_limits(dir.path(), MAX_AUTOMATIC_CHECKPOINTS, 1).unwrap();

        assert!(report.stopped_due_to_named_only);
        assert_eq!(
            list(dir.path()).unwrap().len(),
            1,
            "o nomeado não deveria ser removido"
        );
    }

    fn named_checkpoint_com_conteudo(
        dir: &Path,
        name: &str,
        now: u64,
        files: &[(String, Vec<u8>)],
    ) {
        create(dir, name, Author::User, now, false, files).unwrap();
    }

    #[test]
    fn poda_coleta_objetos_orfaos_mas_preserva_os_referenciados() {
        let dir = tempfile::tempdir().unwrap();
        let unico = [file("so-no-velho.txt", "conteudo unico")];
        let compartilhado = [file("compartilhado.txt", "conteudo compartilhado")];

        automatic_checkpoint_com_conteudo(dir.path(), "velho", 100, &unico);
        let mut ambos = compartilhado.to_vec();
        ambos.extend(unico.iter().cloned());
        // o "novo" também referencia "compartilhado.txt" — deve sobreviver.
        automatic_checkpoint_com_conteudo(
            dir.path(),
            "novo",
            100 + MAX_AUTOMATIC_CHECKPOINTS as u64 + 1,
            &compartilhado,
        );

        prune_with_limits(dir.path(), 1, MAX_TOTAL_SIZE_BYTES).unwrap();

        let objects: Vec<String> = fs::read_dir(dir.path().join("objects"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(
            objects.len(),
            1,
            "só o objeto do checkpoint sobrevivente deveria restar"
        );
    }

    #[test]
    fn poda_do_journal_remove_entradas_mais_antigas_que_o_checkpoint_sobrevivente() {
        let dir = tempfile::tempdir().unwrap();
        let mut journal = Journal::open(dir.path()).unwrap();
        record(&mut journal, 50, "a.txt", 0);
        record(&mut journal, 500, "b.txt", 0);
        journal.flush(500).unwrap();
        drop(journal);

        automatic_checkpoint(dir.path(), "cp", 100);

        prune_with_limits(dir.path(), MAX_AUTOMATIC_CHECKPOINTS, MAX_TOTAL_SIZE_BYTES).unwrap();

        let entries = read_entries(&journal_path(dir.path())).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].file, "b.txt");
    }
}
