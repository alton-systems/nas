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
//! [`Action`]s abstratas; quem chama decide como executá-las. A classe
//! sistema (Ctrl + tecla, docs/keys.md §5.7) é resolvida por
//! [`Machine::system_action`], separado da máquina modal — ela vale
//! em qualquer estado, então não faz sentido passar pelo
//! Insert/Visual/Sequential.
//!
//! ## Escopo desta etapa
//!
//! `SPC r` (substituir) e `SPC h` (ajuda) não têm binding no keymap
//! padrão — substituir precisa de um fluxo interativo de confirmação
//! por ocorrência que ainda não existe, e ajuda não tem pra onde
//! apontar ainda. A gerência de janelas (classe alternar,
//! `Alt+direção`) também não está implementada — `nas-term` não tem
//! painéis múltiplos. Ver `docs/keys.md` para os detalhes.

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
    /// O nome usado em `[commands]` do keymap — também o nome canônico
    /// que o journal de histórico usa pra registrar o comando de um
    /// verbo+objeto (`"delete:word-forward"`), docs/decisions/0005 §1.
    pub fn command_name(&self) -> &'static str {
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
    /// O nome usado em `[commands]` do keymap — também o nome canônico
    /// que o journal de histórico usa como a parte "objeto" do comando
    /// de um verbo+objeto, docs/decisions/0005 §1.
    pub fn command_name(&self) -> &'static str {
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

/// Um comando da classe sistema (docs/keys.md §5.7): Ctrl + tecla, em
/// qualquer estado — abrir/salvar/fechar arquivo, trocar de buffer,
/// sair do editor, ou um comando de histórico/checkpoint sob o
/// prefixo `Ctrl+t` ("tempo"). Resolvido por [`Machine::system_action`]
/// (direto) ou [`Machine::system_prefix_action`] (depois de
/// [`Machine::system_prefix`] indicar um prefixo pendente),
/// independente do estado (Insert/Visual/Sequential) e de qualquer
/// prefixo/verbo/unidade pendente da máquina modal — a classe sistema
/// é o editor como *programa*, não uma edição de texto.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SystemAction {
    Save,
    Quit,
    OpenFile,
    SwitchBuffer,
    CloseBuffer,
    /// `Ctrl+r` — recarrega o buffer atual a partir do disco,
    /// descartando qualquer edição local não salva (pede confirmação
    /// se houver uma) — docs/decisions/0005 §1.
    ReloadBuffer,
    /// `Ctrl+t n` — cria um checkpoint nomeado do projeto.
    CheckpointNew,
    /// `Ctrl+t l` — lista os checkpoints do projeto.
    CheckpointList,
    /// `Ctrl+t e` — diff entre os dois checkpoints mais recentes.
    CheckpointDiff,
    /// `Ctrl+t r` — restaura um checkpoint.
    CheckpointRestore,
    /// `Ctrl+t a` — autoria (histórico de edições) do arquivo atual.
    CheckpointBlame,
    /// `Ctrl+t z` — apaga o histórico do projeto, com confirmação.
    HistoryDelete,
    /// `Ctrl+t m` — abre o histórico de mensagens (a echo area,
    /// docs/keys.md regra 8) numa visualização — "mensagens".
    MessageHistory,
}

impl SystemAction {
    /// O nome usado em `[commands]` do keymap.
    pub(crate) fn command_name(&self) -> &'static str {
        match self {
            SystemAction::Save => "save",
            SystemAction::Quit => "quit",
            SystemAction::OpenFile => "open-file",
            SystemAction::SwitchBuffer => "switch-buffer",
            SystemAction::CloseBuffer => "close-buffer",
            SystemAction::ReloadBuffer => "reload-buffer",
            SystemAction::CheckpointNew => "checkpoint-new",
            SystemAction::CheckpointList => "checkpoint-list",
            SystemAction::CheckpointDiff => "checkpoint-diff",
            SystemAction::CheckpointRestore => "checkpoint-restore",
            SystemAction::CheckpointBlame => "checkpoint-blame",
            SystemAction::HistoryDelete => "history-delete",
            SystemAction::MessageHistory => "message-history",
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
    /// Desfaz a seleção ativa, sem apagar nada — `Esc` sem nenhum
    /// verbo/prefixo/unidade pendente (docs/keys.md §3, "Esc
    /// cancela").
    ClearSelection,
    /// Abre a paleta de comandos (`Espaço p`, ou `Shift+Espaço` — este
    /// último tratado à parte em `nas-term`, nunca aqui: `Key` não
    /// modela Shift, então não há como diferenciar `Shift+Espaço` de
    /// `Espaço` dentro do vocabulário do nas-keys). Quem chama decide
    /// como mostrar e executar a paleta — a máquina só sabe que foi
    /// pedida.
    OpenCommandPalette,
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
            Action::OpenCommandPalette => "open-command-palette",
            Action::DeleteForward
            | Action::DeleteBackward
            | Action::InsertChar(_)
            | Action::InsertNewline
            | Action::Copy
            | Action::Cut
            | Action::DeleteSelection
            | Action::VerbObject(_, _)
            | Action::ClearSelection => return None,
        })
    }

    /// Se este movimento é um objeto **horizontal** (docs/keys.md
    /// §5.3) — o objeto correspondente, pra que `d`/`c`/`x` aceitem a
    /// mesma tecla que já move o cursor assim. `None` para qualquer
    /// ação que não seja um movimento horizontal (inclusive os
    /// verticais). Único lugar onde essa classificação existe: o
    /// `[objects.horizontal]`/`[objects.vertical]` do keymap antigo
    /// foram removidos a favor de derivar isso direto do `[visual]` —
    /// mudar a tecla de um movimento no TOML muda o objeto junto, sem
    /// precisar editar dois lugares.
    pub(crate) fn as_horizontal_object(&self) -> Option<Object> {
        match self {
            Action::MoveLeft => Some(Object::CharLeft),
            Action::MoveRight => Some(Object::CharRight),
            Action::MoveLineStart => Some(Object::LineStart),
            Action::MoveLineEnd => Some(Object::LineEnd),
            _ => None,
        }
    }

    /// O objeto **vertical** correspondente, se este movimento for um
    /// — mesma ideia de [`Action::as_horizontal_object`], pro eixo
    /// vertical (movimento por linha).
    pub(crate) fn as_vertical_object(&self) -> Option<Object> {
        match self {
            Action::MoveDown => Some(Object::LineDown),
            Action::MoveUp => Some(Object::LineUp),
            Action::MoveBufferEnd => Some(Object::BufferEnd),
            Action::MoveBufferStart => Some(Object::BufferStart),
            _ => None,
        }
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
    /// Um verbo que viu uma unidade (`w`/`W`) e agora espera a
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
/// pendente (verbo, unidade, prefixo da máquina modal, ou prefixo de
/// sistema) — docs/keys.md regra 8 (descobrível).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingHint {
    /// A tecla (ou teclas) que levou a este pendente — ex.: `"d"`,
    /// `"g"`, `"d w"` (verbo seguido de unidade), `"Ctrl+t"`. Vazia se
    /// não houver uma tecla única (não deveria acontecer em uso
    /// normal, só uma salvaguarda).
    pub keys: String,
    pub label: String,
    /// Tecla, descrição, e se é uma ação destrutiva (só o prefixo de
    /// sistema usa o terceiro campo — a máquina modal nunca marca
    /// nada como destrutivo).
    pub entries: Vec<(Key, String, bool)>,
    /// `true` se `entries` já está na ordem que deveria ser mostrada
    /// (a ordem declarada no TOML, do prefixo de sistema) — `false`
    /// se vem de um `HashMap` sem ordem significativa (a máquina
    /// modal), e quem exibe precisa alfabetizar por conta própria pra
    /// ficar determinístico.
    pub ordered: bool,
}

/// Um comando do catálogo da paleta de comandos (`Espaço p`/`Shift+Espaço`,
/// docs/keys.md "paleta de comandos") — **todo** nome de `[commands]`
/// do keymap, menos os próprios prefixos (`go`, `leader`, `tempo`), já
/// com o keymap do usuário aplicado. Produzido por
/// [`Machine::command_catalog`], com as teclas traduzidas pro layout
/// físico do usuário (pra exibição — ver [`crate::layout::to_physical`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogEntry {
    /// O nome usado em `[commands]` do keymap — também o que entra em
    /// `App::recent_commands` quando executado pela paleta.
    pub name: String,
    pub description: String,
    /// Vazio se o comando não tem (mais) um atalho que o alcance
    /// sozinho a partir do estado Visual — por natureza (ex.:
    /// `current-line`, só "o verbo repetido", nunca uma tecla própria)
    /// ou porque o keymap do usuário removeu o único vínculo que
    /// existia (`chave = false`, docs/config.md) — em qualquer um dos
    /// dois casos, o comando continua na paleta (docs/keys.md).
    pub keys: Vec<Key>,
    /// `true` se a **primeira** tecla de `keys` é Ctrl (classe sistema) —
    /// nunca a segunda: `Ctrl+t n` é Ctrl só na tecla que abre o
    /// prefixo, nunca na que o completa.
    pub ctrl: bool,
    /// Ação destrutiva (hoje só `history-delete`, `Ctrl+t z`) — a
    /// paleta marca essas com a mesma cor de alerta que o which-key já
    /// usa (`ui::which_key_lines`, `HINT_DESTRUCTIVE_COLOR`).
    pub destructive: bool,
    /// Como executar este comando **sem** passar por nenhuma tecla —
    /// usado só quando `keys` está vazio (quem chama prefere sempre
    /// repetir a tecla de verdade, quando existe uma: isso preserva
    /// nuances como "só arma o pendente se não houver seleção ativa
    /// agora", que `dispatch`/dispatch de sistema não sabem decidir
    /// sozinhos). `None` só para os nomes de **objeto** puro (`char-left`,
    /// `whole-buffer` fora de um verbo pendente, `current-line`, ...) —
    /// esses não têm execução autônoma coerente, por não serem um
    /// `Binding` nem uma `SystemAction` nomeada; só fazem sentido como
    /// o complemento de um verbo já pendente.
    pub target: Option<CommandTarget>,
}

/// O que executar quando um comando do catálogo não tem tecla
/// nenhuma pra "repetir" — ver [`CatalogEntry::target`]. As duas
/// variantes espelham as duas classes de comando que o keymap conhece
/// (modal, resolvida por [`Machine::dispatch`]; sistema/Ctrl,
/// resolvida por quem chama — `App::dispatch_system_action` em
/// `nas-term` — já que `Machine` não sabe nada de classe sistema).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandTarget {
    Modal(Binding),
    System(SystemAction),
}

/// A máquina modal nunca marca nada como destrutivo — só o prefixo de
/// sistema (`Ctrl+t z`) tem esse conceito. Completa o terceiro campo
/// com `false` pra encaixar no formato comum de `PendingHint::entries`.
fn without_destructive(entries: Vec<(Key, String)>) -> Vec<(Key, String, bool)> {
    entries
        .into_iter()
        .map(|(key, desc)| (key, desc, false))
        .collect()
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

    /// Troca o keymap em uso — ADR 0007-configuracao-em-camadas.md,
    /// Parte 2, "Recarregar": depois de validar um `keymap.toml`
    /// novo por completo, a troca é só isto, sem reconstruir a
    /// `Machine` (que perderia o estado modal atual, a sequência
    /// pendente, etc.).
    pub fn set_keymap(&mut self, keymap: Keymap) {
        self.keymap = keymap;
    }

    /// Troca o layout físico de teclado em uso — mesma ideia de
    /// `set_keymap`, pro `keyboard_layout` de `config.toml`
    /// (ADR 0007-configuracao-em-camadas.md) também valer sem
    /// reconstruir a `Machine` ao recarregar.
    pub fn set_layout(&mut self, layout: Layout) {
        self.layout_table = layout.translation_table();
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
                keys: self.key_text_for_prefix(name),
                label: self.keymap.describe(name),
                entries: without_destructive(self.keymap.hint_for_prefix(name)),
                ordered: false,
            }),
            Pending::Verb(verb) => Some(PendingHint {
                keys: self.key_text_for_verb(*verb),
                label: self.keymap.describe(verb.command_name()),
                entries: without_destructive(self.keymap.hint_for_verb()),
                ordered: false,
            }),
            Pending::VerbUnit(verb, unit) => Some(PendingHint {
                keys: format!(
                    "{} {}",
                    self.key_text_for_verb(*verb),
                    self.key_text_for_unit(*unit)
                ),
                label: format!(
                    "{} {}",
                    self.keymap.describe(verb.command_name()),
                    self.keymap.describe(unit.command_name())
                ),
                entries: without_destructive(self.keymap.hint_for_unit(*unit)),
                ordered: false,
            }),
            Pending::Unit(unit) => Some(PendingHint {
                keys: self.key_text_for_unit(*unit),
                label: self.keymap.describe(unit.command_name()),
                entries: without_destructive(self.keymap.hint_for_unit(*unit)),
                ordered: false,
            }),
        }
    }

    /// O catálogo da paleta de comandos (docs/keys.md "paleta de
    /// comandos") — **todo** nome de `[commands]` do keymap, menos os
    /// próprios prefixos (`go`, `leader`, `tempo`), com o atalho que o
    /// alcança sozinho a partir do Visual, se houver um, e as teclas
    /// já traduzidas pro layout físico em uso (pra exibição — a
    /// máquina continua resolvendo tudo em qwerty canônico por
    /// dentro). Determinístico: mesma entrada, mesma ordem, sempre
    /// (ver `Keymap::catalog`) — quem chama decide a ordem de exibição
    /// (recência, busca) por conta própria.
    pub fn command_catalog(&self) -> Vec<CatalogEntry> {
        self.keymap
            .catalog()
            .into_iter()
            .map(|mut entry| {
                for key in &mut entry.keys {
                    *key = layout::to_physical(&self.layout_table, *key);
                }
                entry
            })
            .collect()
    }

    /// Executa diretamente o que a tecla ligada a `binding` faria —
    /// usado pela paleta de comandos quando `CatalogEntry::keys` está
    /// vazio (nenhuma tecla pra "repetir", ver `CommandTarget::Modal`).
    /// Mesmo efeito de [`Machine::handle_key`] resolver pra este
    /// `binding`, com um desvio: um verbo sempre arma o pendente,
    /// nunca age na hora sobre uma seleção — sem uma tecla de verdade
    /// não há como saber se existe uma seleção ativa *agora* (ver
    /// `run_binding`); isso só importa no caso raro de o usuário ter
    /// removido a única tecla de um verbo do keymap.
    pub fn dispatch(&mut self, binding: Binding) -> Vec<Action> {
        self.run_binding(binding)
    }

    fn key_text_for_prefix(&self, name: &str) -> String {
        self.keymap
            .key_for_prefix(name)
            .map(|k| k.to_string())
            .unwrap_or_default()
    }

    fn key_text_for_verb(&self, verb: Verb) -> String {
        self.keymap
            .key_for_verb(verb)
            .map(|k| k.to_string())
            .unwrap_or_default()
    }

    fn key_text_for_unit(&self, unit: Unit) -> String {
        self.keymap
            .key_for_unit(unit)
            .map(|k| k.to_string())
            .unwrap_or_default()
    }

    /// Resolve uma tecla da classe sistema (Ctrl + tecla, docs/keys.md
    /// §5.7) — `None` se a tecla não estiver em `[system]`. Puro: não
    /// muda estado nem consome nenhum prefixo/verbo/unidade pendente
    /// — a classe sistema vale em qualquer estado, independente do
    /// que a máquina modal esteja fazendo. A tecla ainda passa pela
    /// tradução de layout físico, como qualquer outra (docs/keys.md
    /// regra 6).
    pub fn system_action(&self, key: Key) -> Option<SystemAction> {
        let key = layout::to_qwerty(&self.layout_table, key);
        self.keymap.system_action(&key)
    }

    /// O nome do prefixo de sistema que `key` abre (`Ctrl+t` →
    /// `"tempo"`, docs/keys.md §5.7), se houver um — `None` se `key`
    /// resolve direto por [`Machine::system_action`] ou não existe em
    /// `[system]`. Tão puro/estável quanto `system_action`: só
    /// consulta o keymap, não muda estado. Quem chama (`nas-term`) é
    /// responsável por guardar que um prefixo está pendente e
    /// resolver a tecla seguinte com [`Machine::system_prefix_action`]
    /// — a máquina modal em si não sabe de prefixos de sistema.
    pub fn system_prefix(&self, key: Key) -> Option<String> {
        let key = layout::to_qwerty(&self.layout_table, key);
        self.keymap.system_prefix_key(&key).map(str::to_string)
    }

    /// O comando ligado a `key` dentro do prefixo de sistema `name`
    /// (ex.: depois de `Ctrl+t`, `n` → `CheckpointNew`).
    pub fn system_prefix_action(&self, name: &str, key: Key) -> Option<SystemAction> {
        let key = layout::to_qwerty(&self.layout_table, key);
        self.keymap.system_prefix_action(name, &key)
    }

    /// Rótulo e teclas válidas pra um prefixo de sistema pendente —
    /// mesma ideia de [`Machine::pending_hint`], mas pro `Ctrl+t` da
    /// classe sistema, que a máquina modal não controla.
    pub fn system_prefix_hint(&self, name: &str) -> PendingHint {
        let keys = self
            .keymap
            .key_for_system_prefix(name)
            .map(|k| format!("Ctrl+{k}"))
            .unwrap_or_default();
        PendingHint {
            keys,
            label: self.keymap.describe(name),
            entries: self.keymap.hint_for_system_prefix(name),
            ordered: true,
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
        // Esc cancela qualquer verbo, prefixo ou unidade pendente; sem
        // nada pendente, desfaz uma seleção ativa — docs/keys.md §3
        // (Esc = cancelar). Um cancela de cada vez: se há algo
        // pendente E uma seleção ativa ao mesmo tempo (ex.: seleção
        // iniciada antes de um prefixo `g`), o primeiro Esc só cancela
        // o pendente, um segundo desfaz a seleção.
        if matches!(key.code, KeyCode::Escape) && !key.alt {
            if self.pending.is_some() {
                self.pending = None;
                return Vec::new();
            }
            if has_selection {
                return vec![Action::ClearSelection];
            }
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
            // Uma unidade (`w`, `W`): o objeto ainda não está completo,
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
/// parágrafo têm tabela de unidade no keymap. `pub(crate)` porque
/// `Keymap::catalog` (keymap.rs) também precisa dela, pra saber que
/// "unidade palavra/parágrafo + direção" (`w ;`, `w j`, `W k`, `W l`)
/// alcança `move-word-*`/`move-paragraph-*` na paleta de comandos —
/// a mesma resolução que `Machine::resolve_pending` já faz em tempo
/// de tecla.
pub(crate) fn plain_motion_for(object: Object) -> Option<Action> {
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
            run(&mut machine, "dWk"),
            vec![Action::VerbObject(Verb::Delete, Object::ParagraphForward)]
        );

        let mut machine = Machine::new();
        assert_eq!(
            run(&mut machine, "dWl"),
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
    fn esc_desfaz_a_selecao_ativa_quando_nao_ha_nada_pendente() {
        let mut machine = Machine::new();
        assert_eq!(
            machine.handle_key(Key::escape(), true),
            vec![Action::ClearSelection]
        );
    }

    #[test]
    fn esc_sem_pendente_nem_selecao_nao_faz_nada() {
        let mut machine = Machine::new();
        assert_eq!(
            machine.handle_key(Key::escape(), false),
            Vec::<Action>::new()
        );
    }

    #[test]
    fn esc_cancela_o_pendente_antes_de_desfazer_a_selecao() {
        // Com um prefixo pendente E uma seleção ativa ao mesmo tempo
        // (ex.: seleção marcada antes de "g"), o primeiro Esc só
        // cancela o prefixo — a seleção precisa de outro Esc.
        let mut machine = Machine::new();
        run(&mut machine, "g");
        assert!(machine.pending_hint().is_some());

        assert_eq!(
            machine.handle_key(Key::escape(), true),
            Vec::<Action>::new()
        );
        assert!(machine.pending_hint().is_none());

        assert_eq!(
            machine.handle_key(Key::escape(), true),
            vec![Action::ClearSelection]
        );
    }

    #[test]
    fn unidade_sozinha_move_o_cursor() {
        let mut machine = Machine::new();
        assert_eq!(run(&mut machine, "w;"), vec![Action::MoveWordForward]);

        let mut machine = Machine::new();
        assert_eq!(run(&mut machine, "wj"), vec![Action::MoveWordBackward]);

        let mut machine = Machine::new();
        assert_eq!(run(&mut machine, "Wk"), vec![Action::MoveParagraphForward]);

        let mut machine = Machine::new();
        assert_eq!(run(&mut machine, "Wl"), vec![Action::MoveParagraphBackward]);
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
    fn p_sozinho_nao_faz_nada() {
        // A unidade parágrafo é "W" (Shift+w) agora, não "p" — regra 3
        // (Shift intensifica), que passa a valer pra unidades também:
        // w = palavra, W = parágrafo, uma escala maior. "p" sozinho
        // fica reservado, sem binding em [visual].
        let mut machine = Machine::new();
        assert_eq!(run(&mut machine, "p"), Vec::<Action>::new());
        assert!(machine.pending_hint().is_none());
        assert_eq!(machine.state(), ModalState::Visual);

        // nem mesmo entra em espera: a tecla seguinte é um comando
        // normal, não uma direção de unidade.
        assert_eq!(
            machine.handle_key(Key::char(';'), false),
            vec![Action::MoveRight]
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
    fn sistema_resolve_em_qualquer_estado() {
        let mut machine = Machine::new();
        assert_eq!(
            machine.system_action(Key::char('s')),
            Some(SystemAction::Save)
        );

        // continua resolvendo em Insert...
        machine.handle_key(Key::enter(), false);
        assert_eq!(machine.state(), ModalState::Insert);
        assert_eq!(
            machine.system_action(Key::char('q')),
            Some(SystemAction::Quit)
        );

        // ...e em Sequential, sem ser capturado pela sequência.
        machine.handle_key(Key::alt_char(' '), false); // volta pro Visual
        run(&mut machine, "  "); // entra no Sequential
        assert_eq!(machine.state(), ModalState::Sequential);
        assert_eq!(
            machine.system_action(Key::char('k')),
            Some(SystemAction::CloseBuffer)
        );
        // de fato não foi capturado: a sequência continua vazia.
        assert!(machine.pending_sequence().is_empty());
    }

    #[test]
    fn prefixo_de_sistema_ctrl_t_resolve_as_sub_teclas_de_tempo() {
        let machine = Machine::new();

        assert_eq!(machine.system_action(Key::char('t')), None);
        assert_eq!(
            machine.system_prefix(Key::char('t')),
            Some("tempo".to_string())
        );
        assert_eq!(machine.system_prefix(Key::char('s')), None);

        assert_eq!(
            machine.system_prefix_action("tempo", Key::char('n')),
            Some(SystemAction::CheckpointNew)
        );
        assert_eq!(
            machine.system_prefix_action("tempo", Key::char('z')),
            Some(SystemAction::HistoryDelete)
        );
        assert_eq!(machine.system_prefix_action("tempo", Key::char('x')), None);
    }

    #[test]
    fn prefixo_de_sistema_tambem_resolve_em_qualquer_estado() {
        let mut machine = Machine::new();
        machine.handle_key(Key::enter(), false); // Visual -> Insert
        assert_eq!(machine.state(), ModalState::Insert);

        assert_eq!(
            machine.system_prefix(Key::char('t')),
            Some("tempo".to_string())
        );
        assert_eq!(
            machine.system_prefix_action("tempo", Key::char('l')),
            Some(SystemAction::CheckpointList)
        );
    }

    #[test]
    fn sistema_nao_resolve_tecla_fora_de_system() {
        let machine = Machine::new();
        assert_eq!(machine.system_action(Key::char('j')), None);
    }

    #[test]
    fn sistema_nao_e_afetado_por_verbo_pendente() {
        let mut machine = Machine::new();
        run(&mut machine, "d"); // 'd' pendente, esperando objeto
        assert!(machine.pending_hint().is_some());
        assert_eq!(
            machine.system_action(Key::char('s')),
            Some(SystemAction::Save)
        );
        // o verbo continua pendente — consultar a classe sistema não
        // interfere no estado da máquina modal.
        assert!(machine.pending_hint().is_some());
    }

    #[test]
    fn command_catalog_traduz_as_teclas_pro_layout_fisico_em_uso() {
        let qwerty = Machine::new();
        let abnt = Machine::with_layout(Layout::QwertyAbnt);

        let find = |catalog: &[CatalogEntry], name: &str| {
            catalog
                .iter()
                .find(|entry| entry.name == name)
                .cloned()
                .unwrap_or_else(|| panic!("{name} não encontrado no catálogo"))
        };

        // "move-right" é "j" no qwerty... não, é ";" — na posição do
        // qwerty onde o ABNT2 tem 'ç'. A exibição deveria mostrar 'ç'
        // pro usuário ABNT2, nunca ';' (docs/keys.md, "paleta de
        // comandos").
        let qwerty_entry = find(&qwerty.command_catalog(), "move-right");
        assert_eq!(qwerty_entry.keys, vec![Key::char(';')]);

        let abnt_entry = find(&abnt.command_catalog(), "move-right");
        assert_eq!(abnt_entry.keys, vec![Key::char('ç')]);

        // uma tecla que não difere entre os dois layouts (letras)
        // continua igual.
        let qwerty_delete = find(&qwerty.command_catalog(), "delete");
        let abnt_delete = find(&abnt.command_catalog(), "delete");
        assert_eq!(qwerty_delete.keys, abnt_delete.keys);
    }

    #[test]
    fn layout_abnt_nao_muda_letras_identicas_nos_dois_layouts() {
        // "g g" continua funcionando igual, já que 'g' é o mesmo nos
        // dois layouts.
        let mut machine = Machine::with_layout(Layout::QwertyAbnt);
        assert_eq!(run(&mut machine, "gg"), vec![Action::MoveBufferStart]);
    }
}
