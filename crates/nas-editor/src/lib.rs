//! Estado do editor: documentos, cursores e transações.
//!
//! Construído sobre `helix-core` (rope, transações, histórico), isolado
//! dentro do módulo `document` — ver docs/decisions/0001-helix-como-biblioteca.md.
//! O resto do nas só deve ver os tipos reexportados aqui, nunca
//! `helix_core` diretamente.

mod document;

pub use document::{Document, OpenError, Position};
