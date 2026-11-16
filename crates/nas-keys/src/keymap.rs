//! O keymap do nas, carregado de `keymap/default.toml` — nunca
//! espalhado pelo código Rust. Espelha a estrutura do `nas-map.el` do
//! nas-keys: `nas-bind` (bindings diretos por estado), `nas-prefix`
//! (prefixos de duas teclas) e `nas-define-dispatch` (despacho de uma
//! tecla com um "fallback" para a tecla não mapeada).

use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;

use serde::Deserialize;

use crate::key::Key;
use crate::machine::Action;

/// O keymap embutido no binário — a única fonte de verdade para
/// atalhos; ver `docs/decisions/0003-nas-keys-como-keymap.md`.
pub const DEFAULT_TOML: &str = include_str!("../keymap/default.toml");

/// Erro ao carregar um keymap: TOML malformado, tecla ou ação
/// desconhecida.
#[derive(Debug)]
pub enum KeymapError {
    Toml(toml::de::Error),
    InvalidKey(String),
    UnknownAction(String),
}

impl fmt::Display for KeymapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            KeymapError::Toml(err) => write!(f, "keymap malformado: {err}"),
            KeymapError::InvalidKey(key) => write!(f, "tecla inválida no keymap: {key:?}"),
            KeymapError::UnknownAction(name) => write!(f, "ação desconhecida no keymap: {name:?}"),
        }
    }
}

impl std::error::Error for KeymapError {}

impl From<toml::de::Error> for KeymapError {
    fn from(err: toml::de::Error) -> Self {
        KeymapError::Toml(err)
    }
}

/// O que uma tecla faz: uma ação de verdade, uma troca de estado, ou a
/// entrada num prefixo/despacho (que vai consumir a próxima tecla).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Binding {
    Action(Action),
    EnterInsert,
    EnterVisual,
    EnterSequential,
    Prefix(String),
    Dispatch(String),
}

/// Um despacho de uma tecla (`nas-define-dispatch`): algumas teclas têm
/// ação própria; qualquer outra executa `fallback` e é reprocessada como
/// uma tecla normal do estado Visual.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dispatch {
    pub bindings: HashMap<Key, Action>,
    pub fallback: Action,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Keymap {
    pub insert: HashMap<Key, Binding>,
    pub visual: HashMap<Key, Binding>,
    pub prefixes: HashMap<String, HashMap<Key, Binding>>,
    pub dispatches: HashMap<String, Dispatch>,
}

impl Keymap {
    pub fn from_toml_str(source: &str) -> Result<Self, KeymapError> {
        let raw: RawKeymap = toml::from_str(source)?;
        raw.resolve()
    }

    /// O keymap padrão embutido (`keymap/default.toml`).
    pub fn default_keymap() -> Self {
        Keymap::from_toml_str(DEFAULT_TOML).expect("keymap/default.toml deveria ser válido")
    }
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawBinding {
    Named(String),
    Prefix { prefix: String },
    Dispatch { dispatch: String },
}

#[derive(Debug, Default, Deserialize)]
struct RawKeymap {
    #[serde(default)]
    insert: HashMap<String, RawBinding>,
    #[serde(default)]
    visual: HashMap<String, RawBinding>,
    /// Mesma forma de `insert`/`visual` — um prefixo pode, em princípio,
    /// levar a uma troca de estado (como o líder leva ao Sequential), não
    /// só a uma ação direta.
    #[serde(default)]
    prefixes: HashMap<String, HashMap<String, RawBinding>>,
    #[serde(default)]
    dispatches: HashMap<String, RawDispatch>,
}

#[derive(Debug, Deserialize)]
struct RawDispatch {
    fallback: String,
    #[serde(flatten)]
    bindings: HashMap<String, String>,
}

impl RawKeymap {
    fn resolve(self) -> Result<Keymap, KeymapError> {
        Ok(Keymap {
            insert: resolve_bindings(self.insert)?,
            visual: resolve_bindings(self.visual)?,
            prefixes: self
                .prefixes
                .into_iter()
                .map(|(name, bindings)| Ok((name, resolve_bindings(bindings)?)))
                .collect::<Result<_, KeymapError>>()?,
            dispatches: self
                .dispatches
                .into_iter()
                .map(|(name, raw)| {
                    Ok((
                        name,
                        Dispatch {
                            bindings: resolve_actions(raw.bindings)?,
                            fallback: parse_action(&raw.fallback)?,
                        },
                    ))
                })
                .collect::<Result<_, KeymapError>>()?,
        })
    }
}

fn resolve_bindings(
    raw: HashMap<String, RawBinding>,
) -> Result<HashMap<Key, Binding>, KeymapError> {
    raw.into_iter()
        .map(|(key_spec, raw_binding)| {
            let key = parse_key(&key_spec)?;
            let binding = match raw_binding {
                RawBinding::Prefix { prefix } => Binding::Prefix(prefix),
                RawBinding::Dispatch { dispatch } => Binding::Dispatch(dispatch),
                RawBinding::Named(name) => match name.as_str() {
                    "enter-insert" => Binding::EnterInsert,
                    "enter-visual" => Binding::EnterVisual,
                    "enter-sequential" => Binding::EnterSequential,
                    _ => Binding::Action(parse_action(&name)?),
                },
            };
            Ok((key, binding))
        })
        .collect()
}

fn resolve_actions(raw: HashMap<String, String>) -> Result<HashMap<Key, Action>, KeymapError> {
    raw.into_iter()
        .map(|(key_spec, action_name)| {
            let key = parse_key(&key_spec)?;
            let action = parse_action(&action_name)?;
            Ok((key, action))
        })
        .collect()
}

fn parse_key(spec: &str) -> Result<Key, KeymapError> {
    Key::from_str(spec).map_err(|_| KeymapError::InvalidKey(spec.to_string()))
}

fn parse_action(name: &str) -> Result<Action, KeymapError> {
    Ok(match name {
        "move-left" => Action::MoveLeft,
        "move-right" => Action::MoveRight,
        "move-up" => Action::MoveUp,
        "move-down" => Action::MoveDown,
        "move-line-start" => Action::MoveLineStart,
        "move-line-end" => Action::MoveLineEnd,
        "move-buffer-start" => Action::MoveBufferStart,
        "move-buffer-end" => Action::MoveBufferEnd,
        "delete-forward" => Action::DeleteForward,
        "delete-backward" => Action::DeleteBackward,
        "undo" => Action::Undo,
        "prompt-open-file" => Action::PromptOpenFile,
        "prompt-search" => Action::PromptSearch,
        "start-selection" => Action::StartSelection,
        "select-all" => Action::SelectAll,
        "copy" => Action::Copy,
        "cut" => Action::Cut,
        "paste" => Action::Paste,
        "delete-to-line-end" => Action::DeleteToLineEnd,
        "delete-to-line-start" => Action::DeleteToLineStart,
        "delete-word-forward" => Action::DeleteWordForward,
        "erase-buffer" => Action::EraseBuffer,
        "move-word-backward" => Action::MoveWordBackward,
        "move-word-forward" => Action::MoveWordForward,
        "move-paragraph-forward" => Action::MoveParagraphForward,
        "move-paragraph-backward" => Action::MoveParagraphBackward,
        "prompt-goto-line" => Action::PromptGotoLine,
        "prompt-switch-buffer" => Action::PromptSwitchBuffer,
        _ => return Err(KeymapError::UnknownAction(name.to_string())),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn carrega_o_keymap_padrao_embutido() {
        let keymap = Keymap::default_keymap();
        assert_eq!(
            keymap.visual.get(&Key::char('j')),
            Some(&Binding::Action(Action::MoveLeft))
        );
        assert_eq!(
            keymap.visual.get(&Key::char('d')),
            Some(&Binding::Dispatch("delete".to_string()))
        );
        assert_eq!(
            keymap.visual.get(&Key::char('g')),
            Some(&Binding::Prefix("go".to_string()))
        );

        let go = keymap.prefixes.get("go").expect("prefixo go");
        assert_eq!(
            go.get(&Key::char('g')),
            Some(&Binding::Action(Action::MoveBufferStart))
        );

        let leader = keymap.prefixes.get("leader").expect("prefixo leader");
        assert_eq!(leader.get(&Key::char(' ')), Some(&Binding::EnterSequential));

        let delete = keymap.dispatches.get("delete").expect("despacho delete");
        assert_eq!(delete.fallback, Action::DeleteForward);
        assert_eq!(
            delete.bindings.get(&Key::char('j')),
            Some(&Action::DeleteBackward)
        );
    }

    #[test]
    fn rejeita_tecla_invalida() {
        let err = Keymap::from_toml_str(
            r#"[visual]
"M-"  = "move-left"
"#,
        )
        .unwrap_err();
        assert!(matches!(err, KeymapError::InvalidKey(_)));
    }

    #[test]
    fn rejeita_acao_desconhecida() {
        let err = Keymap::from_toml_str(
            r#"[visual]
j = "voar"
"#,
        )
        .unwrap_err();
        assert!(matches!(err, KeymapError::UnknownAction(_)));
    }
}
