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

use crate::key::{Key, KeyCode};

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
}

/// Prefixo ou despacho aguardando a próxima tecla ("g" → go, Espaço →
/// leader, "d" → despacho de apagar). Equivalente ao que, no Emacs, seria
/// o `read-key` de `nas-define-dispatch` ou a navegação dentro de um
/// keymap de prefixo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pending {
    None,
    Go,
    Leader,
    Delete,
}

/// A máquina de estados. Testável sem terminal: alimente com [`Key`]s e
/// confira as [`Action`]s e o estado resultantes.
pub struct Machine {
    state: ModalState,
    pending: Pending,
    sequence: Vec<Key>,
}

impl Default for Machine {
    fn default() -> Self {
        Self::new()
    }
}

impl Machine {
    /// Começa em Visual, como o nas-keys real (`nas--enable` chama
    /// `nas-enter-visual`).
    pub fn new() -> Self {
        Self {
            state: ModalState::Visual,
            pending: Pending::None,
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
    /// toda a sequência acumulada).
    pub fn handle_key(&mut self, key: Key) -> Vec<Action> {
        match self.state {
            ModalState::Insert => self.handle_insert(key),
            ModalState::Visual => self.handle_visual(key),
            ModalState::Sequential => self.handle_sequential(key),
        }
    }

    fn enter_insert(&mut self) {
        self.state = ModalState::Insert;
        self.pending = Pending::None;
    }

    fn enter_visual(&mut self) {
        self.state = ModalState::Visual;
        self.pending = Pending::None;
    }

    fn enter_sequential(&mut self) {
        self.sequence.clear();
        self.state = ModalState::Sequential;
        self.pending = Pending::None;
    }

    /// Em insert, só M-SPC é interceptado — o resto cai para a edição
    /// normal (`nas-insert-map` só sobrescreve a troca de modo).
    fn handle_insert(&mut self, key: Key) -> Vec<Action> {
        if key == Key::alt_char(' ') {
            self.enter_visual();
            return Vec::new();
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
        if self.pending != Pending::None {
            return self.resolve_pending(key);
        }

        if key == Key::enter() || key == Key::alt_char(' ') {
            self.enter_insert();
            return Vec::new();
        }

        if !key.alt {
            if let KeyCode::Char(c) = key.code {
                match c {
                    'g' => {
                        self.pending = Pending::Go;
                        return Vec::new();
                    }
                    ' ' => {
                        self.pending = Pending::Leader;
                        return Vec::new();
                    }
                    'd' => {
                        self.pending = Pending::Delete;
                        return Vec::new();
                    }
                    'j' => return vec![Action::MoveLeft],
                    'k' => return vec![Action::MoveDown],
                    'l' => return vec![Action::MoveUp],
                    ';' => return vec![Action::MoveRight],
                    'J' => return vec![Action::MoveLineStart],
                    ':' => return vec![Action::MoveLineEnd],
                    'L' => return vec![Action::MoveBufferStart],
                    'K' => return vec![Action::MoveBufferEnd],
                    'u' => return vec![Action::Undo],
                    _ => {}
                }
            }
        }

        universal_action(key).into_iter().collect()
    }

    fn resolve_pending(&mut self, key: Key) -> Vec<Action> {
        let pending = self.pending;
        self.pending = Pending::None;
        match pending {
            Pending::Go => match key {
                Key {
                    code: KeyCode::Char('g'),
                    alt: false,
                } => vec![Action::MoveBufferStart],
                _ => Vec::new(),
            },
            Pending::Leader => match key {
                Key {
                    code: KeyCode::Char(' '),
                    alt: false,
                } => {
                    self.enter_sequential();
                    Vec::new()
                }
                _ => Vec::new(),
            },
            Pending::Delete => match key {
                Key {
                    code: KeyCode::Char('d' | ';'),
                    alt: false,
                } => vec![Action::DeleteForward],
                Key {
                    code: KeyCode::Char('j'),
                    alt: false,
                } => vec![Action::DeleteBackward],
                _ => {
                    // Fiel ao fallback de `nas-define-dispatch`: apaga pra
                    // frente e reprocessa a tecla como um comando visual
                    // normal (equivalente a `push key unread-command-events`).
                    let mut actions = vec![Action::DeleteForward];
                    actions.extend(self.handle_visual(key));
                    actions
                }
            },
            Pending::None => unreachable!("resolve_pending só é chamado com um pending ativo"),
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
}
