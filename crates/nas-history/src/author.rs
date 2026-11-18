//! Quem fez uma mudança — docs/decisions/0005-historico-com-autoria.md §1.

use serde::{Deserialize, Serialize};

/// Quem é responsável por uma transação journalada. A IA de verdade só
/// chega na fase 3 do roadmap; até lá, `Ai` só aparece em teste, com um
/// autor simulado — o campo já nasce pronto pra isso.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Author {
    /// A pessoa editando direto.
    User,
    /// Um agente de IA. `agent` nomeia quem está agindo (`"tia"`,
    /// `"claude-code"`); `session` identifica a sessão ou tarefa, pra
    /// distinguir duas rodadas do mesmo agente.
    Ai { agent: String, session: String },
    /// Uma extensão ou plugin (um formatador, uma code action de LSP).
    Extension { name: String },
    /// Uma mudança detectada no arquivo que não veio de nenhuma das
    /// opções acima — alguém rodou `git checkout`, um formatador rodou
    /// fora do nas, outro programa editou o mesmo arquivo. A detecção
    /// em si (quando e como o nas percebe isso) é da etapa 2d; este
    /// variant só precisa existir pra que o journal saiba nomear o
    /// resultado.
    External,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializa_e_desserializa_cada_variante() {
        let authors = [
            Author::User,
            Author::Ai {
                agent: "claude-code".to_string(),
                session: "tarefa-42".to_string(),
            },
            Author::Extension {
                name: "rustfmt".to_string(),
            },
            Author::External,
        ];
        for author in authors {
            let json = serde_json::to_string(&author).expect("serializa");
            let voltou: Author = serde_json::from_str(&json).expect("desserializa");
            assert_eq!(voltou, author);
        }
    }

    #[test]
    fn formato_json_tem_o_campo_kind() {
        assert_eq!(
            serde_json::to_string(&Author::User).unwrap(),
            r#"{"kind":"user"}"#
        );
        assert_eq!(
            serde_json::to_string(&Author::Ai {
                agent: "tia".to_string(),
                session: "s1".to_string()
            })
            .unwrap(),
            r#"{"kind":"ai","agent":"tia","session":"s1"}"#
        );
        assert_eq!(
            serde_json::to_string(&Author::External).unwrap(),
            r#"{"kind":"external"}"#
        );
    }
}
