use std::fmt;
use std::io;

use crate::effect::Effect;

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
    /// `save_file`: o arquivo mudou no disco desde a última leitura
    /// — nunca sobrescreve um trabalho que não é deste `Workspace`
    /// (ADR 0006, revisão da etapa 3c). Quem chama precisa reabrir
    /// com `open_file` antes de tentar salvar de novo.
    DiskChanged(String),
    /// O comando declara um efeito que pede aprovação (ADR 0006 §5:
    /// `WriteFile`, `Shell`, `Network`), pedido por um autor que não é
    /// `Author::User`, e o efeito não está entre os já aprovados desta
    /// chamada
    /// (`CommandContext::approved_effects`) — quem monta o contexto
    /// (`nas-mcp`) já devia ter recusado antes de chegar aqui; este é
    /// só o cinto de segurança do `Registry` em si (etapa 3d).
    NeedsApproval { command: String, effect: Effect },
    /// `list_symbols`/`read_symbol`: a extensão do arquivo não tem
    /// suporte ainda (ADR 0006 §8, etapa 3f-prep) — hoje, só `.rs` e
    /// `.md`/`.markdown`.
    UnsupportedLanguage(String),
    /// `read_symbol`: nenhum símbolo bateu com o nome pedido.
    SymbolNotFound(String),
    /// `read_symbol`: o nome pedido bateu com mais de um símbolo —
    /// cada item já formatado como `"nome (linha N)"`, pra
    /// desambiguar (ex.: com o nome qualificado `Tipo::metodo`).
    AmbiguousSymbol(Vec<String>),
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
            CommandError::DiskChanged(path) => {
                write!(
                    f,
                    "{path:?} mudou no disco desde a última leitura — reabra com open_file antes de salvar"
                )
            }
            CommandError::NeedsApproval { command, effect } => write!(
                f,
                "{command:?} tem efeito {effect:?}, que pede aprovação e não foi liberado — rode nas --permissions neste projeto pra ver ou liberar o que falta"
            ),
            CommandError::UnsupportedLanguage(ext) => {
                write!(f, "linguagem ainda não suportada: {ext:?}")
            }
            CommandError::SymbolNotFound(name) => {
                write!(f, "nenhum símbolo chamado {name:?} encontrado")
            }
            CommandError::AmbiguousSymbol(candidates) => {
                write!(f, "nome ambíguo — candidatos: {}", candidates.join(", "))
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

impl From<nas_syntax::SyntaxError> for CommandError {
    fn from(err: nas_syntax::SyntaxError) -> Self {
        match err {
            nas_syntax::SyntaxError::UnsupportedLanguage(ext) => {
                CommandError::UnsupportedLanguage(ext)
            }
        }
    }
}
