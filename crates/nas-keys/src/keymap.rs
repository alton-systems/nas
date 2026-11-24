//! O keymap do nas, carregado de `keymap/default.toml` — nunca
//! espalhado pelo código Rust. A especificação completa (a ideia dos
//! radicais e compostos, as regras, as classes, a tabela de teclas)
//! vive em `docs/keys.md`; este módulo só sabe interpretar o formato
//! TOML que a implementa.

use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;

use indexmap::IndexMap;
use serde::Deserialize;

use crate::key::Key;
use crate::machine::{Action, CatalogEntry, Object, SystemAction, Unit, Verb, plain_motion_for};

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
    /// Derivados do próprio `visual` (via
    /// `Action::as_horizontal_object`/`as_vertical_object`), não lidos
    /// de uma tabela `[objects.horizontal]`/`[objects.vertical]`
    /// separada — mudar a tecla de um movimento muda o objeto junto,
    /// sem precisar editar dois lugares. Ver `derive_movement_objects`.
    objects_horizontal: HashMap<Key, Object>,
    objects_vertical: HashMap<Key, Object>,
    objects_word: HashMap<Key, Object>,
    objects_paragraph: HashMap<Key, Object>,
    objects_whole: HashMap<Key, Object>,
    commands: HashMap<String, String>,
    system: HashMap<Key, SystemAction>,
    /// Teclas de `[system]` que entram num prefixo nomeado (`Ctrl+t`
    /// → `"tempo"`) em vez de resolver direto pra uma `SystemAction` —
    /// a tecla seguinte (sem Ctrl) é quem resolve, em
    /// `system_prefixes`.
    system_prefix_keys: HashMap<Key, String>,
    /// `[system_prefixes.<nome>]`: a tecla (sem Ctrl) que completa um
    /// prefixo de sistema pendente — hoje só `"tempo"`
    /// (docs/keys.md §5.7, prefixo `Ctrl+t`). Um `Vec`, não um
    /// `HashMap`, porque a **ordem de declaração no TOML importa**
    /// pra dica which-key (docs/keys.md regra 8) — ela não é
    /// alfabetizada como as dicas da máquina modal.
    system_prefixes: HashMap<String, Vec<SystemPrefixEntry>>,
}

/// Uma tecla dentro de um prefixo de sistema (`[system_prefixes.*]`),
/// na ordem em que apareceu no TOML.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SystemPrefixEntry {
    pub key: Key,
    pub action: SystemAction,
    /// Uma ação destrutiva (hoje só `history-delete`) sempre aparece
    /// por último, na própria linha, destacada — ver
    /// `ui::which_key_lines` em `nas-term`.
    pub destructive: bool,
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

    /// O comando da classe sistema (Ctrl + tecla) ligado a `key`, se
    /// houver um em `[system]` que resolva direto (não um prefixo).
    pub(crate) fn system_action(&self, key: &Key) -> Option<SystemAction> {
        self.system.get(key).copied()
    }

    /// O nome do prefixo de sistema que `key` abre (`Ctrl+t` →
    /// `"tempo"`), se houver um.
    pub(crate) fn system_prefix_key(&self, key: &Key) -> Option<&str> {
        self.system_prefix_keys.get(key).map(String::as_str)
    }

    /// O comando ligado a `key` dentro do prefixo de sistema `name`
    /// (ex.: `"tempo"`, `n` → `checkpoint-new`).
    pub(crate) fn system_prefix_action(&self, name: &str, key: &Key) -> Option<SystemAction> {
        self.system_prefixes
            .get(name)?
            .iter()
            .find(|entry| entry.key == *key)
            .map(|entry| entry.action)
    }

    /// As teclas válidas, suas descrições e se são destrutivas, para
    /// um prefixo de sistema pendente (`Ctrl+t`) — mesma ideia de
    /// `hint_for_prefix`, mas para `[system_prefixes.*]`. **Na ordem
    /// declarada no TOML** (não alfabetizada) — ver `SystemPrefixEntry`.
    pub(crate) fn hint_for_system_prefix(&self, name: &str) -> Vec<(Key, String, bool)> {
        let Some(entries) = self.system_prefixes.get(name) else {
            return Vec::new();
        };
        entries
            .iter()
            .map(|entry| {
                (
                    entry.key,
                    self.describe(entry.action.command_name()),
                    entry.destructive,
                )
            })
            .collect()
    }

    /// A tecla (sem Ctrl) que abre o prefixo de sistema `name` — pra
    /// mostrar "Ctrl+<tecla>" no título da dica which-key.
    pub(crate) fn key_for_system_prefix(&self, name: &str) -> Option<Key> {
        self.system_prefix_keys
            .iter()
            .find(|(_, n)| n.as_str() == name)
            .map(|(key, _)| *key)
    }

    /// A tecla que abre o prefixo nomeado `name` em `[visual]` (`g`,
    /// `Espaço`) — pra mostrar no título da dica which-key.
    pub(crate) fn key_for_prefix(&self, name: &str) -> Option<Key> {
        self.visual
            .iter()
            .find(|(_, binding)| matches!(binding, Binding::Prefix(n) if n == name))
            .map(|(key, _)| *key)
    }

    /// A tecla que inicia o verbo `verb` em `[visual]` (`d`, `c`, `x`).
    pub(crate) fn key_for_verb(&self, verb: Verb) -> Option<Key> {
        self.visual
            .iter()
            .find(|(_, binding)| matches!(binding, Binding::Verb(v) if *v == verb))
            .map(|(key, _)| *key)
    }

    /// A tecla que inicia a unidade `unit` em `[visual]` (`w`, `W`).
    pub(crate) fn key_for_unit(&self, unit: Unit) -> Option<Key> {
        self.visual
            .iter()
            .find(|(_, binding)| matches!(binding, Binding::Unit(u) if *u == unit))
            .map(|(key, _)| *key)
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
    /// (`w`, `W`, sozinha ou depois de um verbo).
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

    /// O catálogo da paleta de comandos — ver [`crate::machine::Machine::command_catalog`],
    /// que é quem expõe isto fora do crate (depois de traduzir as
    /// teclas pro layout físico). Monta a lista crua (nome, teclas em
    /// qwerty canônico, se é Ctrl, se é destrutivo) a partir do
    /// próprio keymap, sem tocar nada externo — determinístico:
    /// `HashMap` não garante ordem, então a lista crua é sempre
    /// ordenada e deduplicada antes de devolver (ver `dedup_sorted`).
    ///
    /// **O que entra:** ações diretas, verbos e unidades de `[visual]`
    /// (armam um pendente — executar pela paleta "aperta a tecla", não
    /// completa o composto sozinho), prefixos resolvidos
    /// recursivamente até um comando de verdade (`g l`, `SPC f`, ...),
    /// unidade sozinha + direção (`w ;`, `W k`, ..., que é como
    /// `move-word-forward` etc. são alcançados — a mesma resolução de
    /// `Machine::resolve_pending`), e a classe sistema/Ctrl
    /// (`[system]` + `[system_prefixes.*]`).
    ///
    /// **O que não entra, de propósito:** os próprios prefixos (`go`,
    /// `leader`, `tempo` — docs/keys.md, "Prefixos não aparecem como
    /// itens") e os nomes de **objeto** puro (`char-left`,
    /// `whole-buffer`, `current-line`, ...) — esses só existem como o
    /// complemento de um verbo já pendente, nunca como um comando
    /// autônomo que "apertar uma tecla a partir do Visual" alcance por
    /// si só; incluí-los exigiria simular um verbo arbitrário
    /// pendente, o que não corresponde a nenhuma tecla real.
    pub(crate) fn catalog(&self) -> Vec<CatalogEntry> {
        let mut raw = Vec::new();
        collect_modal_catalog(&self.visual, &[], self, &mut raw);
        collect_system_catalog(self, &mut raw);
        dedup_sorted(raw, self)
    }
}

/// Uma entrada crua do catálogo da paleta, antes de alfabetizar e
/// deduplicar — nome, teclas (em qwerty canônico), se a primeira é
/// Ctrl, se é destrutivo. Ver [`Keymap::catalog`].
type RawCatalogEntry = (String, Vec<Key>, bool, bool);

/// Percorre `[visual]` (ou um `[prefixes.*]`, recursivamente)
/// coletando todo comando alcançável, com a sequência de teclas que o
/// alcança a partir de `prefix` (vazio na chamada de topo). Verbos e
/// unidades entram como o próprio comando (`delete`, `word`, ...) —
/// nunca recursa nos objetos que completam o composto, ver a nota em
/// [`Keymap::catalog`]. A única exceção em que uma unidade *sozinha*
/// produz um comando diferente de si mesma é a direção que a completa
/// (`w ;` → `move-word-forward`), resolvida abaixo via
/// `plain_motion_for` — a mesma conversão que `Machine::resolve_pending`
/// já faz em tempo de tecla.
fn collect_modal_catalog(
    bindings: &HashMap<Key, Binding>,
    prefix: &[Key],
    keymap: &Keymap,
    out: &mut Vec<RawCatalogEntry>,
) {
    for (key, binding) in bindings {
        let mut keys = prefix.to_vec();
        keys.push(*key);
        match binding {
            Binding::Action(action) => {
                if let Some(name) = action.command_name() {
                    out.push((name.to_string(), keys, false, false));
                }
            }
            Binding::EnterInsert => out.push(("enter-insert".to_string(), keys, false, false)),
            Binding::EnterSequential => {
                out.push(("enter-sequential".to_string(), keys, false, false))
            }
            Binding::Repeat => out.push(("repeat".to_string(), keys, false, false)),
            // Só existe em [insert] (M-SPC -> enter-visual); a paleta só
            // abre a partir do Visual (docs/keys.md), então nunca é
            // alcançável daqui — sem entrada no catálogo.
            Binding::EnterVisual => {}
            Binding::Verb(verb) => {
                out.push((verb.command_name().to_string(), keys, false, false));
            }
            Binding::Unit(unit) => {
                out.push((unit.command_name().to_string(), keys.clone(), false, false));
                for (direction, object) in keymap.objects_for_unit(*unit) {
                    if let Some(action) = plain_motion_for(*object)
                        && let Some(name) = action.command_name()
                    {
                        let mut compound = keys.clone();
                        compound.push(*direction);
                        out.push((name.to_string(), compound, false, false));
                    }
                }
            }
            Binding::Prefix(name) => {
                if let Some(sub) = keymap.prefixes.get(name) {
                    collect_modal_catalog(sub, &keys, keymap, out);
                }
            }
        }
    }
}

/// A classe sistema (`[system]` + `[system_prefixes.*]`) — a primeira
/// tecla é sempre Ctrl; a segunda (quando houver, pelo prefixo
/// `Ctrl+t`), nunca.
fn collect_system_catalog(keymap: &Keymap, out: &mut Vec<RawCatalogEntry>) {
    for (key, action) in &keymap.system {
        out.push((action.command_name().to_string(), vec![*key], true, false));
    }
    for (key, prefix_name) in &keymap.system_prefix_keys {
        let Some(entries) = keymap.system_prefixes.get(prefix_name) else {
            continue;
        };
        for entry in entries {
            out.push((
                entry.action.command_name().to_string(),
                vec![*key, entry.key],
                true,
                entry.destructive,
            ));
        }
    }
}

/// Determinismo sobre um `HashMap`: ordena por (nome, teclas
/// renderizadas) e mantém só a primeira ocorrência de cada nome —
/// alguns comandos têm mais de um caminho até eles no keymap padrão
/// (`move-buffer-start` por `L` e por `g g`; `enter-insert` por `RET` e
/// por `M-SPC`), e a paleta mostra só um atalho por item. Critério
/// arbitrário mas estável — o keymap não declara um atalho "canônico",
/// e qualquer escolha fixa serve, desde que não mude de uma chamada
/// pra outra (o que quebraria os snapshots da paleta em `nas-term`).
fn dedup_sorted(mut raw: Vec<RawCatalogEntry>, keymap: &Keymap) -> Vec<CatalogEntry> {
    raw.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| render_keys(&a.1).cmp(&render_keys(&b.1)))
    });
    let mut seen = std::collections::HashSet::new();
    raw.into_iter()
        .filter(|(name, ..)| seen.insert(name.clone()))
        .map(|(name, keys, ctrl, destructive)| {
            let description = keymap.describe(&name);
            CatalogEntry {
                name,
                description,
                keys,
                ctrl,
                destructive,
            }
        })
        .collect()
}

fn render_keys(keys: &[Key]) -> String {
    keys.iter()
        .map(Key::to_string)
        .collect::<Vec<_>>()
        .join(" ")
}

/// `deny_unknown_fields`: um atalho com campos de mais de uma forma
/// ao mesmo tempo (ex.: `{ prefix = "go", verb = "delete" }`) é um
/// erro de verdade, não um dos dois campos lido e o outro ignorado
/// em silêncio — `#[serde(untagged)]` tenta cada variante em ordem,
/// e sem isto a primeira que bater parcialmente venceria, mascarando
/// um `keymap.toml` malformado (ADR 0007-configuracao-em-camadas.md,
/// revisão do PR).
#[derive(Debug, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
enum RawBinding {
    Named(String),
    Prefix { prefix: String },
    Verb { verb: String },
    Unit { unit: String },
}

/// Uma entrada de `[system]`: ou resolve direto pra um comando
/// (`"save"`), ou entra num prefixo nomeado (`{ prefix = "tempo" }`)
/// que espera mais uma tecla, resolvida em `[system-prefixes.<nome>]`.
#[derive(Debug, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
enum RawSystemBinding {
    Named(String),
    Prefix { prefix: String },
}

/// Uma entrada de `[system_prefixes.<nome>]`: ou só o nome do comando
/// (`"checkpoint-new"`), ou uma forma detalhada marcando a ação como
/// destrutiva (`{ action = "history-delete", destructive = true }`) —
/// uma ação destrutiva sempre aparece por último, na própria linha,
/// na dica which-key (ver `ui::which_key_lines` em `nas-term`).
#[derive(Debug, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
enum RawSystemPrefixEntry {
    Named(String),
    Detailed {
        action: String,
        #[serde(default)]
        destructive: bool,
    },
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
    #[serde(default)]
    system: HashMap<String, RawSystemBinding>,
    /// A ordem das chaves dentro de cada `IndexMap` é a ordem em que
    /// apareceram no TOML — preservada de propósito (ver
    /// `SystemPrefixEntry`), diferente dos outros `HashMap` deste
    /// struct, cuja ordem não importa.
    #[serde(default)]
    system_prefixes: HashMap<String, IndexMap<String, RawSystemPrefixEntry>>,
}

#[derive(Debug, Default, Deserialize)]
struct RawObjects {
    #[serde(default)]
    word: HashMap<String, String>,
    #[serde(default)]
    paragraph: HashMap<String, String>,
    #[serde(default)]
    whole: HashMap<String, String>,
}

impl RawKeymap {
    fn resolve(self) -> Result<Keymap, KeymapError> {
        let visual = resolve_bindings(self.visual)?;
        let (objects_horizontal, objects_vertical) = derive_movement_objects(&visual);
        let (system, system_prefix_keys) = resolve_system(self.system)?;
        Ok(Keymap {
            insert: resolve_bindings(self.insert)?,
            visual,
            prefixes: self
                .prefixes
                .into_iter()
                .map(|(name, bindings)| Ok((name, resolve_bindings(bindings)?)))
                .collect::<Result<_, KeymapError>>()?,
            objects_horizontal,
            objects_vertical,
            objects_word: resolve_objects(self.objects.word)?,
            objects_paragraph: resolve_objects(self.objects.paragraph)?,
            objects_whole: resolve_objects(self.objects.whole)?,
            commands: self.commands,
            system,
            system_prefix_keys,
            system_prefixes: resolve_system_prefixes(self.system_prefixes)?,
        })
    }
}

/// Deriva os objetos horizontais e verticais direto das ações já
/// ligadas em `[visual]`, em vez de ler uma tabela separada: cada
/// movimento (`Action::as_horizontal_object`/`as_vertical_object`) diz
/// a que eixo pertence, e a tecla que já move o cursor assim também
/// vale como objeto de um verbo. Mudar a tecla de um movimento no
/// `[visual]` do TOML muda o objeto correspondente automaticamente —
/// não há como os dois ficarem fora de sincronia.
fn derive_movement_objects(
    visual: &HashMap<Key, Binding>,
) -> (HashMap<Key, Object>, HashMap<Key, Object>) {
    let mut horizontal = HashMap::new();
    let mut vertical = HashMap::new();
    for (key, binding) in visual {
        let Binding::Action(action) = binding else {
            continue;
        };
        if let Some(object) = action.as_horizontal_object() {
            horizontal.insert(*key, object);
        }
        if let Some(object) = action.as_vertical_object() {
            vertical.insert(*key, object);
        }
    }
    (horizontal, vertical)
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

/// Separa `[system]` em ações diretas e teclas que entram num
/// prefixo nomeado (`{ prefix = "tempo" }`) — ver [`RawSystemBinding`].
/// Resultado de separar `[system]` em ações diretas e teclas que
/// entram num prefixo nomeado — ver [`resolve_system`].
type ResolvedSystem = (HashMap<Key, SystemAction>, HashMap<Key, String>);

fn resolve_system(raw: HashMap<String, RawSystemBinding>) -> Result<ResolvedSystem, KeymapError> {
    let mut actions = HashMap::new();
    let mut prefix_keys = HashMap::new();
    for (key_spec, binding) in raw {
        let key = parse_key(&key_spec)?;
        match binding {
            RawSystemBinding::Named(name) => {
                actions.insert(key, parse_system_action(&name)?);
            }
            RawSystemBinding::Prefix { prefix } => {
                prefix_keys.insert(key, prefix);
            }
        }
    }
    Ok((actions, prefix_keys))
}

fn resolve_system_prefixes(
    raw: HashMap<String, IndexMap<String, RawSystemPrefixEntry>>,
) -> Result<HashMap<String, Vec<SystemPrefixEntry>>, KeymapError> {
    raw.into_iter()
        .map(|(name, bindings)| {
            let resolved = bindings
                .into_iter()
                .map(|(key_spec, raw_entry)| {
                    let key = parse_key(&key_spec)?;
                    let (action_name, destructive) = match raw_entry {
                        RawSystemPrefixEntry::Named(action) => (action, false),
                        RawSystemPrefixEntry::Detailed {
                            action,
                            destructive,
                        } => (action, destructive),
                    };
                    let action = parse_system_action(&action_name)?;
                    Ok(SystemPrefixEntry {
                        key,
                        action,
                        destructive,
                    })
                })
                .collect::<Result<_, KeymapError>>()?;
            Ok((name, resolved))
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

fn parse_system_action(name: &str) -> Result<SystemAction, KeymapError> {
    Ok(match name {
        "save" => SystemAction::Save,
        "quit" => SystemAction::Quit,
        "open-file" => SystemAction::OpenFile,
        "switch-buffer" => SystemAction::SwitchBuffer,
        "close-buffer" => SystemAction::CloseBuffer,
        "reload-buffer" => SystemAction::ReloadBuffer,
        "checkpoint-new" => SystemAction::CheckpointNew,
        "checkpoint-list" => SystemAction::CheckpointList,
        "checkpoint-diff" => SystemAction::CheckpointDiff,
        "checkpoint-restore" => SystemAction::CheckpointRestore,
        "checkpoint-blame" => SystemAction::CheckpointBlame,
        "history-delete" => SystemAction::HistoryDelete,
        "message-history" => SystemAction::MessageHistory,
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
        "open-command-palette" => Action::OpenCommandPalette,
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
            keymap.visual.get(&Key::char('W')),
            Some(&Binding::Unit(Unit::Paragraph))
        );
        assert_eq!(
            keymap.visual.get(&Key::char('p')),
            None,
            "'p' fica reservado — a unidade parágrafo é 'W' (Shift+w)"
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
    fn rejeita_atalho_com_campos_misturados() {
        // `{ prefix = "go", verb = "delete" }` não é nem um prefixo
        // puro nem um verbo puro — `deny_unknown_fields` faz isto
        // virar um erro de TOML, em vez de silenciosamente bater
        // com a primeira variante (`Prefix`) e descartar `verb`.
        let err = Keymap::from_toml_str(
            r#"[visual]
g = { prefix = "go", verb = "delete" }
"#,
        )
        .unwrap_err();
        assert!(matches!(err, KeymapError::Toml(_)));
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
            r#"[objects.word]
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

    #[test]
    fn objetos_horizontais_e_verticais_sao_derivados_do_visual() {
        // Nenhuma tabela [objects.horizontal]/[objects.vertical] é
        // lida — os objetos vêm das próprias ações de movimento já
        // ligadas em [visual]. Trocar a tecla de um movimento muda o
        // objeto correspondente automaticamente, sem editar dois
        // lugares.
        let keymap = Keymap::from_toml_str(
            r#"[visual]
x = "move-right"
y = "move-down"
"#,
        )
        .unwrap();

        assert_eq!(
            keymap.objects_horizontal().get(&Key::char('x')),
            Some(&Object::CharRight)
        );
        assert_eq!(
            keymap.objects_vertical().get(&Key::char('y')),
            Some(&Object::LineDown)
        );
        // a tecla de qualquer outro keymap não aparece aqui.
        assert_eq!(keymap.objects_horizontal().get(&Key::char(';')), None);
    }

    #[test]
    fn remapear_um_movimento_no_toml_remapeia_o_objeto_junto() {
        // O mesmo movimento (mover à direita), ligado a uma tecla
        // diferente da do default.toml: o objeto segue a tecla nova.
        let padrao = Keymap::default_keymap();
        assert_eq!(
            padrao.objects_horizontal().get(&Key::char('z')),
            None,
            "'z' não deveria significar nada no keymap padrão"
        );

        let remapeado = Keymap::from_toml_str(
            r#"[visual]
z = "move-right"
"#,
        )
        .unwrap();
        assert_eq!(
            remapeado.objects_horizontal().get(&Key::char('z')),
            Some(&Object::CharRight)
        );
    }

    #[test]
    fn classe_sistema_no_keymap_padrao() {
        let keymap = Keymap::default_keymap();
        assert_eq!(
            keymap.system_action(&Key::char('s')),
            Some(SystemAction::Save)
        );
        assert_eq!(
            keymap.system_action(&Key::char('q')),
            Some(SystemAction::Quit)
        );
        assert_eq!(
            keymap.system_action(&Key::char('o')),
            Some(SystemAction::OpenFile)
        );
        assert_eq!(
            keymap.system_action(&Key::char('b')),
            Some(SystemAction::SwitchBuffer)
        );
        assert_eq!(
            keymap.system_action(&Key::char('k')),
            Some(SystemAction::CloseBuffer)
        );
        assert_eq!(
            keymap.system_action(&Key::char('r')),
            Some(SystemAction::ReloadBuffer)
        );
        assert_eq!(keymap.system_action(&Key::char('z')), None);
    }

    #[test]
    fn catalogo_inclui_acao_direta_verbo_unidade_e_prefixo_resolvido() {
        let keymap = Keymap::default_keymap();
        let catalog = keymap.catalog();

        let find = |name: &str| catalog.iter().find(|entry| entry.name == name);

        let move_left = find("move-left").expect("move-left no catálogo");
        assert_eq!(move_left.keys, vec![Key::char('j')]);
        assert!(!move_left.ctrl);
        assert_eq!(move_left.description, "caractere anterior");

        let delete = find("delete").expect("delete (verbo) no catálogo");
        assert_eq!(delete.keys, vec![Key::char('d')]);

        let word = find("word").expect("word (unidade) no catálogo");
        assert_eq!(word.keys, vec![Key::char('w')]);

        // "g l" resolvido recursivamente através do prefixo "go".
        let goto_line = find("prompt-goto-line").expect("prompt-goto-line no catálogo");
        assert_eq!(goto_line.keys, vec![Key::char('g'), Key::char('l')]);

        // unidade sozinha + direção: "w ;" alcança move-word-forward,
        // não listado em [visual] nem em nenhum [prefixes.*] — só
        // através da resolução de unidade (`plain_motion_for`).
        let word_forward = find("move-word-forward").expect("move-word-forward no catálogo");
        assert_eq!(word_forward.keys, vec![Key::char('w'), Key::char(';')]);
    }

    #[test]
    fn catalogo_nao_inclui_prefixos_nem_objetos_puros() {
        let keymap = Keymap::default_keymap();
        let catalog = keymap.catalog();
        let names: Vec<&str> = catalog.iter().map(|entry| entry.name.as_str()).collect();

        for prefix in ["go", "leader", "tempo"] {
            assert!(!names.contains(&prefix), "{prefix} não deveria aparecer");
        }
        for object in ["char-left", "whole-buffer", "current-line", "line-start"] {
            assert!(!names.contains(&object), "{object} não deveria aparecer");
        }
    }

    #[test]
    fn catalogo_classe_sistema_ctrl_so_na_primeira_tecla() {
        let keymap = Keymap::default_keymap();
        let catalog = keymap.catalog();

        let save = catalog
            .iter()
            .find(|entry| entry.name == "save")
            .expect("save no catálogo");
        assert_eq!(save.keys, vec![Key::char('s')]);
        assert!(save.ctrl);

        let checkpoint_new = catalog
            .iter()
            .find(|entry| entry.name == "checkpoint-new")
            .expect("checkpoint-new no catálogo");
        assert_eq!(checkpoint_new.keys, vec![Key::char('t'), Key::char('n')]);
        assert!(checkpoint_new.ctrl);

        let history_delete = catalog
            .iter()
            .find(|entry| entry.name == "history-delete")
            .expect("history-delete no catálogo");
        assert!(history_delete.destructive);
    }

    #[test]
    fn catalogo_e_deterministico_e_sem_nomes_duplicados() {
        let keymap = Keymap::default_keymap();
        let first = keymap.catalog();
        let second = keymap.catalog();
        assert_eq!(
            first, second,
            "duas chamadas deveriam devolver a mesma lista"
        );

        let mut names: Vec<&str> = first.iter().map(|entry| entry.name.as_str()).collect();
        let total = names.len();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), total, "nenhum nome deveria se repetir");

        // "move-buffer-start" é alcançável por "L" e por "g g" — a
        // dedupe deveria manter só um dos dois, não os dois.
        let occurrences = first
            .iter()
            .filter(|entry| entry.name == "move-buffer-start")
            .count();
        assert_eq!(occurrences, 1);
    }

    #[test]
    fn prefixo_de_sistema_tempo_no_keymap_padrao() {
        let keymap = Keymap::default_keymap();

        // 't' não resolve direto — abre o prefixo "tempo".
        assert_eq!(keymap.system_action(&Key::char('t')), None);
        assert_eq!(keymap.system_prefix_key(&Key::char('t')), Some("tempo"));

        assert_eq!(
            keymap.system_prefix_action("tempo", &Key::char('n')),
            Some(SystemAction::CheckpointNew)
        );
        assert_eq!(
            keymap.system_prefix_action("tempo", &Key::char('l')),
            Some(SystemAction::CheckpointList)
        );
        assert_eq!(
            keymap.system_prefix_action("tempo", &Key::char('e')),
            Some(SystemAction::CheckpointDiff)
        );
        assert_eq!(
            keymap.system_prefix_action("tempo", &Key::char('r')),
            Some(SystemAction::CheckpointRestore)
        );
        assert_eq!(
            keymap.system_prefix_action("tempo", &Key::char('a')),
            Some(SystemAction::CheckpointBlame)
        );
        assert_eq!(
            keymap.system_prefix_action("tempo", &Key::char('z')),
            Some(SystemAction::HistoryDelete)
        );
        assert_eq!(keymap.system_prefix_action("tempo", &Key::char('x')), None);
        assert_eq!(
            keymap.system_prefix_action("outro-prefixo", &Key::char('n')),
            None
        );
    }
}
