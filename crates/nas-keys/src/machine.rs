//! A máquina de estados do nas-keys: Insert, Visual e Sequential.
//!
//! A especificação completa das teclas — a ideia dos radicais e
//! compostos, as regras (um radical/um significado, modificador =
//! classe, Shift intensifica, verbo + objeto, seleção primeiro,
//! posição antes de letra, descoberta) e a tabela inteira — vive em
//! `docs/keys.md`. Este módulo implementa só a MECÂNICA de como um
//! estado, prefixo, verbo, unidade e o Sequential funcionam; QUAIS
//! teclas disparam o quê é dado, não código — vem de
//! `keymap/default.toml` (ver [`crate::keymap`]), carregado uma vez em
//! [`Keymap::default_keymap`].
//!
//! Não conhece `nas-editor::Document` nem nenhuma biblioteca de
//! terminal — recebe [`Key`] (e, pra decidir se um verbo age na hora
//! numa seleção, um `bool` dizendo se há uma seleção ativa) e devolve
//! [`Action`]s abstratas; quem chama decide como executá-las.
//!
//! ## Escopo desta etapa
//!
//! `SPC r` (substituir) e `SPC h` (ajuda) não têm binding no keymap
//! padrão — substituir precisa de um fluxo interativo de confirmação
//! por ocorrência que ainda não existe, e ajuda não tem pra onde
//! apontar ainda. A gerência de janelas (classe Espaço/Alt) também não
//! está implementada — `nas-term` não tem painéis múltiplos. Ver
//! `docs/keys.md` para os detalhes.

use std::collections::HashMap;

use crate::key::{Key, KeyCode};
use crate::keymap::{Binding, Keymap};
use crate::layout::{self, Layout};

/// Em qual dos três estados a máquina está.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModalState {
    Insert,
    Visual,
    Sequential,
}

/// Um verbo (docs/keys.md §4.2): espera um objeto, ou age na hora
/// sobre uma seleção ativa.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Verb {
    Delete,
    Copy,
    Cut,
}

impl Verb {
    pub(crate) fn command_name(&self) -> &'static str {
        match self {
            Verb::Delete => "delete",
            Verb::Copy => "copy",
            Verb::Cut => "cut",
        }
    }
}

/// Uma unidade (docs/keys.md §4.1): sozinha, espera uma direção pra
/// mover; depois de um verbo, espera uma direção pra formar um objeto
/// composto (`d w ;`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Unit {
    Word,
    Paragraph,
}

impl Unit {
    pub(crate) fn command_name(&self) -> &'static str {
        match self {
            Unit::Word => "word",
            Unit::Paragraph => "paragraph",
        }
    }
}

/// O objeto de um verbo (docs/keys.md §4.3): a região sobre a qual o
/// verbo age. As primeiras dez são posições de destino de um
/// movimento (o objeto é a região do cursor até lá); `CurrentLine` é o
/// verbo repetido (`d d`); `WholeBuffer` é a tecla `a`. Quem resolve a
/// região de verdade (em caracteres, ou linha a linha) é quem chama —
/// esta máquina não conhece o documento.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Object {
    CharLeft,
    CharRight,
    LineStart,
    LineEnd,
    LineDown,
    LineUp,
    BufferEnd,
    BufferStart,
    WordBackward,
    WordForward,
    ParagraphForward,
    ParagraphBackward,
    CurrentLine,
    WholeBuffer,
}

impl Object {
    pub(crate) fn command_name(&self) -> &'static str {
        match self {
            Object::CharLeft => "char-left",
            Object::CharRight => "char-right",
            Object::LineStart => "line-start",
            Object::LineEnd => "line-end",
            Object::LineDown => "line-down",
            Object::LineUp => "line-up",
            Object::BufferEnd => "buffer-end",
            Object::BufferStart => "buffer-start",
            Object::WordBackward => "word-backward",
            Object::WordForward => "word-forward",
            Object::ParagraphForward => "paragraph-forward",
            Object::ParagraphBackward => "paragraph-backward",
            Object::CurrentLine => "current-line",
            Object::WholeBuffer => "whole-buffer",
        }
    }
}

/// O que fazer, em resposta a uma tecla — a máquina nunca executa nada
/// sozinha, só devolve a lista (possivelmente vazia) de ações.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    MoveLeft,
    MoveRight,
    MoveUp,
    MoveDown,
    MoveLineStart,
    MoveLineEnd,
    MoveBufferStart,
    MoveBufferEnd,
    MoveWordBackward,
    MoveWordForward,
    MoveParagraphForward,
    MoveParagraphBackward,
    DeleteForward,
    DeleteBackward,
    Undo,
    InsertChar(char),
    InsertNewline,
    /// Pede a quem chama pra abrir uma linha de entrada de busca
    /// incremental (líder `f`, isearch-forward).
    PromptSearch,
    /// Pede a quem chama pra abrir uma linha de entrada pedindo um
    /// número de linha (prefixo `g`, `l`, goto-line).
    PromptGotoLine,
    /// Marca o início de uma seleção no cursor (`s`, set-mark-command).
    StartSelection,
    /// Seleciona o documento inteiro (`S`, mark-whole-buffer).
    SelectAll,
    /// Copia a seleção pra área de transferência agora — `c` com uma
    /// seleção ativa (docs/keys.md §4.2, "seleção primeiro").
    Copy,
    /// Recorta a seleção pra área de transferência agora — `x` com
    /// uma seleção ativa.
    Cut,
    /// Apaga a seleção agora — `d` com uma seleção ativa.
    DeleteSelection,
    /// Cola o que está na área de transferência (`y`, yank). Com uma
    /// seleção ativa, substitui a seleção — decisão de quem chama, a
    /// máquina só sabe que `y` foi pressionado.
    Paste,
    /// Um verbo agindo sobre um objeto (`d ;`, `d w ;`, `d k`, `d d`,
    /// `d a`, ...) — docs/keys.md §4.2 e §4.3. Quem chama resolve a
    /// região de verdade a partir do objeto e aplica o verbo sobre ela.
    VerbObject(Verb, Object),
}

impl Action {
    /// O nome usado em `[commands]` do keymap, para as ações que podem
    /// aparecer como alvo nomeado de uma tecla (prefixo ou binding
    /// direto). `None` para ações que só a máquina produz
    /// internamente (nunca aparecem como string no TOML).
    pub(crate) fn command_name(&self) -> Option<&'static str> {
        Some(match self {
            Action::MoveLeft => "move-left",
            Action::MoveRight => "move-right",
            Action::MoveUp => "move-up",
            Action::MoveDown => "move-down",
            Action::MoveLineStart => "move-line-start",
            Action::MoveLineEnd => "move-line-end",
            Action::MoveBufferStart => "move-buffer-start",
            Action::MoveBufferEnd => "move-buffer-end",
            Action::MoveWordBackward => "move-word-backward",
            Action::MoveWordForward => "move-word-forward",
            Action::MoveParagraphForward => "move-paragraph-forward",
            Action::MoveParagraphBackward => "move-paragraph-backward",
            Action::Undo => "undo",
            Action::PromptSearch => "prompt-search",
            Action::PromptGotoLine => "prompt-goto-line",
            Action::StartSelection => "start-selection",
            Action::SelectAll => "select-all",
            Action::Paste => "paste",
            Action::DeleteForward
            | Action::DeleteBackward
            | Action::InsertChar(_)
            | Action::InsertNewline
            | Action::Copy
            | Action::Cut
            | Action::DeleteSelection
            | Action::VerbObject(_, _) => return None,
        })
    }
}

/// Prefixo, verbo ou unidade aguardando a(s) próxima(s) tecla(s).
#[derive(Debug, Clone, PartialEq, Eq)]
enum Pending {
    /// `g`, líder: espera uma tecla do prefixo nomeado. Tecla inválida
    /// cancela (volta pro Visual sem ação) — igual a antes do
    /// redesenho; ver docs/keys.md §4.5/§4.6, que não pedem persistir.
    Prefix(String),
    /// Um verbo esperando o primeiro objeto (docs/keys.md §4.3).
    /// Tecla inválida **persiste**: o verbo continua pendente.
    Verb(Verb),
    /// Um verbo que viu uma unidade (`w`/`p`) e agora espera a
    /// direção que completa o objeto composto (`d w ;`). Também
    /// persiste em tecla inválida — ainda estamos resolvendo o
    /// objeto do verbo.
    VerbUnit(Verb, Unit),
    /// Uma unidade sozinha (sem verbo) esperando uma direção —
    /// movimento puro. Tecla inválida cancela, como um prefixo (não
    /// está envolvida num verbo, então §4.3 não se aplica).
    Unit(Unit),
}

/// Rótulo e teclas válidas para a dica mostrada enquanto algo está
/// pendente (verbo, unidade ou prefixo) — docs/keys.md regra 8
/// (descobrível).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingHint {
    pub label: String,
    pub entries: Vec<(Key, String)>,
}

/// A máquina de estados. Testável sem terminal: alimente com [`Key`]s e
/// confira as [`Action`]s e o estado resultantes. As teclas que cada
/// estado reconhece vêm do [`Keymap`] — ver [`crate::keymap`].
pub struct Machine {
    keymap: Keymap,
    layout_table: HashMap<char, char>,
    state: ModalState,
    pending: Option<Pending>,
    sequence: Vec<Key>,
    /// O último composto verbo+objeto que alterou o texto (Delete ou
    /// Cut — nunca Copy, que não altera nada), pra `.` repetir
    /// (docs/keys.md §4.2). `None` se nenhum composto desses ainda
    /// rodou nesta sessão.
    last_altering_compound: Option<(Verb, Object)>,
}

impl Default for Machine {
    fn default() -> Self {
        Self::new()
    }
}

impl Machine {
    /// Começa em Visual, como o nas-keys real (`nas--enable` chama
    /// `nas-enter-visual`), com o keymap padrão (`keymap/default.toml`)
    /// e o layout físico qwerty.
    pub fn new() -> Self {
        Self::with_layout(Layout::Qwerty)
    }

    /// Como [`Machine::new`], mas traduzindo teclas de um layout físico
    /// diferente (ex.: `Layout::QwertyAbnt`) de volta para o qwerty
    /// canônico antes de consultar o keymap.
    pub fn with_layout(layout: Layout) -> Self {
        Self {
            keymap: Keymap::default_keymap(),
            layout_table: layout.translation_table(),
            state: ModalState::Visual,
            pending: None,
            sequence: Vec::new(),
            last_altering_compound: None,
        }
    }

    pub fn state(&self) -> ModalState {
        self.state
    }

    /// As teclas acumuladas no estado Sequential (vazio em qualquer outro
    /// estado).
    pub fn pending_sequence(&self) -> &[Key] {
        &self.sequence
    }

    /// Enquanto um verbo, unidade ou prefixo está pendente: seu rótulo
    /// e as teclas válidas pra continuar, com uma descrição curta de
    /// cada — pra `nas-term` mostrar na barra de status
    /// (docs/keys.md regra 8). `None` fora do Visual ou sem nada
    /// pendente.
    pub fn pending_hint(&self) -> Option<PendingHint> {
        match self.pending.as_ref()? {
            Pending::Prefix(name) => Some(PendingHint {
                label: self.keymap.describe(name),
                entries: self.keymap.hint_for_prefix(name),
            }),
            Pending::Verb(verb) => Some(PendingHint {
                label: self.keymap.describe(verb.command_name()),
                entries: self.keymap.hint_for_verb(),
            }),
            Pending::VerbUnit(verb, unit) => Some(PendingHint {
                label: format!(
                    "{} {}",
                    self.keymap.describe(verb.command_name()),
                    self.keymap.describe(unit.command_name())
                ),
                entries: self.keymap.hint_for_unit(*unit),
            }),
            Pending::Unit(unit) => Some(PendingHint {
                label: self.keymap.describe(unit.command_name()),
                entries: self.keymap.hint_for_unit(*unit),
            }),
        }
    }

    /// Equivalente a `nas--lighter`: um rótulo curto para a barra de
    /// status, mostrando as teclas acumuladas quando em Sequential.
    pub fn status_label(&self) -> String {
        match self.state {
            ModalState::Insert => "I".to_string(),
            ModalState::Visual => "V".to_string(),
            ModalState::Sequential => {
                if self.sequence.is_empty() {
                    "S".to_string()
                } else {
                    let keys = self
                        .sequence
                        .iter()
                        .map(Key::to_string)
                        .collect::<Vec<_>>()
                        .join(" ");
                    format!("S: {keys}")
                }
            }
        }
    }

    /// Processa uma tecla e devolve as ações resultantes. `has_selection`
    /// diz se há uma seleção ativa *agora*, no documento de quem chama —
    /// só importa pra decidir se um verbo (`d`/`c`/`x`) age na hora
    /// sobre ela (docs/keys.md §4.2, "seleção primeiro") ou entra em
    /// espera por um objeto; em qualquer outro contexto é ignorado. A
    /// tecla é traduzida do layout físico configurado para o qwerty
    /// canônico antes de qualquer outra coisa.
    pub fn handle_key(&mut self, key: Key, has_selection: bool) -> Vec<Action> {
        let key = layout::to_qwerty(&self.layout_table, key);
        match self.state {
            ModalState::Insert => self.handle_insert(key),
            ModalState::Visual => self.handle_visual(key, has_selection),
            ModalState::Sequential => self.handle_sequential(key, has_selection),
        }
    }

    fn enter_insert(&mut self) {
        self.state = ModalState::Insert;
        self.pending = None;
    }

    fn enter_visual(&mut self) {
        self.state = ModalState::Visual;
        self.pending = None;
    }

    fn enter_sequential(&mut self) {
        self.sequence.clear();
        self.state = ModalState::Sequential;
        self.pending = None;
    }

    /// Em insert, só as teclas do keymap (M-SPC) são interceptadas — o
    /// resto cai para a edição normal (`nas-insert-map` só sobrescreve a
    /// troca de modo).
    fn handle_insert(&mut self, key: Key) -> Vec<Action> {
        if let Some(binding) = self.keymap.insert.get(&key).cloned() {
            return self.run_binding(binding);
        }
        if let Some(action) = universal_action(key) {
            return vec![action];
        }
        match key.code {
            KeyCode::Char(c) if !key.alt => vec![Action::InsertChar(c)],
            KeyCode::Enter => vec![Action::InsertNewline],
            _ => Vec::new(),
        }
    }

    /// Em visual, letras são comandos (self-insert suprimido); setas,
    /// Backspace, Delete e Tab não são interceptados pelo nas-map.el e
    /// continuam funcionando normalmente.
    fn handle_visual(&mut self, key: Key, has_selection: bool) -> Vec<Action> {
        // Esc cancela qualquer verbo, prefixo ou unidade pendente —
        // docs/keys.md §4.7. Não existe ação de Esc fora disso.
        if matches!(key.code, KeyCode::Escape) && !key.alt && self.pending.is_some() {
            self.pending = None;
            return Vec::new();
        }

        if let Some(pending) = self.pending.clone() {
            return self.resolve_pending(pending, key);
        }

        if let Some(binding) = self.keymap.visual.get(&key).cloned() {
            if let Binding::Verb(verb) = binding {
                return self.enter_verb(verb, has_selection);
            }
            return self.run_binding(binding);
        }

        universal_action(key).into_iter().collect()
    }

    /// `d`/`c`/`x`: com seleção ativa, age na hora; senão, começa a
    /// esperar um objeto.
    fn enter_verb(&mut self, verb: Verb, has_selection: bool) -> Vec<Action> {
        if has_selection {
            return vec![match verb {
                Verb::Delete => Action::DeleteSelection,
                Verb::Copy => Action::Copy,
                Verb::Cut => Action::Cut,
            }];
        }
        self.pending = Some(Pending::Verb(verb));
        Vec::new()
    }

    /// Executa uma ligação do keymap que não depende de contexto além
    /// da tecla em si (trocas de estado e entrada em prefixo já sabem
    /// qual prefixo é pelo nome; `Binding::Verb` é tratado à parte em
    /// [`Machine::enter_verb`], que precisa saber se há seleção).
    fn run_binding(&mut self, binding: Binding) -> Vec<Action> {
        match binding {
            Binding::Action(action) => vec![action],
            Binding::EnterInsert => {
                self.enter_insert();
                Vec::new()
            }
            Binding::EnterVisual => {
                self.enter_visual();
                Vec::new()
            }
            Binding::EnterSequential => {
                self.enter_sequential();
                Vec::new()
            }
            Binding::Prefix(name) => {
                self.pending = Some(Pending::Prefix(name));
                Vec::new()
            }
            Binding::Verb(verb) => {
                // Só chega aqui fora do Visual (ex.: Sequential, que
                // reprocessa cada tecla por handle_visual — então na
                // prática nunca chega aqui sem seleção decidida; mas
                // pra manter run_binding total, sem seleção por padrão).
                self.enter_verb(verb, false)
            }
            Binding::Unit(unit) => {
                self.pending = Some(Pending::Unit(unit));
                Vec::new()
            }
            Binding::Repeat => self
                .last_altering_compound
                .map(|(verb, object)| vec![Action::VerbObject(verb, object)])
                .unwrap_or_default(),
        }
    }

    fn resolve_pending(&mut self, pending: Pending, key: Key) -> Vec<Action> {
        match pending {
            Pending::Prefix(name) => {
                // Tecla inválida cancela (consome e não faz nada) —
                // comportamento de antes do redesenho, que docs/keys.md
                // não pede mudar pra prefixos.
                let binding = self
                    .keymap
                    .prefixes
                    .get(&name)
                    .and_then(|bindings| bindings.get(&key))
                    .cloned();
                self.pending = None;
                match binding {
                    Some(binding) => self.run_binding(binding),
                    None => Vec::new(),
                }
            }
            Pending::Unit(unit) => {
                // Unidade sozinha: tecla inválida cancela, como um
                // prefixo — não há verbo em jogo, então §4.3 (que pede
                // persistir) não se aplica.
                let object = self.keymap.objects_for_unit(unit).get(&key).copied();
                self.pending = None;
                match object.and_then(plain_motion_for) {
                    Some(action) => vec![action],
                    None => Vec::new(),
                }
            }
            Pending::Verb(verb) => self.resolve_verb(verb, key),
            Pending::VerbUnit(verb, unit) => {
                match self.keymap.objects_for_unit(unit).get(&key).copied() {
                    Some(object) => {
                        self.pending = None;
                        self.remember_if_altering(verb, object);
                        vec![Action::VerbObject(verb, object)]
                    }
                    // Direção que não serve pra essa unidade (ex. "d w k"):
                    // ainda estamos resolvendo o objeto do verbo — persiste.
                    None => {
                        self.pending = Some(Pending::VerbUnit(verb, unit));
                        Vec::new()
                    }
                }
            }
        }
    }

    /// Resolve a tecla seguinte a um verbo pendente contra os objetos
    /// possíveis (docs/keys.md §4.3): um movimento horizontal ou
    /// vertical, o arquivo inteiro (`a`), o próprio verbo repetido
    /// (linha atual) ou uma unidade (que passa a esperar a direção).
    /// Tecla inválida: persiste, sem executar nada.
    fn resolve_verb(&mut self, verb: Verb, key: Key) -> Vec<Action> {
        if let Some(&object) = self
            .keymap
            .objects_horizontal()
            .get(&key)
            .or_else(|| self.keymap.objects_vertical().get(&key))
            .or_else(|| self.keymap.objects_whole().get(&key))
        {
            self.pending = None;
            self.remember_if_altering(verb, object);
            return vec![Action::VerbObject(verb, object)];
        }

        match self.keymap.visual.get(&key).cloned() {
            // O próprio verbo repetido (`d d`, `c c`, `x x`): a linha
            // atual inteira.
            Some(Binding::Verb(same)) if same == verb => {
                self.pending = None;
                self.remember_if_altering(verb, Object::CurrentLine);
                vec![Action::VerbObject(verb, Object::CurrentLine)]
            }
            // Uma unidade (`w`, `p`): o objeto ainda não está completo,
            // falta a direção.
            Some(Binding::Unit(unit)) => {
                self.pending = Some(Pending::VerbUnit(verb, unit));
                Vec::new()
            }
            // Tecla inválida como objeto: o verbo continua esperando.
            _ => {
                self.pending = Some(Pending::Verb(verb));
                Vec::new()
            }
        }
    }

    /// Guarda o composto pra `.` repetir, exceto quando o verbo é
    /// `Copy` — copiar não altera o texto, então `.` não deveria
    /// repeti-lo (docs/keys.md §4.2: "repete o último composto que
    /// alterou o texto").
    fn remember_if_altering(&mut self, verb: Verb, object: Object) {
        if verb != Verb::Copy {
            self.last_altering_compound = Some((verb, object));
        }
    }

    /// Em sequential, nada passa direto: RET executa, ESC cancela, DEL
    /// remove a última tecla acumulada, e qualquer outra tecla (incluindo
    /// setas, Delete, Tab — tudo) é só acumulada, igual ao catch-all `[t]`
    /// do nas-sequential-map.
    fn handle_sequential(&mut self, key: Key, has_selection: bool) -> Vec<Action> {
        match key.code {
            KeyCode::Enter if !key.alt => self.execute_sequential(has_selection),
            KeyCode::Escape if !key.alt => {
                self.sequence.clear();
                self.enter_visual();
                Vec::new()
            }
            KeyCode::Backspace if !key.alt => {
                self.sequence.pop();
                Vec::new()
            }
            _ => {
                self.sequence.push(key);
                Vec::new()
            }
        }
    }

    fn execute_sequential(&mut self, has_selection: bool) -> Vec<Action> {
        let keys = std::mem::take(&mut self.sequence);
        self.enter_visual();
        let mut actions = Vec::new();
        for key in keys {
            actions.extend(self.handle_visual(key, has_selection));
        }
        actions
    }
}

/// Traduz um objeto de unidade (palavra/parágrafo) num movimento puro,
/// pra quando a unidade é usada sozinha (sem verbo). As demais
/// variantes de [`Object`] nunca aparecem aqui — só palavra e
/// parágrafo têm tabela de unidade no keymap.
fn plain_motion_for(object: Object) -> Option<Action> {
    match object {
        Object::WordBackward => Some(Action::MoveWordBackward),
        Object::WordForward => Some(Action::MoveWordForward),
        Object::ParagraphForward => Some(Action::MoveParagraphForward),
        Object::ParagraphBackward => Some(Action::MoveParagraphBackward),
        _ => None,
    }
}

/// Teclas que, segundo `nas-map.el`, o nas-keys não intercepta em Insert
/// nem em Visual (não estão em nenhum dos dois keymaps) — continuam
/// fazendo a mesma coisa nos dois estados. Não se aplica em Sequential,
/// que captura tudo via `[t]`.
fn universal_action(key: Key) -> Option<Action> {
    if key.alt {
        return None;
    }
    match key.code {
        KeyCode::Backspace => Some(Action::DeleteBackward),
        KeyCode::Delete => Some(Action::DeleteForward),
        KeyCode::Left => Some(Action::MoveLeft),
        KeyCode::Right => Some(Action::MoveRight),
        KeyCode::Up => Some(Action::MoveUp),
        KeyCode::Down => Some(Action::MoveDown),
        KeyCode::Tab => Some(Action::InsertChar('\t')),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(s: &str) -> Vec<Key> {
        s.chars().map(Key::char).collect()
    }

    fn run(machine: &mut Machine, s: &str) -> Vec<Action> {
        run_sel(machine, s, false)
    }

    fn run_sel(machine: &mut Machine, s: &str, has_selection: bool) -> Vec<Action> {
        let mut actions = Vec::new();
        for key in keys(s) {
            actions.extend(machine.handle_key(key, has_selection));
        }
        actions
    }

    #[test]
    fn comeca_em_visual() {
        let machine = Machine::new();
        assert_eq!(machine.state(), ModalState::Visual);
    }

    #[test]
    fn enter_insert_troca_estado() {
        let mut machine = Machine::new();
        machine.handle_key(Key::enter(), false);
        assert_eq!(machine.state(), ModalState::Insert);
    }

    #[test]
    fn enter_sequential_limpa_sequencia_acumulada() {
        let mut machine = Machine::new();
        run(&mut machine, "  "); // Espaço, Espaço (líder, depois SPC resolve pra Sequential)
        assert_eq!(machine.state(), ModalState::Sequential);
        assert!(machine.pending_sequence().is_empty());
    }

    #[test]
    fn status_label_por_estado() {
        let mut machine = Machine::new();
        machine.handle_key(Key::enter(), false);
        assert_eq!(machine.status_label(), "I");

        machine.handle_key(Key::alt_char(' '), false);
        assert_eq!(machine.status_label(), "V");

        run(&mut machine, " "); // entra no leader
        machine.handle_key(Key::char(' '), false); // resolve o leader -> sequential
        assert_eq!(machine.status_label(), "S");

        machine.handle_key(Key::char('j'), false);
        machine.handle_key(Key::char('k'), false);
        assert_eq!(machine.status_label(), "S: j k");
    }

    #[test]
    fn visual_motion_por_caractere() {
        let mut machine = Machine::new();
        assert_eq!(
            run(&mut machine, "jjj"),
            vec![Action::MoveLeft, Action::MoveLeft, Action::MoveLeft]
        );

        let mut machine = Machine::new();
        assert_eq!(
            run(&mut machine, ";;;"),
            vec![Action::MoveRight, Action::MoveRight, Action::MoveRight]
        );
    }

    #[test]
    fn visual_motion_de_linha() {
        let mut machine = Machine::new();
        assert_eq!(
            run(&mut machine, "kl"),
            vec![Action::MoveDown, Action::MoveUp]
        );
    }

    #[test]
    fn visual_motion_extrema() {
        let mut machine = Machine::new();
        assert_eq!(
            machine.handle_key(Key::char('J'), false),
            vec![Action::MoveLineStart]
        );
        assert_eq!(
            machine.handle_key(Key::char(':'), false),
            vec![Action::MoveLineEnd]
        );
        assert_eq!(
            machine.handle_key(Key::char('L'), false),
            vec![Action::MoveBufferStart]
        );
        assert_eq!(
            machine.handle_key(Key::char('K'), false),
            vec![Action::MoveBufferEnd]
        );
    }

    #[test]
    fn enter_em_visual_troca_para_insert_em_vez_de_inserir_texto() {
        let mut machine = Machine::new();
        assert_eq!(
            machine.handle_key(Key::enter(), false),
            Vec::<Action>::new()
        );
        assert_eq!(machine.state(), ModalState::Insert);
        // a próxima tecla já insere texto, não é mais um comando visual
        assert_eq!(
            machine.handle_key(Key::char('X'), false),
            vec![Action::InsertChar('X')]
        );
    }

    #[test]
    fn alt_espaco_no_insert_troca_para_visual_e_a_proxima_tecla_e_comando() {
        let mut machine = Machine::new();
        machine.handle_key(Key::enter(), false); // Insert
        assert_eq!(
            machine.handle_key(Key::alt_char(' '), false),
            Vec::<Action>::new()
        );
        assert_eq!(machine.state(), ModalState::Visual);
        assert_eq!(
            machine.handle_key(Key::char('j'), false),
            vec![Action::MoveLeft]
        );
    }

    #[test]
    fn prefixo_go_duplo_g_vai_para_inicio_do_buffer() {
        let mut machine = Machine::new();
        assert_eq!(run(&mut machine, "gg"), vec![Action::MoveBufferStart]);
    }

    #[test]
    fn prefixo_go_com_tecla_nao_mapeada_cancela() {
        // Prefixos continuam cancelando numa tecla inválida — só o
        // verbo (§4.3) persiste; docs/keys.md não pede mudar isso.
        let mut machine = Machine::new();
        assert_eq!(run(&mut machine, "gz"), Vec::<Action>::new());
        assert_eq!(machine.state(), ModalState::Visual);
        assert!(machine.pending_hint().is_none());
    }

    #[test]
    fn prefixo_go_l_abre_prompt_de_ir_para_linha() {
        let mut machine = Machine::new();
        assert_eq!(run(&mut machine, "gl"), vec![Action::PromptGotoLine]);
    }

    #[test]
    fn leader_f_abre_prompt_de_busca() {
        let mut machine = Machine::new();
        assert_eq!(run(&mut machine, " f"), vec![Action::PromptSearch]);
        assert_eq!(machine.state(), ModalState::Visual);
    }

    #[test]
    fn selecao_e_area_de_transferencia() {
        let mut machine = Machine::new();
        assert_eq!(
            machine.handle_key(Key::char('s'), false),
            vec![Action::StartSelection]
        );
        assert_eq!(
            machine.handle_key(Key::char('S'), false),
            vec![Action::SelectAll]
        );
        assert_eq!(
            machine.handle_key(Key::char('y'), false),
            vec![Action::Paste]
        );
    }

    #[test]
    fn verbo_com_selecao_ativa_age_na_hora() {
        let mut machine = Machine::new();
        assert_eq!(
            run_sel(&mut machine, "d", true),
            vec![Action::DeleteSelection]
        );
        assert_eq!(machine.state(), ModalState::Visual);
        assert!(machine.pending_hint().is_none());

        let mut machine = Machine::new();
        assert_eq!(run_sel(&mut machine, "c", true), vec![Action::Copy]);

        let mut machine = Machine::new();
        assert_eq!(run_sel(&mut machine, "x", true), vec![Action::Cut]);
    }

    #[test]
    fn verbo_sem_selecao_espera_objeto() {
        let mut machine = Machine::new();
        assert_eq!(run(&mut machine, "d"), Vec::<Action>::new());
        let hint = machine.pending_hint().expect("d deveria deixar uma dica");
        assert_eq!(hint.label, "apagar");
    }

    #[test]
    fn verbo_mais_objeto_horizontal_e_vertical() {
        let mut machine = Machine::new();
        assert_eq!(
            run(&mut machine, "d;"),
            vec![Action::VerbObject(Verb::Delete, Object::CharRight)]
        );

        let mut machine = Machine::new();
        assert_eq!(
            run(&mut machine, "dj"),
            vec![Action::VerbObject(Verb::Delete, Object::CharLeft)]
        );

        let mut machine = Machine::new();
        assert_eq!(
            run(&mut machine, "dJ"),
            vec![Action::VerbObject(Verb::Delete, Object::LineStart)]
        );

        let mut machine = Machine::new();
        assert_eq!(
            run(&mut machine, "d:"),
            vec![Action::VerbObject(Verb::Delete, Object::LineEnd)]
        );

        let mut machine = Machine::new();
        assert_eq!(
            run(&mut machine, "dk"),
            vec![Action::VerbObject(Verb::Delete, Object::LineDown)]
        );

        let mut machine = Machine::new();
        assert_eq!(
            run(&mut machine, "dl"),
            vec![Action::VerbObject(Verb::Delete, Object::LineUp)]
        );

        let mut machine = Machine::new();
        assert_eq!(
            run(&mut machine, "dK"),
            vec![Action::VerbObject(Verb::Delete, Object::BufferEnd)]
        );

        let mut machine = Machine::new();
        assert_eq!(
            run(&mut machine, "dL"),
            vec![Action::VerbObject(Verb::Delete, Object::BufferStart)]
        );
    }

    #[test]
    fn verbo_mais_unidade_mais_direcao() {
        let mut machine = Machine::new();
        assert_eq!(
            run(&mut machine, "dw;"),
            vec![Action::VerbObject(Verb::Delete, Object::WordForward)]
        );

        let mut machine = Machine::new();
        assert_eq!(
            run(&mut machine, "dwj"),
            vec![Action::VerbObject(Verb::Delete, Object::WordBackward)]
        );

        let mut machine = Machine::new();
        assert_eq!(
            run(&mut machine, "dpk"),
            vec![Action::VerbObject(Verb::Delete, Object::ParagraphForward)]
        );

        let mut machine = Machine::new();
        assert_eq!(
            run(&mut machine, "dpl"),
            vec![Action::VerbObject(Verb::Delete, Object::ParagraphBackward)]
        );
    }

    #[test]
    fn verbo_repetido_e_a_linha_atual() {
        let mut machine = Machine::new();
        assert_eq!(
            run(&mut machine, "dd"),
            vec![Action::VerbObject(Verb::Delete, Object::CurrentLine)]
        );

        let mut machine = Machine::new();
        assert_eq!(
            run(&mut machine, "cc"),
            vec![Action::VerbObject(Verb::Copy, Object::CurrentLine)]
        );

        let mut machine = Machine::new();
        assert_eq!(
            run(&mut machine, "xx"),
            vec![Action::VerbObject(Verb::Cut, Object::CurrentLine)]
        );
    }

    #[test]
    fn verbo_mais_a_e_o_arquivo_inteiro() {
        let mut machine = Machine::new();
        assert_eq!(
            run(&mut machine, "da"),
            vec![Action::VerbObject(Verb::Delete, Object::WholeBuffer)]
        );
    }

    #[test]
    fn tecla_invalida_depois_de_verbo_nao_executa_nada_e_persiste() {
        // 'u' (desfazer) não é um objeto válido: o verbo continua
        // pendente, esperando — ao contrário do dispatch antigo, não
        // reprocessa 'u' como comando visual nem cancela.
        let mut machine = Machine::new();
        assert_eq!(run(&mut machine, "du"), Vec::<Action>::new());
        assert!(machine.pending_hint().is_some());

        // uma tecla válida em seguida ainda resolve o verbo pendente.
        assert_eq!(
            run(&mut machine, ";"),
            vec![Action::VerbObject(Verb::Delete, Object::CharRight)]
        );
    }

    #[test]
    fn tecla_invalida_depois_de_verbo_mais_unidade_tambem_persiste() {
        // "d w k": 'k' não é direção válida pra unidade palavra —
        // continua esperando j ou ;.
        let mut machine = Machine::new();
        assert_eq!(run(&mut machine, "dwk"), Vec::<Action>::new());
        assert_eq!(
            run(&mut machine, ";"),
            vec![Action::VerbObject(Verb::Delete, Object::WordForward)]
        );
    }

    #[test]
    fn esc_cancela_verbo_pendente() {
        let mut machine = Machine::new();
        run(&mut machine, "d");
        assert!(machine.pending_hint().is_some());
        assert_eq!(
            machine.handle_key(Key::escape(), false),
            Vec::<Action>::new()
        );
        assert!(machine.pending_hint().is_none());
        // depois do cancelamento, uma tecla normal volta a ser comando.
        assert_eq!(
            machine.handle_key(Key::char(';'), false),
            vec![Action::MoveRight]
        );
    }

    #[test]
    fn esc_cancela_verbo_mais_unidade_pendente() {
        let mut machine = Machine::new();
        run(&mut machine, "dw");
        assert!(machine.pending_hint().is_some());
        machine.handle_key(Key::escape(), false);
        assert!(machine.pending_hint().is_none());
    }

    #[test]
    fn unidade_sozinha_move_o_cursor() {
        let mut machine = Machine::new();
        assert_eq!(run(&mut machine, "w;"), vec![Action::MoveWordForward]);

        let mut machine = Machine::new();
        assert_eq!(run(&mut machine, "wj"), vec![Action::MoveWordBackward]);

        let mut machine = Machine::new();
        assert_eq!(run(&mut machine, "pk"), vec![Action::MoveParagraphForward]);

        let mut machine = Machine::new();
        assert_eq!(run(&mut machine, "pl"), vec![Action::MoveParagraphBackward]);
    }

    #[test]
    fn unidade_sozinha_com_direcao_invalida_cancela() {
        // "w k" não faz sentido pra unidade palavra: cancela, não
        // persiste (sem verbo em jogo).
        let mut machine = Machine::new();
        assert_eq!(run(&mut machine, "wk"), Vec::<Action>::new());
        assert!(machine.pending_hint().is_none());
        assert_eq!(
            machine.handle_key(Key::char('k'), false),
            vec![Action::MoveDown]
        );
    }

    #[test]
    fn repetir_o_ultimo_composto_que_alterou_o_texto() {
        let mut machine = Machine::new();
        run(&mut machine, "dw;"); // d w ; : apaga até a próxima palavra
        assert_eq!(
            run(&mut machine, "."),
            vec![Action::VerbObject(Verb::Delete, Object::WordForward)]
        );
    }

    #[test]
    fn repetir_sem_composto_anterior_nao_faz_nada() {
        let mut machine = Machine::new();
        assert_eq!(run(&mut machine, "."), Vec::<Action>::new());
    }

    #[test]
    fn copiar_nao_fica_disponivel_pro_repetir() {
        // Copiar não altera o texto — "." não deveria repeti-lo.
        let mut machine = Machine::new();
        run(&mut machine, "c;"); // copia 1 caractere
        assert_eq!(run(&mut machine, "."), Vec::<Action>::new());
    }

    #[test]
    fn sequential_captura_e_executa() {
        let mut machine = Machine::new();
        let mut actions = run(&mut machine, "  jj");
        assert_eq!(actions, Vec::<Action>::new());
        actions.extend(machine.handle_key(Key::enter(), false));
        assert_eq!(actions, vec![Action::MoveLeft, Action::MoveLeft]);
        assert_eq!(machine.state(), ModalState::Visual);
    }

    #[test]
    fn sequential_cancela_e_descarta_entrada() {
        let mut machine = Machine::new();
        run(&mut machine, "  jj");
        let actions = machine.handle_key(Key::escape(), false);
        assert_eq!(actions, Vec::<Action>::new());
        assert_eq!(machine.state(), ModalState::Visual);
        assert!(machine.pending_sequence().is_empty());
    }

    #[test]
    fn sequential_backspace_remove_a_ultima_tecla() {
        let mut machine = Machine::new();
        run(&mut machine, "  jk");
        assert_eq!(
            machine.pending_sequence(),
            &[Key::char('j'), Key::char('k')]
        );
        machine.handle_key(Key::backspace(), false);
        assert_eq!(machine.pending_sequence(), &[Key::char('j')]);
    }

    #[test]
    fn sequential_captura_composto_e_resolve_na_execucao() {
        // "d ;" dentro do Sequential não apaga nada ao digitar — só é
        // interpretado (como verbo+objeto) quando a sequência é
        // executada.
        let mut machine = Machine::new();
        run(&mut machine, "  ");
        assert_eq!(run(&mut machine, "d;"), Vec::<Action>::new());
        assert_eq!(
            machine.handle_key(Key::enter(), false),
            vec![Action::VerbObject(Verb::Delete, Object::CharRight)]
        );
    }

    #[test]
    fn teclas_universais_funcionam_em_insert_e_visual() {
        let mut machine = Machine::new();
        machine.handle_key(Key::enter(), false); // Insert
        assert_eq!(
            machine.handle_key(Key::backspace(), false),
            vec![Action::DeleteBackward]
        );

        machine.handle_key(Key::alt_char(' '), false); // Visual
        assert_eq!(
            machine.handle_key(Key::backspace(), false),
            vec![Action::DeleteBackward]
        );
    }

    #[test]
    fn em_sequential_ate_as_teclas_universais_sao_so_capturadas() {
        // Em Insert/Visual, Left sempre move o cursor na hora. Em
        // Sequential nada passa direto — Left também só é acumulado,
        // igual ao catch-all `[t]` do nas-sequential-map.
        let mut machine = Machine::new();
        run(&mut machine, "  ");
        let actions = machine.handle_key(Key::new(KeyCode::Left), false);
        assert_eq!(actions, Vec::<Action>::new());
        assert_eq!(machine.pending_sequence(), &[Key::new(KeyCode::Left)]);
    }

    #[test]
    fn layout_abnt_traduz_antes_de_consultar_o_keymap() {
        // No ABNT2 a posição do ';' do qwerty produz 'ç' — digitar 'ç'
        // deve disparar o mesmo comando que ';' dispara no qwerty.
        let mut qwerty = Machine::new();
        let mut abnt = Machine::with_layout(Layout::QwertyAbnt);

        assert_eq!(
            qwerty.handle_key(Key::char(';'), false),
            abnt.handle_key(Key::char('ç'), false)
        );
        assert_eq!(
            qwerty.handle_key(Key::char(';'), false),
            vec![Action::MoveRight]
        );
    }

    #[test]
    fn layout_abnt_nao_muda_letras_identicas_nos_dois_layouts() {
        // "g g" continua funcionando igual, já que 'g' é o mesmo nos
        // dois layouts.
        let mut machine = Machine::with_layout(Layout::QwertyAbnt);
        assert_eq!(run(&mut machine, "gg"), vec![Action::MoveBufferStart]);
    }
}
