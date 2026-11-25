//! Símbolos de um arquivo-fonte (funções, structs, títulos de
//! Markdown, ...) usados por `list_symbols`/`read_symbol` de
//! `nas-commands` (docs/decisions/0006-ia-trabalha-pelo-nas.md §8,
//! etapa 3f-prep). Tree-sitter fica isolado atrás deste crate, do
//! mesmo jeito que `helix-core` fica atrás de `nas-editor` (decisão
//! 0001); nenhum outro crate do workspace importa `tree_sitter`
//! direto.
//!
//! A gramática (`tree-sitter-rust`) é **embutida no binário**,
//! compilada como código C normal via `cc`, igual a qualquer outra
//! dependência Cargo, em vez da pasta de gramáticas que o Helix
//! carrega em tempo de execução (bibliotecas dinâmicas, baixadas e
//! compiladas separadamente com `hx --grammar fetch/build`). Ver a
//! justificativa completa na ADR (§8, etapa 3f-prep): o motivo é o
//! mesmo da decisão 0001 (helix-core como dependência fixada, não
//! `helix-term`), manter o nas um binário único, reprodutível, sem
//! uma etapa de instalação/download separada nem uma superfície de
//! versão que o Cargo não controla.
//!
//! Isolado atrás deste módulo pra que o destaque de sintaxe (fora do
//! escopo desta etapa) possa reaproveitar a mesma extração de árvore
//! depois, sem duplicar a dependência do `tree-sitter` em outro
//! crate.

mod markdown;
mod rust;

use std::fmt;
use std::path::Path;

/// A categoria de um símbolo — o "tipo" que `list_symbols` devolve
/// junto do nome e do intervalo de linhas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolKind {
    Fn,
    /// Uma função dentro de um `impl` — o nome já vem qualificado
    /// como `Tipo::metodo` (`Symbol::name`), diferente de `Fn`.
    Method,
    Struct,
    Enum,
    Trait,
    Impl,
    Const,
    Mod,
    /// Um título de Markdown (`#`, `##`, ...).
    Heading,
}

impl SymbolKind {
    /// Rótulo compacto usado na listagem de `list_symbols` — uma
    /// palavra, sempre em inglês/minúsculo, pra ficar estável como
    /// chave (`"method"`, não "método").
    pub fn label(self) -> &'static str {
        match self {
            SymbolKind::Fn => "fn",
            SymbolKind::Method => "method",
            SymbolKind::Struct => "struct",
            SymbolKind::Enum => "enum",
            SymbolKind::Trait => "trait",
            SymbolKind::Impl => "impl",
            SymbolKind::Const => "const",
            SymbolKind::Mod => "mod",
            SymbolKind::Heading => "heading",
        }
    }
}

/// Um símbolo encontrado em `list_symbols`/buscado por `read_symbol`
/// — nome (já qualificado, se for um método), tipo e as duas linhas
/// (1-indexadas, inclusive) que importam: a do item em si
/// (`start_line`/`end_line`, o que `list_symbols` mostra) e a de onde
/// `read_symbol` deveria começar a devolver (`doc_start_line`),
/// incluindo qualquer doc comment/atributo imediatamente acima —
/// igual a `start_line` quando não há nenhum.
#[derive(Debug, Clone)]
pub struct Symbol {
    pub kind: SymbolKind,
    pub name: String,
    pub start_line: usize,
    pub end_line: usize,
    pub doc_start_line: usize,
}

/// Por que `list_symbols`/`read_symbol` não conseguiram processar um
/// arquivo — hoje, só uma linguagem sem suporte ainda.
#[derive(Debug)]
pub enum SyntaxError {
    /// A extensão do arquivo (ou uma descrição dela, se não houver
    /// extensão) — nunca o caminho inteiro, mesma cautela de
    /// `CommandError::OutsideRoot`.
    UnsupportedLanguage(String),
}

impl fmt::Display for SyntaxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SyntaxError::UnsupportedLanguage(ext) => {
                write!(f, "linguagem ainda não suportada: {ext:?}")
            }
        }
    }
}

impl std::error::Error for SyntaxError {}

/// Lista os símbolos de `source` — a linguagem é decidida pela
/// extensão de `path` (o conteúdo não é inspecionado pra adivinhar).
/// Hoje: `.rs` (Tree-sitter) e `.md`/`.markdown` (títulos ATX, por
/// escaneamento de texto — não precisa de uma gramática pra algo tão
/// simples quanto `# título`). Qualquer outra extensão recusa com
/// [`SyntaxError::UnsupportedLanguage`].
pub fn list_symbols(path: &Path, source: &str) -> Result<Vec<Symbol>, SyntaxError> {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some("rs") => Ok(rust::list_symbols(source)),
        Some("md") | Some("markdown") => Ok(markdown::list_symbols(source)),
        other => Err(SyntaxError::UnsupportedLanguage(
            other.unwrap_or("(sem extensão)").to_string(),
        )),
    }
}
