//! A máquina de estados do nas-keys: Insert, Visual e Sequential.
//!
//! Porta a semântica de `nas-state.el` + `nas-map.el` (ver
//! https://github.com/2gab/nas-keys), não só as teclas. Não conhece
//! `nas-editor::Document` nem nenhuma biblioteca de terminal — recebe
//! [`Key`] e devolve [`Action`]s abstratas; quem chama decide como
//! executá-las.
//!
//! ## Escopo desta etapa
//!
//! O nas-keys real tem comandos que o nas ainda não suporta: seleção/região
//! (`s`, `S`), recortar/copiar/colar (`y`, `c`, `x`), os prefixos `g`/`SPC`
//! além de "ir para o início do buffer", busca (`SPC f`), substituição
//! (`SPC r`), trocar de buffer (`b`), abrir arquivo (`o`) e gerência de
//! janelas. Essas teclas existem no nas-keys real mas, por ora, não têm
//! `Action` correspondente aqui — resolvem para nenhuma ação (um "ainda
//! não implementado" silencioso, não um erro), até que `nas-editor`/`nas-term`
//! ganhem as capacidades que elas precisam.
//!
//! ## Atalhos como dados
//!
//! Quais teclas fazem o quê é dado, não código: vem de
//! `keymap/default.toml` (ver [`crate::keymap`]), carregado uma vez em
//! [`Keymap::default_keymap`]. Esta máquina só sabe *como* um estado,
//! prefixo, despacho e o Sequential funcionam — não hardcoda nenhuma
//! tecla específica.

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
    DeleteForward,
    DeleteBackward,
    Undo,
    InsertChar(char),
    InsertNewline,
    /// Pede a quem chama pra abrir uma linha de entrada pedindo um
    /// caminho de arquivo (`o`, find-file).
    PromptOpenFile,
    /// Pede a quem chama pra abrir uma linha de entrada de busca
    /// incremental (`SPC f`, isearch-forward).
    PromptSearch,
}

/// Prefixo ou despacho aguardando a próxima tecla ("g" → go, Espaço →
/// leader, "d" → despacho de apagar, nos nomes do keymap padrão).
/// Equivalente ao que, no Emacs, seria o `read-key` de
/// `nas-define-dispatch` ou a navegação dentro de um keymap de prefixo.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Pending {
    Prefix(String),
    Dispatch(String),
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

    /// Processa uma tecla e devolve as ações resultantes (zero, uma, ou
    /// mais de uma — por exemplo, o despacho de apagar reprocessa a tecla
    /// quando ela não é uma das suas, e executar o Sequential reexecuta
    /// toda a sequência acumulada). A tecla é traduzida do layout físico
    /// configurado para o qwerty canônico antes de qualquer outra coisa
    /// — o keymap é sempre consultado em termos de qwerty.
    pub fn handle_key(&mut self, key: Key) -> Vec<Action> {
        let key = layout::to_qwerty(&self.layout_table, key);
        match self.state {
            ModalState::Insert => self.handle_insert(key),
            ModalState::Visual => self.handle_visual(key),
            ModalState::Sequential => self.handle_sequential(key),
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
    fn handle_visual(&mut self, key: Key) -> Vec<Action> {
        if let Some(pending) = self.pending.take() {
            return self.resolve_pending(pending, key);
        }

        if let Some(binding) = self.keymap.visual.get(&key).cloned() {
            return self.run_binding(binding);
        }

        universal_action(key).into_iter().collect()
    }

    /// Executa uma ligação do keymap que não depende da tecla em si
    /// (trocas de estado e entrada em prefixo/despacho já sabem qual
    /// prefixo/despacho são pelo nome).
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
            Binding::Dispatch(name) => {
                self.pending = Some(Pending::Dispatch(name));
                Vec::new()
            }
        }
    }

    fn resolve_pending(&mut self, pending: Pending, key: Key) -> Vec<Action> {
        match pending {
            Pending::Prefix(name) => {
                let binding = self
                    .keymap
                    .prefixes
                    .get(&name)
                    .and_then(|bindings| bindings.get(&key))
                    .cloned();
                match binding {
                    Some(binding) => self.run_binding(binding),
                    None => Vec::new(),
                }
            }
            Pending::Dispatch(name) => {
                let (matched, fallback) = {
                    let dispatch = self
                        .keymap
                        .dispatches
                        .get(&name)
                        .expect("despacho pendente deveria existir no keymap");
                    (dispatch.bindings.get(&key).copied(), dispatch.fallback)
                };
                match matched {
                    Some(action) => vec![action],
                    None => {
                        // Fiel ao fallback de `nas-define-dispatch`: executa
                        // o fallback e reprocessa a tecla como um comando
                        // visual normal (equivalente a
                        // `push key unread-command-events`).
                        let mut actions = vec![fallback];
                        actions.extend(self.handle_visual(key));
                        actions
                    }
                }
            }
        }
    }

    /// Em sequential, nada passa direto: RET executa, ESC cancela, DEL
    /// remove a última tecla acumulada, e qualquer outra tecla (incluindo
    /// setas, Delete, Tab — tudo) é só acumulada, igual ao catch-all `[t]`
    /// do nas-sequential-map.
    fn handle_sequential(&mut self, key: Key) -> Vec<Action> {
        match key.code {
            KeyCode::Enter if !key.alt => self.execute_sequential(),
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

    fn execute_sequential(&mut self) -> Vec<Action> {
        let keys = std::mem::take(&mut self.sequence);
        self.enter_visual();
        let mut actions = Vec::new();
        for key in keys {
            actions.extend(self.handle_visual(key));
        }
        actions
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
        let mut actions = Vec::new();
        for key in keys(s) {
            actions.extend(machine.handle_key(key));
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
        machine.handle_key(Key::enter());
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
        machine.handle_key(Key::enter());
        assert_eq!(machine.status_label(), "I");

        machine.handle_key(Key::alt_char(' '));
        assert_eq!(machine.status_label(), "V");

        run(&mut machine, " "); // entra no leader
        machine.handle_key(Key::char(' ')); // resolve o leader -> sequential
        assert_eq!(machine.status_label(), "S");

        machine.handle_key(Key::char('j'));
        machine.handle_key(Key::char('k'));
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
            machine.handle_key(Key::char('J')),
            vec![Action::MoveLineStart]
        );
        assert_eq!(
            machine.handle_key(Key::char(':')),
            vec![Action::MoveLineEnd]
        );
        assert_eq!(
            machine.handle_key(Key::char('L')),
            vec![Action::MoveBufferStart]
        );
        assert_eq!(
            machine.handle_key(Key::char('K')),
            vec![Action::MoveBufferEnd]
        );
    }

    #[test]
    fn enter_em_visual_troca_para_insert_em_vez_de_inserir_texto() {
        let mut machine = Machine::new();
        assert_eq!(machine.handle_key(Key::enter()), Vec::<Action>::new());
        assert_eq!(machine.state(), ModalState::Insert);
        // a próxima tecla já insere texto, não é mais um comando visual
        assert_eq!(
            machine.handle_key(Key::char('X')),
            vec![Action::InsertChar('X')]
        );
    }

    #[test]
    fn alt_espaco_no_insert_troca_para_visual_e_a_proxima_tecla_e_comando() {
        let mut machine = Machine::new();
        machine.handle_key(Key::enter()); // Insert
        assert_eq!(machine.handle_key(Key::alt_char(' ')), Vec::<Action>::new());
        assert_eq!(machine.state(), ModalState::Visual);
        assert_eq!(machine.handle_key(Key::char('j')), vec![Action::MoveLeft]);
    }

    #[test]
    fn prefixo_go_duplo_g_vai_para_inicio_do_buffer() {
        let mut machine = Machine::new();
        assert_eq!(run(&mut machine, "gg"), vec![Action::MoveBufferStart]);
    }

    #[test]
    fn prefixo_go_com_tecla_nao_mapeada_nao_faz_nada() {
        let mut machine = Machine::new();
        assert_eq!(run(&mut machine, "gz"), Vec::<Action>::new());
        assert_eq!(machine.state(), ModalState::Visual);
    }

    #[test]
    fn o_abre_prompt_de_arquivo() {
        let mut machine = Machine::new();
        assert_eq!(run(&mut machine, "o"), vec![Action::PromptOpenFile]);
        // "o" não é um prefixo: o estado não muda, quem decide o que
        // fazer com o prompt é quem chama (nas-term).
        assert_eq!(machine.state(), ModalState::Visual);
    }

    #[test]
    fn leader_f_abre_prompt_de_busca() {
        let mut machine = Machine::new();
        assert_eq!(run(&mut machine, " f"), vec![Action::PromptSearch]);
        assert_eq!(machine.state(), ModalState::Visual);
    }

    #[test]
    fn despacho_de_apagar_direto() {
        let mut machine = Machine::new();
        assert_eq!(run(&mut machine, "dd"), vec![Action::DeleteForward]);

        let mut machine = Machine::new();
        assert_eq!(run(&mut machine, "d;"), vec![Action::DeleteForward]);

        let mut machine = Machine::new();
        assert_eq!(run(&mut machine, "dj"), vec![Action::DeleteBackward]);
    }

    #[test]
    fn despacho_de_apagar_fallback_so_apaga_quando_a_tecla_nao_faz_nada() {
        // 'n' não é um comando visual nem uma tecla do despacho de apagar:
        // o fallback apaga pra frente e reprocessa 'n', que não faz nada.
        let mut machine = Machine::new();
        assert_eq!(run(&mut machine, "dn"), vec![Action::DeleteForward]);
    }

    #[test]
    fn despacho_de_apagar_fallback_reprocessa_a_tecla_como_comando_visual() {
        // 'u' (desfazer) não é uma tecla do despacho de apagar: cai no
        // fallback, que apaga pra frente E reprocessa 'u' como comando
        // visual normal — as duas ações devem aparecer.
        let mut machine = Machine::new();
        assert_eq!(
            run(&mut machine, "du"),
            vec![Action::DeleteForward, Action::Undo]
        );
    }

    #[test]
    fn sequential_captura_e_executa() {
        let mut machine = Machine::new();
        let mut actions = run(&mut machine, "  jj");
        assert_eq!(actions, Vec::<Action>::new());
        actions.extend(machine.handle_key(Key::enter()));
        assert_eq!(actions, vec![Action::MoveLeft, Action::MoveLeft]);
        assert_eq!(machine.state(), ModalState::Visual);
    }

    #[test]
    fn sequential_cancela_e_descarta_entrada() {
        let mut machine = Machine::new();
        run(&mut machine, "  jj");
        let actions = machine.handle_key(Key::escape());
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
        machine.handle_key(Key::backspace());
        assert_eq!(machine.pending_sequence(), &[Key::char('j')]);
    }

    #[test]
    fn sequential_captura_despacho_e_resolve_na_execucao() {
        // "d j" dentro do Sequential não apaga nada ao digitar — só é
        // interpretado (como despacho de apagar pra trás) quando a
        // sequência é executada.
        let mut machine = Machine::new();
        run(&mut machine, "  ");
        assert_eq!(run(&mut machine, "dj"), Vec::<Action>::new());
        assert_eq!(
            machine.handle_key(Key::enter()),
            vec![Action::DeleteBackward]
        );
    }

    #[test]
    fn teclas_universais_funcionam_em_insert_e_visual() {
        let mut machine = Machine::new();
        machine.handle_key(Key::enter()); // Insert
        assert_eq!(
            machine.handle_key(Key::backspace()),
            vec![Action::DeleteBackward]
        );

        machine.handle_key(Key::alt_char(' ')); // Visual
        assert_eq!(
            machine.handle_key(Key::backspace()),
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
        let actions = machine.handle_key(Key::new(KeyCode::Left));
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
            qwerty.handle_key(Key::char(';')),
            abnt.handle_key(Key::char('ç'))
        );
        assert_eq!(qwerty.handle_key(Key::char(';')), vec![Action::MoveRight]);
    }

    #[test]
    fn layout_abnt_nao_muda_letras_identicas_nos_dois_layouts() {
        // "g g" continua funcionando igual, já que 'g' é o mesmo nos
        // dois layouts.
        let mut machine = Machine::with_layout(Layout::QwertyAbnt);
        assert_eq!(run(&mut machine, "gg"), vec![Action::MoveBufferStart]);
    }
}
