//! Registro de comandos tipados — nome, argumentos, descrição e
//! efeito (pilar 1 do README), de onde sai, automaticamente, tanto a
//! validação quanto o esquema JSON que um agente de IA recebe como
//! ferramenta — docs/decisions/0006-ia-trabalha-pelo-nas.md §1.
//!
//! Não depende de `nas-keys` nem sabe de teclas — só dos tipos de
//! domínio (`nas-editor::Document`, `nas-history`), do mesmo jeito
//! que `nas-history` não depende de `nas-editor` nem de `nas-keys`
//! (ADR 0005 §9). Esta é a etapa 3a: os comandos chamam `Document`/
//! `nas-history` direto, em processo — sem núcleo em segundo plano
//! nem servidor MCP ainda (3b/3c).

mod commands;
mod context;
mod effect;
mod error;
mod registry;

pub use commands::register_all;
pub use context::{CommandContext, Workspace};
pub use effect::Effect;
pub use error::CommandError;
pub use registry::{CommandSpec, Registry, typed_command};
