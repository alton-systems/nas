use std::fmt;
use std::path::PathBuf;

/// Um erro de configuração, sempre com onde ele aconteceu — nunca só
/// "chave inválida" sem dizer em qual arquivo e onde (ADR 0007 §3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigError {
    pub file: PathBuf,
    /// 1-indexada; `None` quando o erro não é sobre uma posição
    /// específica no arquivo (ex.: o arquivo inteiro não é TOML
    /// válido e o parser não localizou onde).
    pub line: Option<usize>,
    pub column: Option<usize>,
    pub message: String,
    /// Um nome parecido, quando a chave desconhecida lembra uma
    /// válida — ver `crate::suggest`.
    pub suggestion: Option<String>,
}

impl ConfigError {
    pub fn new(file: impl Into<PathBuf>, message: impl Into<String>) -> Self {
        ConfigError {
            file: file.into(),
            line: None,
            column: None,
            message: message.into(),
            suggestion: None,
        }
    }

    pub fn at(
        file: impl Into<PathBuf>,
        line: usize,
        column: usize,
        message: impl Into<String>,
    ) -> Self {
        ConfigError {
            file: file.into(),
            line: Some(line),
            column: Some(column),
            message: message.into(),
            suggestion: None,
        }
    }

    pub fn with_suggestion(mut self, suggestion: impl Into<String>) -> Self {
        self.suggestion = Some(suggestion.into());
        self
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.file.display())?;
        if let (Some(line), Some(column)) = (self.line, self.column) {
            write!(f, ":{line}:{column}")?;
        }
        write!(f, ": {}", self.message)?;
        if let Some(suggestion) = &self.suggestion {
            write!(f, " (você quis dizer {suggestion:?}?)")?;
        }
        Ok(())
    }
}

impl std::error::Error for ConfigError {}

/// Formata uma lista de erros, um por linha — o que a echo area e
/// `nas --check-config` mostram (ADR 0007 §3).
pub fn format_errors(errors: &[ConfigError]) -> String {
    errors
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n")
}

/// Converte um offset de byte em `source` pra linha/coluna
/// 1-indexadas — `toml_edit` dá spans em bytes; echo area e terminal
/// querem linha/coluna.
pub(crate) fn line_col(source: &str, byte_offset: usize) -> (usize, usize) {
    let mut line = 1;
    let mut column = 1;
    for (i, ch) in source.char_indices() {
        if i >= byte_offset {
            break;
        }
        if ch == '\n' {
            line += 1;
            column = 1;
        } else {
            column += 1;
        }
    }
    (line, column)
}
