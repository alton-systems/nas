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
    /// Escreve no armazenamento do histórico (`~/.local/state/nas/`)
    /// — nunca no arquivo do projeto em si. Separado de `WriteFile`
    /// porque o risco é outro: `checkpoint_create` só acrescenta um
    /// snapshot ao histórico, sempre reversível (não existe "apagar
    /// um checkpoint" ainda) — liberado por padrão, ADR 0006 §5.
    WriteHistory,
    WriteFile,
    Shell,
    Network,
}
