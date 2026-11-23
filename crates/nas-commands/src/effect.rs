//! O efeito de um comando (README, pilar 1/2) — a base de quais
//! comandos pedem aprovação antes de rodar; a regra de quais efeitos
//! pedem aprovação de verdade é por risco real, não só pela classe
//! — ver docs/decisions/0006-ia-trabalha-pelo-nas.md §5 (etapa 3d,
//! ainda não implementada aqui).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Effect {
    Pure,
    ReadFile,
    WriteBuffer,
    WriteFile,
    Shell,
    Network,
}
