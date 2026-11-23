//! Restaurar o projeto inteiro pro estado de um checkpoint —
//! docs/decisions/0005-historico-com-autoria.md §4/§7. Uma função
//! só, compartilhada por `nas-term` (`Ctrl+t r`) e `nas-commands`
//! (`checkpoint_restore`, docs/decisions/0006 §1) — antes, cada um
//! tinha sua própria cópia do mesmo algoritmo.
//!
//! Esta crate não depende de `nas-editor` (ADR 0005 §9): quem
//! escreve de verdade — num buffer aberto, ou direto no disco — é
//! responsabilidade de quem chama, através de [`RestoreTarget`].

use std::collections::HashSet;
use std::io;
use std::path::Path;

use serde::Serialize;

use crate::author::Author;
use crate::checkpoint::{self, CheckpointManifest};
use crate::originals;

/// O que [`restore`] precisa de quem chama: como ler o conteúdo atual
/// de um caminho (pra montar o checkpoint automático "antes de
/// restaurar") e como escrever um conteúdo novo nele — incluindo
/// journalar a mudança, já que cada chamador sabe como fazer isso
/// (um buffer aberto ou não, `nas-term` ou `nas-commands`).
pub trait RestoreTarget {
    /// O conteúdo atual de `path` — de um buffer aberto, se houver
    /// um; senão, do próprio arquivo em disco. `None` se não houver
    /// buffer e o arquivo não existir (ou não puder ser lido).
    fn current_content(&self, path: &str) -> Option<Vec<u8>>;

    /// Escreve `content` em `path` — atualiza um buffer aberto (e
    /// salva) se houver um, senão escreve direto no arquivo; journala
    /// a mudança como `"restore-checkpoint"`. Erros de E/S nunca são
    /// ignorados — quem chama [`restore`] vê cada falha em
    /// `RestoreReport::failed`.
    fn write(&mut self, path: &str, content: &str) -> io::Result<()>;

    /// Move `path` pra lixeira do sistema (nunca apaga de vez) —
    /// usado quando `path` foi rastreado só depois do checkpoint
    /// sendo restaurado e não existia antes dele. Implementação
    /// padrão: [`crate::move_to_trash`].
    fn trash(&self, path: &str) -> io::Result<()> {
        crate::trash::move_to_trash(Path::new(path))
    }
}

/// O resultado de uma restauração — cada caminho rastreado cai em
/// exatamente um destes baldes.
#[derive(Debug, Clone, Default, Serialize)]
pub struct RestoreReport {
    pub restored: Vec<String>,
    pub trashed: Vec<String>,
    pub skipped: Vec<String>,
    pub failed: Vec<(String, String)>,
}

/// Restaura o **projeto inteiro** pro estado de `checkpoint`, não só
/// os arquivos que ele lista (docs/decisions/0005 §4, correção):
///
/// 1. Cria primeiro um checkpoint automático "antes de restaurar" com
///    o conteúdo atual de **todo** arquivo rastreado — o que torna a
///    própria restauração desfazível por completo (restaurar esse
///    automático desfaz tudo, inclusive os arquivos revertidos ou
///    jogados na lixeira abaixo), sem nenhum código separado.
/// 2. Arquivos rastreados que `checkpoint` não lista (tocados pela
///    primeira vez depois dele): revertidos ao original, se existiam;
///    jogados na lixeira, se não existiam ainda.
/// 3. Os arquivos que `checkpoint` lista: escritos com o conteúdo
///    guardado nele.
///
/// Nenhum erro de gravação é ignorado — cada arquivo que falhar
/// aparece em `RestoreReport::failed`, com o motivo.
pub fn restore(
    history_dir: &Path,
    checkpoint: &CheckpointManifest,
    author: Author,
    now: u64,
    target: &mut impl RestoreTarget,
) -> io::Result<RestoreReport> {
    let tracked = originals::list(history_dir)?;

    let pre_restore_files: Vec<(String, Vec<u8>)> = tracked
        .iter()
        .filter_map(|entry| {
            let content = target.current_content(&entry.path)?;
            Some((entry.path.clone(), content))
        })
        .collect();
    if !pre_restore_files.is_empty() {
        let name = format!("antes de restaurar {}", checkpoint.name);
        checkpoint::create(history_dir, &name, author, now, true, &pre_restore_files)?;
    }

    let mut report = RestoreReport::default();
    let in_checkpoint: HashSet<&str> = checkpoint.files.iter().map(|f| f.path.as_str()).collect();

    for entry in tracked
        .iter()
        .filter(|entry| !in_checkpoint.contains(entry.path.as_str()))
    {
        if !entry.existed {
            match target.trash(&entry.path) {
                Ok(()) => report.trashed.push(entry.path.clone()),
                Err(err) => report.failed.push((entry.path.clone(), err.to_string())),
            }
            continue;
        }
        let Some(hash) = &entry.content_hash else {
            report.skipped.push(entry.path.clone());
            continue;
        };
        match read_object_as_text(history_dir, hash) {
            Some(content) => match target.write(&entry.path, &content) {
                Ok(()) => report.restored.push(entry.path.clone()),
                Err(err) => report.failed.push((entry.path.clone(), err.to_string())),
            },
            None => report.skipped.push(entry.path.clone()),
        }
    }

    for file in &checkpoint.files {
        let Some(hash) = &file.content_hash else {
            report.skipped.push(file.path.clone());
            continue;
        };
        match read_object_as_text(history_dir, hash) {
            Some(content) => match target.write(&file.path, &content) {
                Ok(()) => report.restored.push(file.path.clone()),
                Err(err) => report.failed.push((file.path.clone(), err.to_string())),
            },
            None => report.skipped.push(file.path.clone()),
        }
    }

    Ok(report)
}

/// O conteúdo de um objeto do histórico como texto — `None` se não
/// puder ser lido ou não for UTF-8 válido (não deveria acontecer:
/// objetos só guardam conteúdo que já passou por esse mesmo crivo ao
/// serem criados, mas um erro de E/S na leitura ainda é possível).
fn read_object_as_text(history_dir: &Path, hash: &str) -> Option<String> {
    checkpoint::read_object(history_dir, hash)
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::checkpoint;
    use crate::originals;

    /// Um `RestoreTarget` de brinquedo — um mapa em memória em vez de
    /// buffers ou arquivos de verdade; `trash` não toca o disco
    /// (os caminhos de teste não existem de verdade).
    struct FakeTarget(HashMap<String, String>);

    impl RestoreTarget for FakeTarget {
        fn current_content(&self, path: &str) -> Option<Vec<u8>> {
            self.0.get(path).map(|s| s.as_bytes().to_vec())
        }

        fn write(&mut self, path: &str, content: &str) -> io::Result<()> {
            self.0.insert(path.to_string(), content.to_string());
            Ok(())
        }

        fn trash(&self, _path: &str) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn restaura_arquivos_do_checkpoint_e_joga_na_lixeira_o_que_nao_existia_antes() {
        let dir = tempfile::tempdir().unwrap();

        originals::ensure_tracked(dir.path(), "a.txt", Some(b"original")).unwrap();
        let checkpoint = checkpoint::create(
            dir.path(),
            "cp1",
            Author::User,
            100,
            false,
            &[("a.txt".to_string(), b"v1".to_vec())],
        )
        .unwrap()
        .manifest;
        // "b.txt" só passa a ser rastreado *depois* de cp1 — não
        // aparece no manifesto dele.
        originals::ensure_tracked(dir.path(), "b.txt", None).unwrap();

        let mut target = FakeTarget(HashMap::from([
            ("a.txt".to_string(), "v1 mudado".to_string()),
            ("b.txt".to_string(), "arquivo novo".to_string()),
        ]));

        let report = restore(dir.path(), &checkpoint, Author::User, 200, &mut target).unwrap();

        assert_eq!(report.restored, vec!["a.txt".to_string()]);
        assert_eq!(report.trashed, vec!["b.txt".to_string()]);
        assert!(report.failed.is_empty());
        assert_eq!(target.0.get("a.txt").unwrap(), "v1");
    }

    #[test]
    fn cria_checkpoint_automatico_antes_de_restaurar_com_o_estado_atual() {
        let dir = tempfile::tempdir().unwrap();
        originals::ensure_tracked(dir.path(), "a.txt", Some(b"original")).unwrap();
        let checkpoint = checkpoint::create(
            dir.path(),
            "cp1",
            Author::User,
            100,
            false,
            &[("a.txt".to_string(), b"v1".to_vec())],
        )
        .unwrap()
        .manifest;

        let mut target = FakeTarget(HashMap::from([(
            "a.txt".to_string(),
            "v1 mudado".to_string(),
        )]));
        restore(dir.path(), &checkpoint, Author::User, 200, &mut target).unwrap();

        let manifests = checkpoint::list(dir.path()).unwrap();
        let automatic = manifests
            .iter()
            .find(|m| m.automatic && m.name == "antes de restaurar cp1")
            .expect("deveria ter criado o checkpoint automático");
        let hash = automatic.files[0].content_hash.as_ref().unwrap();
        assert_eq!(
            checkpoint::read_object(dir.path(), hash).unwrap(),
            b"v1 mudado"
        );
    }

    #[test]
    fn arquivo_sem_conteudo_atual_nao_entra_no_automatico_mas_restaura_normalmente() {
        let dir = tempfile::tempdir().unwrap();
        originals::ensure_tracked(dir.path(), "a.txt", Some(b"original")).unwrap();
        let checkpoint = checkpoint::create(
            dir.path(),
            "cp1",
            Author::User,
            100,
            false,
            &[("a.txt".to_string(), b"v1".to_vec())],
        )
        .unwrap()
        .manifest;

        // `FakeTarget` vazio: nenhum conteúdo atual pra "a.txt" — sem
        // checkpoint automático nenhum (nada pra capturar), mas o
        // próprio arquivo ainda é restaurado.
        let mut target = FakeTarget(HashMap::new());
        let report = restore(dir.path(), &checkpoint, Author::User, 200, &mut target).unwrap();

        assert_eq!(report.restored, vec!["a.txt".to_string()]);
        assert!(
            checkpoint::list(dir.path())
                .unwrap()
                .iter()
                .all(|m| !m.automatic)
        );
    }
}
