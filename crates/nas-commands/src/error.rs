use std::fmt;
use std::io;

/// Por que um comando falhou — nunca aplica parcialmente nem finge
/// sucesso (mesmo princípio de `nas-history`, ADR 0005 §5).
#[derive(Debug)]
pub enum CommandError {
    /// Os argumentos recebidos não bateram com o esquema do comando.
    InvalidArgs(serde_json::Error),
    /// Erro ao serializar o resultado do comando.
    Serialize(serde_json::Error),
    /// Nenhum comando registrado com este nome.
    UnknownCommand(String),
    /// `path` não tem um documento aberto neste `Workspace` — quem
    /// chama precisa de `open_file` antes.
    NotOpen(String),
    /// Falha de E/S (abrir, ler, escrever um arquivo; checkpoint).
    Io(io::Error),
    /// `edit`: o texto antigo não foi encontrado.
    NoMatch,
    /// `edit`: o texto antigo apareceu mais de uma vez — `count`
    /// ocorrências — sem um `line_range` que desambigue pra uma só.
    Ambiguous(usize),
    /// Histórico não disponível pra este `Workspace` (sem
    /// `history_dir`) — `show_diff`/`checkpoint_create`/
    /// `checkpoint_restore` precisam dele.
    HistoryUnavailable,
    /// O caminho, depois de resolvido (`..`, absoluto, links
    /// simbólicos — `Workspace::resolve`), cai fora da raiz do
    /// projeto. Nunca revela o caminho resolvido de verdade na
    /// mensagem — só o que foi pedido, pra não dar a quem pediu uma
    /// pista do layout de fora da raiz.
    OutsideRoot(String),
}

impl fmt::Display for CommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CommandError::InvalidArgs(err) => write!(f, "argumentos inválidos: {err}"),
            CommandError::Serialize(err) => write!(f, "erro ao serializar o resultado: {err}"),
            CommandError::UnknownCommand(name) => write!(f, "comando desconhecido: {name:?}"),
            CommandError::NotOpen(path) => {
                write!(f, "{path:?} não está aberto — chame open_file primeiro")
            }
            CommandError::Io(err) => write!(f, "{err}"),
            CommandError::NoMatch => write!(f, "texto antigo não encontrado"),
            CommandError::Ambiguous(count) => {
                write!(
                    f,
                    "texto antigo apareceu {count} vezes — desambigue com line_range"
                )
            }
            CommandError::HistoryUnavailable => write!(f, "histórico não disponível"),
            CommandError::OutsideRoot(path) => {
                write!(f, "{path:?} está fora da raiz do projeto")
            }
        }
    }
}

impl std::error::Error for CommandError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            CommandError::InvalidArgs(err) | CommandError::Serialize(err) => Some(err),
            CommandError::Io(err) => Some(err),
            _ => None,
        }
    }
}

impl From<io::Error> for CommandError {
    fn from(err: io::Error) -> Self {
        CommandError::Io(err)
    }
}

impl From<nas_editor::OpenError> for CommandError {
    fn from(err: nas_editor::OpenError) -> Self {
        CommandError::Io(err.into())
    }
}
