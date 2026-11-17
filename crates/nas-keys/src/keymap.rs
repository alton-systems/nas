//! O keymap do nas, carregado de `keymap/default.toml` — nunca
//! espalhado pelo código Rust. A especificação completa (a ideia dos
//! radicais e compostos, as regras, as classes, a tabela de teclas)
//! vive em `docs/keys.md`; este módulo só sabe interpretar o formato
//! TOML que a implementa.

use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;

use serde::Deserialize;

use crate::key::Key;
use crate::machine::{Action, Object, Unit, Verb};

/// O keymap embutido no binário — a única fonte de verdade para
/// atalhos; ver `docs/decisions/0004-especificacao-de-teclas-no-nas.md`.
pub const DEFAULT_TOML: &str = include_str!("../keymap/default.toml");

/// Erro ao carregar um keymap: TOML malformado, tecla, ação ou objeto
/// desconhecido.
#[derive(Debug)]
pub enum KeymapError {
    Toml(toml::de::Error),
    InvalidKey(String),
    UnknownAction(String),
    UnknownObject(String),
}

impl fmt::Display for KeymapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            KeymapError::Toml(err) => write!(f, "keymap malformado: {err}"),
            KeymapError::InvalidKey(key) => write!(f, "tecla inválida no keymap: {key:?}"),
            KeymapError::UnknownAction(name) => write!(f, "ação desconhecida no keymap: {name:?}"),
            KeymapError::UnknownObject(name) => {
                write!(f, "objeto desconhecido no keymap: {name:?}")
            }
        }
    }
}

impl std::error::Error for KeymapError {}

impl From<toml::de::Error> for KeymapError {
    fn from(err: toml::de::Error) -> Self {
        KeymapError::Toml(err)
    }
}

/// O que uma tecla faz em `[insert]`/`[visual]`/`[prefixes.*]`: uma
/// ação de verdade, uma troca de estado, a entrada num prefixo (que
/// consome mais uma tecla), num verbo (que espera um objeto) ou numa
/// unidade (que espera uma direção), ou repetir o último composto que
/// alterou o texto.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Binding {
    Action(Action),
    EnterInsert,
    EnterVisual,
    EnterSequential,
    Prefix(String),
    Verb(Verb),
    Unit(Unit),
    Repeat,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Keymap {
    pub insert: HashMap<Key, Binding>,
    pub visual: HashMap<Key, Binding>,
    pub prefixes: HashMap<String, HashMap<Key, Binding>>,
    objects_horizontal: HashMap<Key, Object>,
    objects_vertical: HashMap<Key, Object>,
    objects_word: HashMap<Key, Object>,
    objects_paragraph: HashMap<Key, Object>,
    objects_whole: HashMap<Key, Object>,
    commands: HashMap<String, String>,
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

    pub(crate) fn objects_horizontal(&self) -> &HashMap<Key, Object> {
        &self.objects_horizontal
    }

    pub(crate) fn objects_vertical(&self) -> &HashMap<Key, Object> {
        &self.objects_vertical
    }

    pub(crate) fn objects_whole(&self) -> &HashMap<Key, Object> {
        &self.objects_whole
    }

    pub(crate) fn objects_for_unit(&self, unit: Unit) -> &HashMap<Key, Object> {
        match unit {
            Unit::Word => &self.objects_word,
            Unit::Paragraph => &self.objects_paragraph,
        }
    }

    /// Descrição curta de um nome de comando (ação, verbo, unidade,
    /// prefixo ou objeto) — usada nas dicas da barra de status. O
    /// próprio nome, se não houver descrição em `[commands]`.
    pub(crate) fn describe(&self, name: &str) -> String {
        self.commands
            .get(name)
            .cloned()
            .unwrap_or_else(|| name.to_string())
    }

    /// As teclas válidas e suas descrições para um prefixo pendente
    /// (`g`, líder) — para a dica da barra de status.
    pub(crate) fn hint_for_prefix(&self, name: &str) -> Vec<(Key, String)> {
        let Some(bindings) = self.prefixes.get(name) else {
            return Vec::new();
        };
        bindings
            .iter()
            .map(|(key, binding)| (*key, self.describe_binding(binding)))
            .collect()
    }

    /// As teclas válidas e suas descrições para uma unidade pendente
    /// (`w`, `p`, sozinha ou depois de um verbo).
    pub(crate) fn hint_for_unit(&self, unit: Unit) -> Vec<(Key, String)> {
        self.objects_for_unit(unit)
            .iter()
            .map(|(key, object)| (*key, self.describe(object.command_name())))
            .collect()
    }

    /// As teclas válidas e suas descrições para um verbo pendente (`d`,
    /// `c`, `x`) — os objetos horizontais, verticais e o arquivo
    /// inteiro, mais as unidades (que continuam pendentes, esperando
    /// uma direção).
    pub(crate) fn hint_for_verb(&self) -> Vec<(Key, String)> {
        self.objects_horizontal
            .iter()
            .chain(self.objects_vertical.iter())
            .chain(self.objects_whole.iter())
            .map(|(key, object)| (*key, self.describe(object.command_name())))
            .chain(
                self.visual
                    .iter()
                    .filter_map(|(key, binding)| match binding {
                        Binding::Unit(unit) => Some((*key, self.describe(unit.command_name()))),
                        _ => None,
                    }),
            )
            .collect()
    }

    fn describe_binding(&self, binding: &Binding) -> String {
        match binding {
            Binding::Action(action) => self.describe(action.command_name().unwrap_or("?")),
            Binding::EnterInsert => self.describe("enter-insert"),
            Binding::EnterVisual => self.describe("enter-visual"),
            Binding::EnterSequential => self.describe("enter-sequential"),
            Binding::Prefix(name) => self.describe(name),
            Binding::Verb(verb) => self.describe(verb.command_name()),
            Binding::Unit(unit) => self.describe(unit.command_name()),
            Binding::Repeat => self.describe("repeat"),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawBinding {
    Named(String),
    Prefix { prefix: String },
    Verb { verb: String },
    Unit { unit: String },
}

#[derive(Debug, Default, Deserialize)]
struct RawKeymap {
    #[serde(default)]
    insert: HashMap<String, RawBinding>,
    #[serde(default)]
    visual: HashMap<String, RawBinding>,
    #[serde(default)]
    prefixes: HashMap<String, HashMap<String, RawBinding>>,
    #[serde(default)]
    objects: RawObjects,
    #[serde(default)]
    commands: HashMap<String, String>,
}

#[derive(Debug, Default, Deserialize)]
struct RawObjects {
    #[serde(default)]
    horizontal: HashMap<String, String>,
    #[serde(default)]
    vertical: HashMap<String, String>,
    #[serde(default)]
    word: HashMap<String, String>,
    #[serde(default)]
    paragraph: HashMap<String, String>,
    #[serde(default)]
    whole: HashMap<String, String>,
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
            objects_horizontal: resolve_objects(self.objects.horizontal)?,
            objects_vertical: resolve_objects(self.objects.vertical)?,
            objects_word: resolve_objects(self.objects.word)?,
            objects_paragraph: resolve_objects(self.objects.paragraph)?,
            objects_whole: resolve_objects(self.objects.whole)?,
            commands: self.commands,
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
                RawBinding::Verb { verb } => Binding::Verb(parse_verb(&verb)?),
                RawBinding::Unit { unit } => Binding::Unit(parse_unit(&unit)?),
                RawBinding::Named(name) => match name.as_str() {
                    "enter-insert" => Binding::EnterInsert,
                    "enter-visual" => Binding::EnterVisual,
                    "enter-sequential" => Binding::EnterSequential,
                    "repeat" => Binding::Repeat,
                    _ => Binding::Action(parse_action(&name)?),
                },
            };
            Ok((key, binding))
        })
        .collect()
}

fn resolve_objects(raw: HashMap<String, String>) -> Result<HashMap<Key, Object>, KeymapError> {
    raw.into_iter()
        .map(|(key_spec, object_name)| {
            let key = parse_key(&key_spec)?;
            let object = parse_object(&object_name)?;
            Ok((key, object))
        })
        .collect()
}

fn parse_key(spec: &str) -> Result<Key, KeymapError> {
    Key::from_str(spec).map_err(|_| KeymapError::InvalidKey(spec.to_string()))
}

fn parse_verb(name: &str) -> Result<Verb, KeymapError> {
    Ok(match name {
        "delete" => Verb::Delete,
        "copy" => Verb::Copy,
        "cut" => Verb::Cut,
        _ => return Err(KeymapError::UnknownAction(name.to_string())),
    })
}

fn parse_unit(name: &str) -> Result<Unit, KeymapError> {
    Ok(match name {
        "word" => Unit::Word,
        "paragraph" => Unit::Paragraph,
        _ => return Err(KeymapError::UnknownAction(name.to_string())),
    })
}

fn parse_object(name: &str) -> Result<Object, KeymapError> {
    Ok(match name {
        "char-left" => Object::CharLeft,
        "char-right" => Object::CharRight,
        "line-start" => Object::LineStart,
        "line-end" => Object::LineEnd,
        "line-down" => Object::LineDown,
        "line-up" => Object::LineUp,
        "buffer-end" => Object::BufferEnd,
        "buffer-start" => Object::BufferStart,
        "word-backward" => Object::WordBackward,
        "word-forward" => Object::WordForward,
        "paragraph-forward" => Object::ParagraphForward,
        "paragraph-backward" => Object::ParagraphBackward,
        "whole-buffer" => Object::WholeBuffer,
        _ => return Err(KeymapError::UnknownObject(name.to_string())),
    })
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
        "undo" => Action::Undo,
        "prompt-search" => Action::PromptSearch,
        "prompt-goto-line" => Action::PromptGotoLine,
        "start-selection" => Action::StartSelection,
        "select-all" => Action::SelectAll,
        "paste" => Action::Paste,
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
            Some(&Binding::Verb(Verb::Delete))
        );
        assert_eq!(
            keymap.visual.get(&Key::char('w')),
            Some(&Binding::Unit(Unit::Word))
        );
        assert_eq!(
            keymap.visual.get(&Key::char('g')),
            Some(&Binding::Prefix("go".to_string()))
        );
        assert_eq!(keymap.visual.get(&Key::char('.')), Some(&Binding::Repeat));

        let go = keymap.prefixes.get("go").expect("prefixo go");
        assert_eq!(
            go.get(&Key::char('g')),
            Some(&Binding::Action(Action::MoveBufferStart))
        );

        let leader = keymap.prefixes.get("leader").expect("prefixo leader");
        assert_eq!(leader.get(&Key::char(' ')), Some(&Binding::EnterSequential));

        assert_eq!(
            keymap.objects_horizontal().get(&Key::char(';')),
            Some(&Object::CharRight)
        );
        assert_eq!(
            keymap.objects_vertical().get(&Key::char('k')),
            Some(&Object::LineDown)
        );
        assert_eq!(
            keymap.objects_whole().get(&Key::char('a')),
            Some(&Object::WholeBuffer)
        );
        assert_eq!(
            keymap.objects_for_unit(Unit::Word).get(&Key::char(';')),
            Some(&Object::WordForward)
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

    #[test]
    fn rejeita_objeto_desconhecido() {
        let err = Keymap::from_toml_str(
            r#"[objects.horizontal]
j = "voar"
"#,
        )
        .unwrap_err();
        assert!(matches!(err, KeymapError::UnknownObject(_)));
    }

    #[test]
    fn describe_cai_no_proprio_nome_sem_descricao() {
        let keymap = Keymap::default_keymap();
        assert_eq!(keymap.describe("inexistente"), "inexistente");
        assert_eq!(keymap.describe("delete"), "apagar");
    }
}
