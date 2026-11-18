//! Identificação do projeto e a pasta onde o histórico dele fica —
//! docs/decisions/0005-historico-com-autoria.md §5 e §6.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Um projeto, identificado pela raiz do repositório Git (se o
/// diretório onde o nas foi iniciado estiver dentro de um) ou pelo
/// próprio diretório, caso contrário — nunca escreve nem roda `git`,
/// só lê se um `.git` existe subindo os diretórios (§6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectId {
    root: PathBuf,
    hash: String,
}

impl ProjectId {
    /// Identifica o projeto a partir de `start` (o diretório onde o
    /// nas foi iniciado). `start` precisa existir — é canonicalizado
    /// antes de procurar a raiz.
    pub fn discover(start: &Path) -> io::Result<Self> {
        let canonical = start.canonicalize()?;
        let root = find_git_root(&canonical).unwrap_or(canonical);
        let hash = hash_path(&root);
        Ok(Self { root, hash })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Os 16 primeiros hex do SHA-256 do caminho raiz — o nome da
    /// pasta do histórico deste projeto.
    pub fn hash(&self) -> &str {
        &self.hash
    }

    /// Garante que `<state_home>/nas/history/<hash>/` existe, com
    /// permissão `0700` (ela e a pasta `history/` por cima), e que
    /// `project.toml` guarda o caminho de verdade — cria os dois na
    /// primeira vez, não faz nada na próxima. `state_home` é
    /// `$XDG_STATE_HOME`, ou `~/.local/state` sem a variável — ver
    /// [`state_home`].
    pub fn ensure_history_dir(&self, state_home: &Path) -> io::Result<PathBuf> {
        let history_root = state_home.join("nas").join("history");
        create_dir_private(&history_root)?;

        let dir = history_root.join(&self.hash);
        let is_new = !dir.exists();
        create_dir_private(&dir)?;

        if is_new {
            let manifest = ProjectManifest {
                path: self.root.display().to_string(),
            };
            let toml = toml::to_string_pretty(&manifest)
                .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
            fs::write(dir.join("project.toml"), toml)?;
        }

        Ok(dir)
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct ProjectManifest {
    path: String,
}

/// Remove o histórico inteiro de um projeto — `journal.jsonl` e todos
/// os checkpoints, nomeados ou não (`<hash-do-projeto>/` por
/// completo). Não há checkpoint de segurança pra esta ação (apagar o
/// próprio histórico); a confirmação explícita é a única rede —
/// `confirmation` precisa ser exatamente `"apagar"`, senão nada é
/// removido. docs/decisions/0005 §5.
pub fn delete_project_history(history_dir: &Path, confirmation: &str) -> io::Result<()> {
    if confirmation != "apagar" {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "confirmação incorreta: digite exatamente \"apagar\"",
        ));
    }
    match fs::remove_dir_all(history_dir) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err),
    }
}

/// `$XDG_STATE_HOME`, ou `~/.local/state` se a variável não estiver
/// definida (ou estiver vazia) — mesmo padrão de fallback manual que
/// `NAS_KEYBOARD_LAYOUT` já usa pra variável de ambiente, sem
/// depender de uma crate de diretórios.
pub fn state_home() -> io::Result<PathBuf> {
    if let Some(dir) = std::env::var_os("XDG_STATE_HOME")
        && !dir.is_empty()
    {
        return Ok(PathBuf::from(dir));
    }
    let home = std::env::var_os("HOME").ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "variável de ambiente HOME não definida",
        )
    })?;
    Ok(PathBuf::from(home).join(".local/state"))
}

fn find_git_root(start: &Path) -> Option<PathBuf> {
    let mut dir = start;
    loop {
        if dir.join(".git").exists() {
            return Some(dir.to_path_buf());
        }
        dir = dir.parent()?;
    }
}

fn hash_path(path: &Path) -> String {
    let mut hasher = Sha256::new();
    hasher.update(path.to_string_lossy().as_bytes());
    let digest = hasher.finalize();
    digest.iter().take(8).map(|b| format!("{b:02x}")).collect()
}

/// Cria `dir` (e os pais necessários) se não existir, com permissão
/// `0700` — só o dono lê/escreve/entra. Não faz nada se `dir` já
/// existir (não reaplica a permissão a uma pasta já existente, pra
/// não sobrescrever uma permissão que o usuário tenha mudado à mão).
fn create_dir_private(dir: &Path) -> io::Result<()> {
    if dir.exists() {
        return Ok(());
    }
    fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descobre_a_raiz_do_git_subindo_diretorios() {
        let base = tempfile::tempdir().expect("diretório temporário");
        let git_dir = base.path().join(".git");
        fs::create_dir(&git_dir).unwrap();
        let nested = base.path().join("src").join("deep");
        fs::create_dir_all(&nested).unwrap();

        let project = ProjectId::discover(&nested).expect("descobrir projeto");

        assert_eq!(project.root(), base.path().canonicalize().unwrap());
    }

    #[test]
    fn sem_git_usa_o_proprio_diretorio() {
        let base = tempfile::tempdir().expect("diretório temporário");

        let project = ProjectId::discover(base.path()).expect("descobrir projeto");

        assert_eq!(project.root(), base.path().canonicalize().unwrap());
    }

    #[test]
    fn caminhos_diferentes_tem_hashes_diferentes() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();

        let project_a = ProjectId::discover(a.path()).unwrap();
        let project_b = ProjectId::discover(b.path()).unwrap();

        assert_ne!(project_a.hash(), project_b.hash());
    }

    #[test]
    fn o_mesmo_caminho_sempre_da_o_mesmo_hash() {
        let dir = tempfile::tempdir().unwrap();

        let first = ProjectId::discover(dir.path()).unwrap();
        let second = ProjectId::discover(dir.path()).unwrap();

        assert_eq!(first.hash(), second.hash());
    }

    #[test]
    #[cfg(unix)]
    fn ensure_history_dir_cria_com_permissao_0700_e_grava_project_toml() {
        use std::os::unix::fs::PermissionsExt;

        let project_dir = tempfile::tempdir().unwrap();
        let state_home = tempfile::tempdir().unwrap();
        let project = ProjectId::discover(project_dir.path()).unwrap();

        let history_dir = project.ensure_history_dir(state_home.path()).unwrap();

        let mode = fs::metadata(&history_dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);

        let history_root_mode = fs::metadata(history_dir.parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(history_root_mode, 0o700);

        let manifest = fs::read_to_string(history_dir.join("project.toml")).unwrap();
        assert!(
            manifest.contains(
                &project_dir
                    .path()
                    .canonicalize()
                    .unwrap()
                    .display()
                    .to_string()
            )
        );
    }

    #[test]
    fn apagar_historico_exige_a_confirmacao_exata() {
        let state_home = tempfile::tempdir().unwrap();
        let project_dir = tempfile::tempdir().unwrap();
        let project = ProjectId::discover(project_dir.path()).unwrap();
        let history_dir = project.ensure_history_dir(state_home.path()).unwrap();

        let err = delete_project_history(&history_dir, "sim, apagar").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        assert!(history_dir.exists(), "não deveria ter apagado nada");
    }

    #[test]
    fn apagar_historico_remove_a_pasta_inteira_com_a_confirmacao_certa() {
        let state_home = tempfile::tempdir().unwrap();
        let project_dir = tempfile::tempdir().unwrap();
        let project = ProjectId::discover(project_dir.path()).unwrap();
        let history_dir = project.ensure_history_dir(state_home.path()).unwrap();
        fs::write(history_dir.join("journal.jsonl"), "").unwrap();

        delete_project_history(&history_dir, "apagar").unwrap();

        assert!(!history_dir.exists());
    }

    #[test]
    fn apagar_historico_inexistente_nao_e_erro() {
        let dir = tempfile::tempdir().unwrap();
        let nunca_criado = dir.path().join("nunca-existiu");

        delete_project_history(&nunca_criado, "apagar").expect("não deveria falhar");
    }

    #[test]
    fn ensure_history_dir_e_idempotente() {
        let project_dir = tempfile::tempdir().unwrap();
        let state_home = tempfile::tempdir().unwrap();
        let project = ProjectId::discover(project_dir.path()).unwrap();

        let first = project.ensure_history_dir(state_home.path()).unwrap();
        let second = project.ensure_history_dir(state_home.path()).unwrap();

        assert_eq!(first, second);
    }
}
