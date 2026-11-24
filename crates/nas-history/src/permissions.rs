//! Política de permissões por projeto e o registro de cada decisão
//! tomada contra ela — docs/decisions/0006-ia-trabalha-pelo-nas.md §5
//! (etapa 3d). Dois arquivos, lado a lado com `journal.jsonl`:
//!
//! - `permissions.toml`: o que o usuário libera de antemão, sem
//!   perguntar — ver [`PermissionsPolicy`]/[`load`].
//! - `permissions_log.jsonl`: cada pedido de aprovação, negado ou
//!   aceito — ver [`PermissionDecisionEntry`]/[`record_decision`].
//!
//! Fica fora do projeto de propósito (dentro de `history_dir`, que
//! vive em `~/.local/state/nas/`, nunca no repositório) — nada dentro
//! do projeto pode conceder permissão a si mesmo.

use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Quais dos três efeitos que pedem aprovação (ADR 0006 §5:
/// `WriteFile`, `Shell`, `Network`) o usuário já liberou de antemão,
/// sem perguntar, para este projeto. `nas-commands` é quem sabe o
/// nome de cada efeito — aqui são só três booleanos, pra esta crate
/// não precisar depender de `nas-commands::Effect` (a direção certa é
/// a contrária).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PermissionsPolicy {
    pub write_file: bool,
    pub shell: bool,
    pub network: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct RawPolicy {
    #[serde(default)]
    write_file: bool,
    #[serde(default)]
    shell: bool,
    #[serde(default)]
    network: bool,
}

/// Por que `permissions.toml` não pôde ser lido — sempre com o
/// caminho, nunca só "inválido" (mesma ideia de configuração
/// recuperável da ADR 0007 §3). Quem recebe este erro trata como
/// "tudo negado": nunca aplica metade de um arquivo que não terminou
/// de validar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionsError {
    pub file: PathBuf,
    pub message: String,
}

impl fmt::Display for PermissionsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.file.display(), self.message)
    }
}

impl std::error::Error for PermissionsError {}

/// O caminho de `permissions.toml` dentro de `history_dir` — nunca
/// dentro do projeto (ver o módulo).
pub fn permissions_path(history_dir: &Path) -> PathBuf {
    history_dir.join("permissions.toml")
}

/// Lê e valida `permissions.toml`. Sem o arquivo (nunca criado ainda):
/// `Ok(PermissionsPolicy::default())` — tudo negado, mas não é um
/// erro, já que "nenhuma política ainda" é o estado inicial normal de
/// todo projeto. Com o arquivo presente mas inválido (TOML malformado,
/// chave desconhecida, valor que não é booleano): `Err`, com a
/// mensagem do motivo — quem chama decide tratar como "tudo negado"
/// (nunca aplica metade de um arquivo inválido).
pub fn load(history_dir: &Path) -> Result<PermissionsPolicy, PermissionsError> {
    let path = permissions_path(history_dir);
    let content = match fs::read_to_string(&path) {
        Ok(content) => content,
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            return Ok(PermissionsPolicy::default());
        }
        Err(err) => {
            return Err(PermissionsError {
                file: path,
                message: err.to_string(),
            });
        }
    };
    let raw: RawPolicy = toml::from_str(&content).map_err(|err| PermissionsError {
        file: path.clone(),
        message: err.to_string(),
    })?;
    Ok(PermissionsPolicy {
        write_file: raw.write_file,
        shell: raw.shell,
        network: raw.network,
    })
}

const TEMPLATE: &str = "\
# Política de permissões do nas para este projeto.
#
# Um agente de IA (nunca o usuário, editando direto) pede aprovação
# antes de rodar um comando com um destes três efeitos: write-file
# (grava de verdade no disco do projeto), shell, network. Qualquer
# efeito ausente ou marcado \"false\" continua pedindo aprovação — e,
# nesta etapa, sem ninguém pra perguntar, é negado direto, com um erro
# claro pro agente dizendo o que falta.
#
# Marque \"true\" pra liberar o efeito sem perguntar.

write-file = false
shell = false
network = false
";

/// Garante que `permissions.toml` exista, com permissão `0600` — cria
/// um modelo comentado com tudo negado se ainda não existir; não toca
/// no arquivo se já existir (nunca sobrescreve uma política que o
/// usuário já escreveu). Devolve o caminho nos dois casos.
pub fn ensure_template(history_dir: &Path) -> io::Result<PathBuf> {
    let path = permissions_path(history_dir);
    if path.exists() {
        return Ok(path);
    }
    fs::write(&path, TEMPLATE)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(path)
}

/// De onde veio uma aprovação — só faz sentido quando a decisão foi
/// `Allowed` (ver [`PermissionDecision`]).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PermissionSource {
    /// Liberado de antemão em `permissions.toml`.
    File,
    /// Respondida por quem está usando o cliente MCP, via elicitation
    /// (ADR 0006 §5) — ainda não implementado nesta etapa (o `rmcp`
    /// hoje fixado não suporta elicitation); o variant já existe pra
    /// quando existir.
    Question,
    /// "Sempre nesta sessão", concedida por uma pergunta anterior na
    /// mesma sessão (`agent`+`session`) — só em memória, nunca
    /// persistida; também pendente da pergunta acima.
    Session,
}

/// O resultado de um pedido de aprovação — registrado no journal
/// (`record_decision`) pra alimentar a futura etapa de revisão (ADR
/// 0006 §8).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum PermissionDecision {
    Allowed { source: PermissionSource },
    Denied,
}

/// Uma linha do registro de decisões: quem pediu, o quê, e o
/// resultado — docs/decisions/0006 §5 ("Histórico").
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PermissionDecisionEntry {
    pub ts: u64,
    pub agent: String,
    pub session: String,
    pub command: String,
    /// O nome do efeito como `permissions.toml` o escreve
    /// (`"write-file"`, `"shell"`, `"network"`) — nunca o `Debug` do
    /// enum de `nas-commands`, que esta crate não conhece.
    pub effect: String,
    pub decision: PermissionDecision,
}

/// O caminho de `permissions_log.jsonl` dentro de `history_dir`.
pub fn permission_log_path(history_dir: &Path) -> PathBuf {
    history_dir.join("permissions_log.jsonl")
}

/// Acrescenta uma decisão ao registro — sempre sob a mesma trava que
/// todo escritor do histórico já usa (`crate::lock::HistoryLock`),
/// porque `nas-term` e `nas --mcp` podem escrever no mesmo projeto ao
/// mesmo tempo (ADR 0006, revisão da etapa 3c).
pub fn record_decision(history_dir: &Path, entry: &PermissionDecisionEntry) -> io::Result<()> {
    let _lock = crate::lock::HistoryLock::acquire(history_dir)?;
    let line = serde_json::to_string(entry)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(permission_log_path(history_dir))?;
    writeln!(file, "{line}")?;
    Ok(())
}

/// Lê todo o registro de decisões — vazio se o projeto nunca pediu
/// nenhuma aprovação ainda.
pub fn read_decisions(history_dir: &Path) -> io::Result<Vec<PermissionDecisionEntry>> {
    let content = match fs::read_to_string(permission_log_path(history_dir)) {
        Ok(content) => content,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err),
    };
    content
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| {
            serde_json::from_str(line)
                .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sem_arquivo_e_tudo_negado_sem_erro() {
        let dir = tempfile::tempdir().unwrap();
        let policy = load(dir.path()).expect("ausência não é erro");
        assert_eq!(policy, PermissionsPolicy::default());
    }

    #[test]
    fn le_o_que_foi_liberado() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            permissions_path(dir.path()),
            "write-file = true\nshell = false\n",
        )
        .unwrap();

        let policy = load(dir.path()).expect("deveria validar");
        assert!(policy.write_file);
        assert!(!policy.shell);
        assert!(!policy.network);
    }

    #[test]
    fn toml_malformado_e_erro_com_motivo() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(permissions_path(dir.path()), "write-file = [\n").unwrap();

        let err = load(dir.path()).expect_err("devia recusar");
        assert!(!err.message.is_empty());
    }

    #[test]
    fn chave_desconhecida_e_erro() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(permissions_path(dir.path()), "voar = true\n").unwrap();

        let err = load(dir.path()).expect_err("devia recusar");
        assert!(err.message.to_lowercase().contains("voar") || !err.message.is_empty());
    }

    #[test]
    fn valor_nao_booleano_e_erro() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(permissions_path(dir.path()), "write-file = \"sim\"\n").unwrap();

        assert!(load(dir.path()).is_err());
    }

    #[test]
    fn ensure_template_cria_com_tudo_negado_e_permissao_0600() {
        let dir = tempfile::tempdir().unwrap();

        let path = ensure_template(dir.path()).expect("criar modelo");
        let policy = load(dir.path()).expect("modelo deveria validar");
        assert_eq!(policy, PermissionsPolicy::default());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
    }

    #[test]
    fn ensure_template_nao_sobrescreve_arquivo_existente() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(permissions_path(dir.path()), "write-file = true\n").unwrap();

        ensure_template(dir.path()).expect("não deveria falhar");

        let policy = load(dir.path()).expect("deveria validar");
        assert!(
            policy.write_file,
            "ensure_template não deveria ter sobrescrito a política existente"
        );
    }

    #[test]
    fn registra_e_le_decisoes() {
        let dir = tempfile::tempdir().unwrap();
        let allowed = PermissionDecisionEntry {
            ts: 1,
            agent: "claude-code".to_string(),
            session: "s1".to_string(),
            command: "save_file".to_string(),
            effect: "write-file".to_string(),
            decision: PermissionDecision::Allowed {
                source: PermissionSource::File,
            },
        };
        let denied = PermissionDecisionEntry {
            ts: 2,
            agent: "claude-code".to_string(),
            session: "s1".to_string(),
            command: "create_file".to_string(),
            effect: "write-file".to_string(),
            decision: PermissionDecision::Denied,
        };
        record_decision(dir.path(), &allowed).unwrap();
        record_decision(dir.path(), &denied).unwrap();

        let entries = read_decisions(dir.path()).unwrap();
        assert_eq!(entries, vec![allowed, denied]);
    }

    #[test]
    fn ler_decisoes_sem_nenhuma_e_vazio() {
        let dir = tempfile::tempdir().unwrap();
        assert!(read_decisions(dir.path()).unwrap().is_empty());
    }
}
