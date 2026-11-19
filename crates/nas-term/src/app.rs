//! Estado da aplicação: liga o teclado (crossterm) à máquina de estados
//! do nas-keys e suas ações aos documentos abertos. Atalhos com Ctrl
//! (classe sistema: salvar, sair, abrir arquivo, trocar e fechar
//! buffer) não fazem parte do vocabulário do nas-keys — são tratados
//! aqui, antes de chegar à máquina de estados. Ver docs/keys.md.

use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use nas_editor::{Document, OpenError, RegionObject};
use nas_history::{
    Author, CheckpointManifest, ExcludedReason, FileChange, Journal, OriginalEntry, ProjectId,
};
use nas_keys::{Action, Key, KeyCode as NasKeyCode, Layout, Machine, Object, SystemAction, Verb};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// O journal de histórico, compartilhado com o panic hook de
/// `main.rs` (ver `install_panic_hook`) — pra que um pânico também
/// force o flush do que ainda estiver pendente, não só os caminhos
/// normais (Ctrl+Q, Ctrl+K, Ctrl+S). `None` enquanto não há projeto
/// identificado ou o journal não pôde ser aberto; todos os testes
/// deste módulo usam esse caso, então editam normalmente sem
/// journalar nada.
pub type SharedJournal = Arc<Mutex<Option<Journal>>>;

/// Um buffer aberto: o documento mais o estado de rolagem que é dele,
/// não da aplicação como um todo (voltar a um buffer deveria mostrar
/// onde você o deixou). O cursor, a seleção e o histórico de desfazer
/// já são do próprio `Document`.
pub struct Buffer {
    pub document: Document,
    pub scroll_top: usize,
    pub scroll_left: usize,
}

impl Buffer {
    fn new(document: Document) -> Self {
        Self {
            document,
            scroll_top: 0,
            scroll_left: 0,
        }
    }
}

/// O que uma linha de entrada está pedindo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptKind {
    OpenFile,
    Search,
    SwitchBuffer,
    GotoLine,
    /// `Ctrl+t n` — nome do novo checkpoint.
    CheckpointName,
    /// `Ctrl+t r` — nome (ou trecho do nome/id) do checkpoint a
    /// restaurar.
    RestoreCheckpoint,
    /// `Ctrl+t z` — exige digitar exatamente "apagar".
    DeleteHistoryConfirm,
}

/// Uma visualização somente-leitura de texto maior que uma linha —
/// diff entre checkpoints, lista de checkpoints, autoria de um
/// arquivo (docs/decisions/0005 §7/§8). Toma a tela inteira enquanto
/// ativa; `j`/`k` (ou as setas) rolam, qualquer outra tecla fecha.
pub struct Overlay {
    pub title: String,
    pub lines: Vec<String>,
    pub scroll: u16,
}

/// Uma linha de entrada ativa na barra de status — equivalente ao
/// minibuffer do Emacs que Ctrl+o (find-file), líder `f`
/// (isearch-forward), Ctrl+b (switch-to-buffer) e `g l` (goto-line)
/// abrem no nas-keys real. Enquanto ativa, nenhuma tecla chega à
/// máquina modal: o texto digitado é só o conteúdo do prompt, nunca
/// interpretado como comando do estado Visual.
pub struct Prompt {
    pub kind: PromptKind,
    pub input: String,
    /// Posição do cursor antes da busca começar, pra restaurar se o
    /// usuário cancelar. `None` fora de uma busca.
    search_origin: Option<usize>,
}

pub struct App {
    pub buffers: Vec<Buffer>,
    pub active: usize,
    pub modal: Machine,
    pub should_quit: bool,
    pub quit_confirm_pending: bool,
    /// Ctrl+k (fechar o buffer atual) foi pressionado uma vez num
    /// buffer com alterações não salvas; a mesma tecla de novo
    /// confirma. Como `quit_confirm_pending`, não é limpo
    /// incondicionalmente — só por outra tecla qualquer, não por
    /// Ctrl+k de novo.
    close_confirm_pending: bool,
    /// Um verbo que apaga ou recorta (`d a`/`x a`) o arquivo inteiro
    /// pediu confirmação — guarda qual verbo, pra só confirmar se a
    /// mesma combinação repetir. Limpo por qualquer ação que não seja
    /// essa mesma repetição.
    whole_buffer_confirm_pending: Option<Verb>,
    pub status_message: Option<String>,
    pub prompt: Option<Prompt>,
    /// Registro interno da área de transferência — sempre atualizado em
    /// copiar/recortar. É o que `y` cola quando a área de transferência
    /// do sistema não está acessível (ex.: SSH sem terminal compatível,
    /// sessão sem display gráfico); quando está, `y` lê de lá primeiro,
    /// pra colar o que foi copiado em outro programa.
    pub clipboard: String,
    /// Texto que `main.rs` deve escrever na área de transferência do
    /// sistema (OSC 52 + `arboard`) depois desta tecla — `App` não tem
    /// acesso a E/S real, pra ficar testável sem terminal nem display.
    pub pending_clipboard_write: Option<String>,
    /// `y` foi pressionado: `main.rs` deve tentar atualizar `clipboard`
    /// a partir da área de transferência do sistema (`arboard`) e então
    /// chamar `apply_pending_paste`. Mesmo motivo do campo acima — ler
    /// a área de transferência do sistema é E/S de verdade, que `App`
    /// não faz sozinho.
    pub pending_paste: bool,
    /// Journal de histórico do projeto — ver [`SharedJournal`].
    history: SharedJournal,
    /// Projeto identificado e onde fica `$XDG_STATE_HOME` — `None`
    /// sem histórico ligado (todos os testes deste módulo, a menos
    /// que chamem `attach_project`). `ProjectId::ensure_history_dir`
    /// é idempotente, então `history_dir()` pode chamá-lo de novo a
    /// cada comando de checkpoint sem custo relevante — isso também é
    /// o que recria a pasta depois de `Ctrl+t z` apagar tudo.
    project: Option<(ProjectId, PathBuf)>,
    /// `Ctrl+t` foi pressionado: a próxima tecla (sem Ctrl) resolve
    /// dentro do prefixo nomeado (hoje só `"tempo"`) — ver
    /// `Machine::system_prefix`.
    pending_system_prefix: Option<String>,
    /// `Ctrl+t r` encontrou um checkpoint cujos arquivos afetados têm
    /// alterações não salvas — guarda o id do checkpoint, pra só
    /// confirmar se o mesmo nome for restaurado de novo. Mesma ideia
    /// de `whole_buffer_confirm_pending`.
    restore_confirm_pending: Option<String>,
    /// `Ctrl+s` encontrou uma mudança externa não resolvida no buffer
    /// atual — a mesma tecla de novo confirma a sobrescrita
    /// (docs/decisions/0005 §1: "nunca salva por cima de uma mudança
    /// externa sem perguntar"). Mesma ideia de `close_confirm_pending`.
    external_overwrite_confirm_pending: bool,
    /// `Ctrl+r` encontrou alterações locais não salvas no buffer atual
    /// — a mesma tecla de novo confirma descartá-las e recarregar.
    reload_confirm_pending: bool,
    /// Uma visualização somente-leitura ativa (diff, lista de
    /// checkpoints, autoria) — ver [`Overlay`].
    pub overlay: Option<Overlay>,
}

impl App {
    /// Traduz teclas do layout físico de teclado dado antes de repassá-las
    /// à máquina modal (ver `NAS_KEYBOARD_LAYOUT` em `main.rs`).
    pub fn with_layout(document: Document, layout: Layout) -> Self {
        Self {
            buffers: vec![Buffer::new(document)],
            active: 0,
            modal: Machine::with_layout(layout),
            should_quit: false,
            quit_confirm_pending: false,
            close_confirm_pending: false,
            whole_buffer_confirm_pending: None,
            status_message: None,
            prompt: None,
            clipboard: String::new(),
            pending_clipboard_write: None,
            pending_paste: false,
            history: Arc::new(Mutex::new(None)),
            project: None,
            pending_system_prefix: None,
            restore_confirm_pending: None,
            external_overwrite_confirm_pending: false,
            reload_confirm_pending: false,
            overlay: None,
        }
    }

    /// Liga este `App` a um journal de histórico compartilhado — o
    /// mesmo `Arc` que `main.rs` guarda pro panic hook, pra que um
    /// pânico também force o flush do que ainda estiver pendente.
    /// Sem chamar isto (como em todos os testes deste módulo), a
    /// edição segue funcionando normalmente, só sem registrar nada no
    /// histórico.
    pub fn attach_history(&mut self, history: SharedJournal) {
        self.history = history;
    }

    /// Liga este `App` ao projeto identificado (raiz do Git ou
    /// diretório atual) e a `$XDG_STATE_HOME` — habilita os comandos
    /// de checkpoint do prefixo `Ctrl+t` (novo, listar, diff,
    /// restaurar, autoria, apagar histórico), que precisam da pasta
    /// de histórico além do journal. Sem chamar isto, esses comandos
    /// respondem "histórico não disponível" em vez de fazer nada.
    pub fn attach_project(&mut self, project: ProjectId, state_home: PathBuf) {
        self.project = Some((project, state_home));
    }

    /// A pasta de histórico deste projeto, criando-a se preciso
    /// (`ProjectId::ensure_history_dir` é idempotente) — `None` sem
    /// `attach_project` ou se a pasta não puder ser criada/lida.
    fn history_dir(&self) -> Option<PathBuf> {
        let (project, state_home) = self.project.as_ref()?;
        project.ensure_history_dir(state_home).ok()
    }

    pub fn document(&self) -> &Document {
        &self.buffers[self.active].document
    }

    pub fn document_mut(&mut self) -> &mut Document {
        &mut self.buffers[self.active].document
    }

    pub fn scroll_top(&self) -> usize {
        self.buffers[self.active].scroll_top
    }

    pub fn scroll_left(&self) -> usize {
        self.buffers[self.active].scroll_left
    }

    /// O prefixo de sistema pendente (`Ctrl+t` → `"tempo"`), se
    /// houver um esperando a próxima tecla — pra `ui.rs` mostrar a
    /// dica na barra de status.
    pub fn pending_system_prefix(&self) -> Option<&str> {
        self.pending_system_prefix.as_deref()
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        if self.overlay.is_some() {
            self.handle_overlay_key(key);
            return;
        }

        if self.prompt.is_some() {
            self.handle_prompt_key(key);
            return;
        }

        if let Some(prefix) = self.pending_system_prefix.take() {
            self.handle_system_prefix_key(&prefix, key);
            return;
        }

        if key.modifiers.contains(KeyModifiers::CONTROL) {
            self.handle_system_key(key);
            return;
        }

        let Some(nas_key) = translate_key(key) else {
            return;
        };

        let has_selection = self.document().selection_range().is_some();
        for action in self.modal.handle_key(nas_key, has_selection) {
            self.apply_action(action);
        }
    }

    /// Enquanto um `Overlay` está ativo: `j`/`k` ou as setas rolam,
    /// qualquer outra tecla (inclusive Esc) fecha.
    fn handle_overlay_key(&mut self, key: KeyEvent) {
        let Some(overlay) = &mut self.overlay else {
            return;
        };
        match key.code {
            KeyCode::Down | KeyCode::Char('j') => overlay.scroll = overlay.scroll.saturating_add(1),
            KeyCode::Up | KeyCode::Char('k') => overlay.scroll = overlay.scroll.saturating_sub(1),
            _ => self.overlay = None,
        }
    }

    /// Ctrl + tecla: a classe sistema (docs/keys.md §5.7), resolvida
    /// pelo keymap (`[system]` em `default.toml`), não mais escrita
    /// direto aqui. Uma tecla pode resolver direto pra uma ação, ou
    /// abrir um prefixo nomeado (`Ctrl+t`, "tempo") que espera mais
    /// uma tecla — `handle_system_prefix_key` resolve essa.
    fn handle_system_key(&mut self, key: KeyEvent) {
        let Some(nas_key) = translate_key(key) else {
            return;
        };

        if let Some(prefix) = self.modal.system_prefix(nas_key) {
            self.pending_system_prefix = Some(prefix);
            self.quit_confirm_pending = false;
            self.close_confirm_pending = false;
            self.status_message = None;
            return;
        }

        let Some(system_action) = self.modal.system_action(nas_key) else {
            return;
        };
        self.dispatch_system_action(system_action);
    }

    /// A segunda tecla de um prefixo de sistema pendente (ex.: o `n`
    /// de `Ctrl+t n`) — tecla fora da tabela do prefixo não faz nada,
    /// como qualquer outro prefixo inválido (docs/keys.md §3, regra
    /// 4). `pending_system_prefix` já foi consumido por quem chama
    /// (`handle_key`) antes desta função ser chamada.
    fn handle_system_prefix_key(&mut self, prefix: &str, key: KeyEvent) {
        let Some(nas_key) = translate_key(key) else {
            return;
        };
        let Some(action) = self.modal.system_prefix_action(prefix, nas_key) else {
            return;
        };
        self.dispatch_system_action(action);
    }

    /// `Quit`, `CloseBuffer`, `Save` e `ReloadBuffer` cuidam da própria
    /// confirmação (campos de pendência que não devem ser limpos pela
    /// repetição da mesma tecla); os demais limpam qualquer
    /// mensagem/confirmação pendente antes de agir.
    fn dispatch_system_action(&mut self, action: SystemAction) {
        if action == SystemAction::Quit {
            self.handle_quit_request();
            return;
        }
        if action == SystemAction::CloseBuffer {
            self.close_buffer();
            return;
        }
        if action == SystemAction::Save {
            self.save();
            return;
        }
        if action == SystemAction::ReloadBuffer {
            self.reload_buffer();
            return;
        }

        self.quit_confirm_pending = false;
        self.close_confirm_pending = false;
        self.status_message = None;

        match action {
            SystemAction::OpenFile => self.open_prompt(PromptKind::OpenFile),
            SystemAction::SwitchBuffer => self.open_prompt(PromptKind::SwitchBuffer),
            SystemAction::CheckpointNew => self.open_prompt(PromptKind::CheckpointName),
            SystemAction::CheckpointList => self.checkpoint_list(),
            SystemAction::CheckpointDiff => self.checkpoint_diff(),
            SystemAction::CheckpointRestore => self.open_prompt(PromptKind::RestoreCheckpoint),
            SystemAction::CheckpointBlame => self.checkpoint_blame(),
            SystemAction::HistoryDelete => self.open_prompt(PromptKind::DeleteHistoryConfirm),
            SystemAction::Quit
            | SystemAction::CloseBuffer
            | SystemAction::Save
            | SystemAction::ReloadBuffer => {
                unreachable!()
            }
        }
    }

    fn apply_action(&mut self, action: Action) {
        let repeats_whole_buffer_confirm = matches!(
            action,
            Action::VerbObject(verb, Object::WholeBuffer)
                if Some(verb) == self.whole_buffer_confirm_pending
        );
        if !repeats_whole_buffer_confirm {
            self.whole_buffer_confirm_pending = None;
        }

        match action {
            Action::MoveLeft => self.document_mut().move_left(),
            Action::MoveRight => self.document_mut().move_right(),
            Action::MoveUp => self.document_mut().move_up(),
            Action::MoveDown => self.document_mut().move_down(),
            Action::MoveLineStart => self.document_mut().move_to_line_start(),
            Action::MoveLineEnd => self.document_mut().move_to_line_end(),
            Action::MoveBufferStart => self.document_mut().move_to_buffer_start(),
            Action::MoveBufferEnd => self.document_mut().move_to_buffer_end(),
            Action::MoveWordForward => self.document_mut().move_word_forward(),
            Action::MoveWordBackward => self.document_mut().move_word_backward(),
            Action::MoveParagraphForward => self.document_mut().move_paragraph_forward(),
            Action::MoveParagraphBackward => self.document_mut().move_paragraph_backward(),
            Action::DeleteForward => {
                let (from, to) = self.document().region_for(RegionObject::CharRight);
                let removed = self.document().text_in(from, to);
                self.document_mut().delete_forward();
                self.journal_record("delete-forward", from, to, removed, String::new());
            }
            Action::DeleteBackward => {
                let (from, to) = self.document().region_for(RegionObject::CharLeft);
                let removed = self.document().text_in(from, to);
                self.document_mut().delete_backward();
                self.journal_record("delete-backward", from, to, removed, String::new());
            }
            Action::Undo => self.undo(),
            Action::InsertChar(c) => {
                let from = self.document().cursor_offset();
                self.document_mut().insert_char(c);
                self.journal_record("insert", from, from, String::new(), c.to_string());
            }
            Action::InsertNewline => {
                let from = self.document().cursor_offset();
                self.document_mut().insert_newline();
                self.journal_record("insert", from, from, String::new(), "\n".to_string());
            }
            Action::PromptSearch => {
                let origin = self.document().cursor_offset();
                self.prompt = Some(Prompt {
                    kind: PromptKind::Search,
                    input: String::new(),
                    search_origin: Some(origin),
                });
            }
            Action::PromptGotoLine => self.open_prompt(PromptKind::GotoLine),
            Action::StartSelection => self.document_mut().start_selection(),
            Action::SelectAll => self.document_mut().select_all(),
            Action::Copy => self.copy(),
            Action::Cut => self.cut(),
            Action::DeleteSelection => self.delete_selection_action(),
            Action::Paste => self.pending_paste = true,
            Action::VerbObject(verb, object) => self.apply_verb_object(verb, object),
            Action::ClearSelection => self.document_mut().clear_selection(),
        }
    }

    fn open_prompt(&mut self, kind: PromptKind) {
        self.prompt = Some(Prompt {
            kind,
            input: String::new(),
            search_origin: None,
        });
    }

    /// Um verbo agindo sobre um objeto (docs/keys.md §4.3). `d a`/`x a`
    /// (apagar/recortar o arquivo inteiro) pedem confirmação antes de
    /// executar — a mesma combinação repetida confirma.
    fn apply_verb_object(&mut self, verb: Verb, object: Object) {
        if object == Object::WholeBuffer && matches!(verb, Verb::Delete | Verb::Cut) {
            if self.whole_buffer_confirm_pending == Some(verb) {
                self.whole_buffer_confirm_pending = None;
            } else {
                self.whole_buffer_confirm_pending = Some(verb);
                self.status_message = Some(format!(
                    "{} o arquivo inteiro? repita pra confirmar.",
                    verb_label(verb)
                ));
                return;
            }
        }

        let region = to_region_object(object);
        let (from, to) = self.document().region_for(region);
        match verb {
            Verb::Copy => {
                let text = self.document().text_in(from, to);
                self.write_clipboard_if_nonempty(text);
            }
            Verb::Delete | Verb::Cut => {
                let text = self.document_mut().delete_region(from, to);
                let command = format!("{}:{}", verb.command_name(), object.command_name());
                self.journal_record(&command, from, to, text.clone(), String::new());
                self.write_clipboard_if_nonempty(text);
            }
        }
    }

    /// Registra uma mudança no journal de histórico do projeto, com
    /// `Author::User` — toda transação que o `nas-editor` aplica passa
    /// por aqui, incluindo o desfazer (docs/decisions/0005 §1). Não
    /// faz nada se não houver journal ligado (`attach_history` nunca
    /// chamado, como em todos os testes deste módulo) ou se a mudança
    /// não alterou nada (`from == to` e nada removido/inserido).
    fn journal_record(
        &mut self,
        command: &str,
        from: usize,
        to: usize,
        removed: String,
        inserted: String,
    ) {
        let file = buffer_name(self.document());
        self.journal_record_for(file, command, from, to, removed, inserted);
    }

    /// Como `journal_record`, mas pro arquivo `file` em vez do buffer
    /// ativo — usado por `restore_file`, que pode afetar um arquivo
    /// que não é o buffer atual (e, pros arquivos restaurados que não
    /// estão abertos, nem é um buffer de verdade).
    ///
    /// Também é aqui que um arquivo passa a ser **rastreado**
    /// (`nas_history::ensure_tracked`): na primeira vez que `file`
    /// aparece — e só nessa vez — captura o conteúdo que já estava no
    /// disco, antes desta edição. Isto é seguro porque nenhuma ação
    /// comum de editar escreve em disco (só `save`/`restore_file`, que
    /// só tocam arquivos já rastreados); se já fosse tarde pra capturar
    /// o original, `ensure_tracked` não teria como saber.
    fn journal_record_for(
        &mut self,
        file: String,
        command: &str,
        from: usize,
        to: usize,
        removed: String,
        inserted: String,
    ) {
        if from == to && removed.is_empty() && inserted.is_empty() {
            return;
        }
        if file != UNNAMED_BUFFER
            && let Some(history_dir) = self.history_dir()
        {
            let disk_content = std::fs::read(&file).ok();
            let _ = nas_history::ensure_tracked(&history_dir, &file, disk_content.as_deref());
        }
        let Ok(mut guard) = self.history.lock() else {
            return;
        };
        let Some(journal) = guard.as_mut() else {
            return;
        };
        let now = nas_history::now_unix();
        let _ = journal.record(
            now,
            Author::User,
            command.to_string(),
            file,
            from,
            to,
            removed,
            inserted,
        );
    }

    /// Força o flush do journal de histórico — chamado nos pontos em
    /// que o nas pode terminar ou perder o buffer em memória: sair
    /// (Ctrl+Q), fechar um buffer (Ctrl+K) e salvar (Ctrl+S). Um
    /// pânico também força o flush, via o mesmo `Arc` compartilhado
    /// com o panic hook de `main.rs`.
    fn flush_history(&mut self) {
        if let Ok(mut guard) = self.history.lock()
            && let Some(journal) = guard.as_mut()
        {
            let _ = journal.flush(nas_history::now_unix());
        }
    }

    fn write_clipboard_if_nonempty(&mut self, text: String) {
        if !text.is_empty() {
            self.clipboard = text.clone();
            self.pending_clipboard_write = Some(text);
        }
    }

    /// Insere o conteúdo atual de `clipboard` no documento, e desarma
    /// `pending_paste`. Chamado por `main.rs` depois de tentar atualizar
    /// `clipboard` a partir da área de transferência do sistema — se
    /// essa tentativa falhar (sem display, terminal sem suporte), o
    /// registro interno (o que já estava em `clipboard`) é quem cola.
    /// Com uma seleção ativa, substitui a seleção (docs/keys.md §4.2).
    /// Não faz nada se não houver um `y` pendente.
    pub fn apply_pending_paste(&mut self) {
        if !std::mem::take(&mut self.pending_paste) {
            return;
        }
        if self.document().selection_range().is_some() {
            self.document_mut().delete_selection();
        }
        let text = self.clipboard.clone();
        self.paste_text(&text);
    }

    fn copy(&mut self) {
        if let Some(text) = self.document().selected_text() {
            self.write_clipboard_if_nonempty(text);
        }
        self.document_mut().clear_selection();
    }

    fn cut(&mut self) {
        let Some((from, to)) = self.document().selection_range() else {
            return;
        };
        let text = self.document().selected_text().unwrap_or_default();
        self.document_mut().delete_selection();
        self.journal_record("cut", from, to, text.clone(), String::new());
        self.write_clipboard_if_nonempty(text);
    }

    /// Apaga a seleção agora, sem tocar na área de transferência — `d`
    /// com uma seleção ativa (docs/keys.md §4.2).
    fn delete_selection_action(&mut self) {
        let Some((from, to)) = self.document().selection_range() else {
            return;
        };
        let removed = self.document().selected_text().unwrap_or_default();
        self.document_mut().delete_selection();
        self.journal_record("delete", from, to, removed, String::new());
    }

    /// Insere `text` no documento, caractere a caractere — exceto
    /// quebras de linha (`"\n"`, `"\r\n"` ou `"\r"` sozinho, tratadas
    /// como uma única quebra), que passam por `insert_newline` pra
    /// respeitar o estilo de quebra de linha do documento atual, em vez
    /// de reproduzir literalmente o que veio colado.
    fn paste_text(&mut self, text: &str) {
        let mut chars = text.chars().peekable();
        while let Some(c) = chars.next() {
            let from = self.document().cursor_offset();
            let inserted = match c {
                '\n' => {
                    self.document_mut().insert_newline();
                    "\n".to_string()
                }
                '\r' => {
                    if chars.peek() == Some(&'\n') {
                        chars.next();
                    }
                    self.document_mut().insert_newline();
                    "\n".to_string()
                }
                _ => {
                    self.document_mut().insert_char(c);
                    c.to_string()
                }
            };
            self.journal_record("paste", from, from, String::new(), inserted);
        }
    }

    /// Texto colado pelo terminal via bracketed paste
    /// (`Event::Paste` do crossterm) — nunca passa pela máquina modal,
    /// então nunca é interpretado como comando do estado Visual, só
    /// como texto. Vai pro prompt ativo, se houver um; senão, substitui
    /// uma seleção ativa (como `y`) e insere direto no documento,
    /// independente do estado modal.
    pub fn handle_paste(&mut self, text: String) {
        if self.prompt.is_some() {
            if let Some(prompt) = &mut self.prompt {
                prompt.input.push_str(&text);
            }
            self.update_incremental_search();
            return;
        }
        if self.document().selection_range().is_some() {
            self.document_mut().delete_selection();
        }
        self.paste_text(&text);
    }

    /// Enquanto uma linha de entrada está ativa, só ela recebe teclas —
    /// nada chega à máquina modal (equivalente ao `minibuffer-setup-hook`
    /// do nas-keys real, que troca pra Insert, mas aqui é mais simples
    /// nem repassar pra máquina nenhuma).
    fn handle_prompt_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => self.cancel_prompt(),
            KeyCode::Enter => self.confirm_prompt(),
            KeyCode::Backspace => {
                if let Some(prompt) = &mut self.prompt {
                    prompt.input.pop();
                }
                self.update_incremental_search();
            }
            KeyCode::Char(c)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                if let Some(prompt) = &mut self.prompt {
                    prompt.input.push(c);
                }
                self.update_incremental_search();
            }
            _ => {}
        }
    }

    /// A cada tecla da busca, refaz a busca inteira a partir de onde ela
    /// começou (não de onde o cursor está agora, que já pode ter saltado
    /// pro match anterior) — assim apagar um caractere da consulta pode
    /// achar uma ocorrência mais cedo no documento.
    fn update_incremental_search(&mut self) {
        let Some(prompt) = &self.prompt else { return };
        let (PromptKind::Search, Some(origin)) = (prompt.kind, prompt.search_origin) else {
            return;
        };
        let query = prompt.input.clone();
        if let Some(pos) = self.document().find_from(origin, &query) {
            self.document_mut().set_cursor(pos);
        }
    }

    fn confirm_prompt(&mut self) {
        let Some(prompt) = self.prompt.take() else {
            return;
        };
        match prompt.kind {
            PromptKind::OpenFile => self.open_file(&prompt.input),
            // O cursor já está no último match encontrado (ou de volta
            // na posição original, se nada bateu) — só sair do prompt.
            PromptKind::Search => {}
            PromptKind::SwitchBuffer => self.switch_buffer(&prompt.input),
            PromptKind::GotoLine => self.goto_line(&prompt.input),
            PromptKind::CheckpointName => self.checkpoint_new(&prompt.input),
            PromptKind::RestoreCheckpoint => self.restore_checkpoint(&prompt.input),
            PromptKind::DeleteHistoryConfirm => self.confirm_delete_history(&prompt.input),
        }
    }

    fn cancel_prompt(&mut self) {
        let Some(prompt) = self.prompt.take() else {
            return;
        };
        if let Some(origin) = prompt.search_origin {
            self.document_mut().set_cursor(origin);
        }
    }

    /// Abre `path` num buffer novo — ou troca pro buffer já aberto com
    /// esse caminho, se houver um. Não recusa por causa de alterações
    /// não salvas: o buffer atual continua intacto em segundo plano.
    fn open_file(&mut self, path: &str) {
        let path = path.trim();
        if path.is_empty() {
            self.status_message = Some("Nenhum arquivo informado.".to_string());
            return;
        }
        let resolved = expand_path(path);

        if let Some(index) = self.find_buffer_by_path(&resolved) {
            self.active = index;
            self.check_external_change_active();
            return;
        }

        match Document::open(&resolved) {
            Ok(document) => {
                self.buffers.push(Buffer::new(document));
                self.active = self.buffers.len() - 1;
            }
            Err(OpenError::NotUtf8) => {
                self.status_message = Some(format!("'{path}' não é UTF-8 válido."));
            }
            Err(err) => {
                self.status_message = Some(format!("Erro ao abrir '{path}': {err}"));
            }
        }
    }

    fn find_buffer_by_path(&self, path: &Path) -> Option<usize> {
        self.buffers
            .iter()
            .position(|b| b.document.path() == Some(path))
    }

    /// Troca pro primeiro buffer aberto cujo nome contém `query`. Com
    /// `query` vazia, ou sem nenhum buffer batendo, mostra uma mensagem
    /// em vez de trocar.
    fn switch_buffer(&mut self, query: &str) {
        let query = query.trim();
        if query.is_empty() {
            self.status_message = Some("Nenhum buffer informado.".to_string());
            return;
        }
        match self
            .buffers
            .iter()
            .position(|b| buffer_name(&b.document).contains(query))
        {
            Some(index) => {
                self.active = index;
                self.check_external_change_active();
            }
            None => {
                self.status_message = Some(format!("Nenhum buffer encontrado para {query:?}."));
            }
        }
    }

    fn goto_line(&mut self, input: &str) {
        match input.trim().parse::<usize>() {
            Ok(line) if line >= 1 => self.document_mut().goto_line(line),
            _ => self.status_message = Some("Número de linha inválido.".to_string()),
        }
    }

    /// Fecha o buffer atual — Ctrl+k. Pede confirmação se ele tiver
    /// alterações não salvas (a mesma tecla de novo confirma). Fechar
    /// o último buffer aberto deixa um `[sem nome]` vazio no lugar: o
    /// nas sempre tem pelo menos um buffer.
    fn close_buffer(&mut self) {
        if self.document().is_modified() && !self.close_confirm_pending {
            self.close_confirm_pending = true;
            self.status_message = Some(format!(
                "{} tem alterações não salvas — Ctrl+K de novo fecha sem salvar.",
                buffer_name(self.document())
            ));
            return;
        }
        self.close_confirm_pending = false;
        self.status_message = None;

        self.buffers.remove(self.active);
        if self.buffers.is_empty() {
            self.buffers.push(Buffer::new(Document::empty()));
            self.active = 0;
        } else if self.active >= self.buffers.len() {
            self.active = self.buffers.len() - 1;
        }
        self.check_external_change_active();
        self.flush_history();
    }

    /// Ajusta a rolagem do buffer ativo para que o cursor fique visível
    /// numa área de `width` x `height` células.
    pub fn ensure_cursor_visible(&mut self, width: usize, height: usize) {
        let position = self.document().cursor_position();
        let visual_column = self.document().cursor_visual_column();
        let buffer = &mut self.buffers[self.active];

        if position.line < buffer.scroll_top {
            buffer.scroll_top = position.line;
        } else if position.line >= buffer.scroll_top + height {
            buffer.scroll_top = position.line + 1 - height;
        }

        if visual_column < buffer.scroll_left {
            buffer.scroll_left = visual_column;
        } else if width > 0 && visual_column >= buffer.scroll_left + width {
            buffer.scroll_left = visual_column + 1 - width;
        }
    }

    /// Pergunta só se algum buffer tiver alterações não salvas,
    /// dizendo quais — Ctrl+S só salva o buffer atual, então a mensagem
    /// deixa isso claro quando há mais de um.
    fn handle_quit_request(&mut self) {
        let modified: Vec<String> = self
            .buffers
            .iter()
            .filter(|b| b.document.is_modified())
            .map(|b| buffer_name(&b.document))
            .collect();

        if modified.is_empty() || self.quit_confirm_pending {
            self.should_quit = true;
            self.flush_history();
            return;
        }
        self.quit_confirm_pending = true;
        self.status_message = Some(format!(
            "Alterações não salvas em: {} — Ctrl+Q de novo sai sem salvar, Ctrl+S salva o buffer atual",
            modified.join(", ")
        ));
    }

    /// Ctrl+s. Confere antes se o arquivo mudou no disco desde a
    /// última leitura/escrita do nas (docs/decisions/0005 §1) — se
    /// mudou, pede confirmação (repetir a tecla) em vez de
    /// sobrescrever direto; a própria mudança externa já foi
    /// journalada no momento em que foi detectada, independente de
    /// confirmar ou não a sobrescrita aqui.
    fn save(&mut self) {
        self.quit_confirm_pending = false;
        self.close_confirm_pending = false;

        self.check_external_change_active();
        if self.document().external_change_pending() && !self.external_overwrite_confirm_pending {
            self.external_overwrite_confirm_pending = true;
            self.status_message = Some(format!(
                "{} mudou no disco desde a última leitura — Ctrl+S de novo sobrescreve, Ctrl+R recarrega.",
                buffer_name(self.document())
            ));
            return;
        }
        self.external_overwrite_confirm_pending = false;

        self.status_message = Some(match self.document_mut().save() {
            Ok(true) => "Salvo.".to_string(),
            Ok(false) => "Sem nome de arquivo; nada para salvar.".to_string(),
            Err(err) => format!("Erro ao salvar: {err}"),
        });
        self.flush_history();
    }

    /// Ctrl+r: recarrega o buffer atual a partir do disco — resolve
    /// de propósito uma mudança externa detectada, ou simplesmente
    /// "sei que mudei isto fora do nas". Pede confirmação (repetir a
    /// tecla) se houver alterações locais não salvas, já que
    /// recarregar as descarta — mesmo risco que fechar o buffer.
    fn reload_buffer(&mut self) {
        self.quit_confirm_pending = false;
        self.close_confirm_pending = false;

        if self.document().is_modified() && !self.reload_confirm_pending {
            self.reload_confirm_pending = true;
            self.status_message = Some(format!(
                "{} tem alterações não salvas — Ctrl+R de novo descarta e recarrega.",
                buffer_name(self.document())
            ));
            return;
        }
        self.reload_confirm_pending = false;
        self.external_overwrite_confirm_pending = false;

        let file = buffer_name(self.document());
        match self.document_mut().reload() {
            Ok(Some(change)) => {
                self.journal_record_for(
                    file,
                    "reload",
                    change.from,
                    change.to,
                    change.removed,
                    change.inserted,
                );
                self.status_message = Some("Recarregado.".to_string());
            }
            Ok(None) => {
                self.status_message = Some("Já estava sincronizado com o disco.".to_string())
            }
            Err(err) => self.status_message = Some(format!("Erro ao recarregar: {err}")),
        }
    }

    /// Confere o buffer **ativo** contra o disco — chamado
    /// explicitamente antes de salvar (docs/decisions/0005 §1), além
    /// da conferência geral de `check_external_changes`.
    fn check_external_change_active(&mut self) {
        if let Some(message) = self.check_external_change_at(self.active) {
            self.status_message = Some(message);
        }
    }

    /// Confere **todo buffer aberto** contra o disco — um `stat` por
    /// buffer, barato (docs/decisions/0005 §1: "de forma leve, a cada
    /// poucos segundos"). Chamado pela checagem periódica e por
    /// `handle_focus_gained`. Mostra uma mensagem combinando todos os
    /// buffers que mudaram, se algum mudou.
    pub fn check_external_changes(&mut self) {
        let messages: Vec<String> = (0..self.buffers.len())
            .filter_map(|index| self.check_external_change_at(index))
            .collect();
        if !messages.is_empty() {
            self.status_message = Some(messages.join("; "));
        }
    }

    /// O terminal recuperou o foco (`Event::FocusGained` do
    /// crossterm) — o caso mais comum de um editor externo ou um
    /// `git checkout` acontecerem enquanto o usuário estava noutra
    /// janela (docs/decisions/0005 §1).
    pub fn handle_focus_gained(&mut self) {
        self.check_external_changes();
    }

    /// A conferência de verdade pra um buffer: `stat`, e só lê o
    /// conteúdo se `mtime`/tamanho mudaram. Se uma mudança externa for
    /// confirmada, journala como `Author::External` na hora (mesmo que
    /// o usuário nunca chegue a recarregar) e devolve uma mensagem pra
    /// avisar. Falha de E/S ao conferir (ex.: pasta removida) não trava
    /// a interface — só não detecta nada desta vez.
    fn check_external_change_at(&mut self, index: usize) -> Option<String> {
        let file = buffer_name(&self.buffers[index].document);
        let change = self.buffers[index]
            .document
            .check_external_change()
            .ok()??;
        self.journal_record_external(file.clone(), change);
        Some(format!("{file} mudou no disco — Ctrl+R recarrega"))
    }

    /// Registra uma mudança externa no journal — como `journal_record`,
    /// mas com `Author::External` em vez de `Author::User`, e sem
    /// marcar `file` como rastreado (`nas_history::ensure_tracked`):
    /// uma mudança externa não é uma edição do nas, então não deveria
    /// iniciar o rastreamento de um arquivo que o nas ainda não tinha
    /// tocado (docs/decisions/0005 §4) — só a primeira edição de
    /// verdade (`journal_record_for`) faz isso.
    fn journal_record_external(&mut self, file: String, change: nas_editor::TextChange) {
        if change.from == change.to && change.removed.is_empty() && change.inserted.is_empty() {
            return;
        }
        let Ok(mut guard) = self.history.lock() else {
            return;
        };
        let Some(journal) = guard.as_mut() else {
            return;
        };
        let now = nas_history::now_unix();
        let _ = journal.record(
            now,
            Author::External,
            "external-change".to_string(),
            file,
            change.from,
            change.to,
            change.removed,
            change.inserted,
        );
    }

    fn undo(&mut self) {
        match self.document_mut().undo() {
            Some(change) => self.journal_record(
                "undo",
                change.from,
                change.to,
                change.removed,
                change.inserted,
            ),
            None => self.status_message = Some("Nada para desfazer.".to_string()),
        }
    }

    /// `Ctrl+t n`: cria um checkpoint nomeado com o estado atual de
    /// **todo arquivo rastreado** do projeto — não só os que mudaram
    /// desde o checkpoint anterior (correção de
    /// docs/decisions/0005 §4: só assim restaurar um checkpoint antigo
    /// sabe o estado de um arquivo tocado por primeira vez depois
    /// dele). O conteúdo vem do buffer aberto, se houver um, senão do
    /// disco (`current_content_for`) — um arquivo rastreado que não
    /// está mais aberto como buffer ainda entra no checkpoint. Faz
    /// flush do journal antes, pra que todo arquivo editado até agora
    /// já esteja rastreado.
    fn checkpoint_new(&mut self, name: &str) {
        let name = name.trim();
        if name.is_empty() {
            self.status_message = Some("Nenhum nome informado.".to_string());
            return;
        }
        let Some(history_dir) = self.history_dir() else {
            self.status_message = Some("Histórico não disponível.".to_string());
            return;
        };
        self.flush_history();

        let tracked = nas_history::list_tracked(&history_dir).unwrap_or_default();
        let files: Vec<(String, Vec<u8>)> = tracked
            .iter()
            .filter_map(|entry| {
                let content = self.current_content_for(&entry.path)?;
                Some((entry.path.clone(), content))
            })
            .collect();

        if files.is_empty() {
            self.status_message = Some("Nenhum arquivo rastreado ainda.".to_string());
            return;
        }

        match nas_history::create_checkpoint(
            &history_dir,
            name,
            Author::User,
            nas_history::now_unix(),
            false,
            &files,
        ) {
            Ok(created) => self.status_message = Some(checkpoint_created_message(&created)),
            Err(err) => self.status_message = Some(format!("Erro ao criar checkpoint: {err}")),
        }
    }

    /// `Ctrl+t l`: lista os checkpoints do projeto, mais recente
    /// primeiro, num `Overlay`.
    fn checkpoint_list(&mut self) {
        let Some(history_dir) = self.history_dir() else {
            self.status_message = Some("Histórico não disponível.".to_string());
            return;
        };
        match nas_history::list_checkpoints(&history_dir) {
            Ok(checkpoints) if checkpoints.is_empty() => {
                self.status_message = Some("Nenhum checkpoint ainda.".to_string());
            }
            Ok(checkpoints) => {
                let lines = checkpoints
                    .iter()
                    .rev()
                    .map(|c| {
                        let kind = if c.automatic {
                            "automático"
                        } else {
                            "nomeado"
                        };
                        format!(
                            "{}  {} ({}, {} arquivo(s))",
                            format_timestamp(c.created_at),
                            c.name,
                            kind,
                            c.files.len()
                        )
                    })
                    .collect();
                self.overlay = Some(Overlay {
                    title: "Checkpoints (mais recente primeiro) — qualquer tecla fecha".to_string(),
                    lines,
                    scroll: 0,
                });
            }
            Err(err) => self.status_message = Some(format!("Erro ao listar checkpoints: {err}")),
        }
    }

    /// `Ctrl+t e`: diff entre os dois checkpoints mais recentes, num
    /// `Overlay`. Comparar dois checkpoints quaisquer (escolhidos pelo
    /// usuário) fica pra quando houver uma lista navegável — por ora,
    /// os dois últimos já cobrem o caso comum de revisar a última
    /// tarefa.
    fn checkpoint_diff(&mut self) {
        let Some(history_dir) = self.history_dir() else {
            self.status_message = Some("Histórico não disponível.".to_string());
            return;
        };
        let checkpoints = nas_history::list_checkpoints(&history_dir).unwrap_or_default();
        if checkpoints.len() < 2 {
            self.status_message =
                Some("É preciso pelo menos dois checkpoints pra comparar.".to_string());
            return;
        }
        let from = &checkpoints[checkpoints.len() - 2];
        let to = &checkpoints[checkpoints.len() - 1];
        match nas_history::diff(&history_dir, &from.id, &to.id) {
            Ok(diffs) => {
                let mut lines = Vec::new();
                for d in diffs.iter().filter(|d| d.change != FileChange::Unchanged) {
                    lines.push(format!("=== {} ({:?}) ===", d.path, d.change));
                    lines.extend(d.unified.lines().map(str::to_string));
                    lines.push(String::new());
                }
                if lines.is_empty() {
                    lines.push("Nenhuma diferença.".to_string());
                }
                self.overlay = Some(Overlay {
                    title: format!("Diff: {} → {} — qualquer tecla fecha", from.name, to.name),
                    lines,
                    scroll: 0,
                });
            }
            Err(err) => self.status_message = Some(format!("Erro ao gerar diff: {err}")),
        }
    }

    /// `Ctrl+t a`: autoria (histórico de edições) do arquivo do
    /// buffer ativo, num `Overlay` — ver `nas_history::file_history`.
    fn checkpoint_blame(&mut self) {
        let Some(history_dir) = self.history_dir() else {
            self.status_message = Some("Histórico não disponível.".to_string());
            return;
        };
        let file = buffer_name(self.document());
        self.flush_history();
        match nas_history::file_history(&history_dir, &file) {
            Ok(entries) if entries.is_empty() => {
                self.status_message = Some(format!("Nenhuma edição registrada para {file}."));
            }
            Ok(entries) => {
                let lines = entries
                    .iter()
                    .rev()
                    .map(|e| {
                        format!(
                            "{}  {}  {}  [{}, {})",
                            format_timestamp(e.ts_start),
                            format_author(&e.author),
                            e.command,
                            e.from,
                            e.to
                        )
                    })
                    .collect();
                self.overlay = Some(Overlay {
                    title: format!("Autoria: {file} — qualquer tecla fecha"),
                    lines,
                    scroll: 0,
                });
            }
            Err(err) => self.status_message = Some(format!("Erro ao ler histórico: {err}")),
        }
    }

    /// `Ctrl+t r`: restaura o checkpoint cujo nome (ou id) contém
    /// `query` — o mais recente, se mais de um bater. Pede
    /// confirmação (repetir com o mesmo `query`) se algum arquivo
    /// afetado tiver um buffer aberto com alterações não salvas.
    fn restore_checkpoint(&mut self, query: &str) {
        let query = query.trim();
        if query.is_empty() {
            self.status_message = Some("Nenhum checkpoint informado.".to_string());
            return;
        }
        let Some(history_dir) = self.history_dir() else {
            self.status_message = Some("Histórico não disponível.".to_string());
            return;
        };
        let checkpoints = nas_history::list_checkpoints(&history_dir).unwrap_or_default();
        let Some(checkpoint) = checkpoints
            .iter()
            .rev()
            .find(|c| c.name.contains(query) || c.id.contains(query))
        else {
            self.status_message = Some(format!("Nenhum checkpoint encontrado para {query:?}."));
            return;
        };
        let checkpoint = checkpoint.clone();

        let unsaved: Vec<String> = self
            .buffers
            .iter()
            .filter(|b| {
                let name = buffer_name(&b.document);
                b.document.is_modified() && checkpoint.files.iter().any(|f| f.path == name)
            })
            .map(|b| buffer_name(&b.document))
            .collect();
        let extra = files_not_in_checkpoint(&history_dir, &checkpoint);

        if (!unsaved.is_empty() || !extra.is_empty())
            && self.restore_confirm_pending.as_deref() != Some(&checkpoint.id)
        {
            self.restore_confirm_pending = Some(checkpoint.id.clone());
            let mut warning = Vec::new();
            if !unsaved.is_empty() {
                warning.push(format!("sem salvar: {}", unsaved.join(", ")));
            }
            if !extra.is_empty() {
                warning.push(format!(
                    "não rastreados por este checkpoint, vão pro original ou pra lixeira: {}",
                    extra.join(", ")
                ));
            }
            self.status_message = Some(format!(
                "{} — Ctrl+t r de novo com o mesmo nome confirma.",
                warning.join("; ")
            ));
            return;
        }
        self.restore_confirm_pending = None;

        self.perform_restore(&history_dir, &checkpoint);
    }

    /// A restauração de verdade, depois de resolvida qualquer
    /// confirmação — restaura o **projeto inteiro** pro estado do
    /// checkpoint, não só os arquivos que ele lista
    /// (docs/decisions/0005 §4, correção):
    ///
    /// 1. Cria primeiro um checkpoint automático "antes de restaurar"
    ///    com o conteúdo atual de **todo arquivo rastreado** (o que
    ///    torna a própria restauração desfazível por completo —
    ///    restaurar esse automático desfaz tudo, inclusive os
    ///    arquivos revertidos ou jogados na lixeira abaixo — sem
    ///    nenhum código separado).
    /// 2. Arquivos rastreados que o checkpoint sendo restaurado não
    ///    lista (tocados pela primeira vez depois dele): revertidos
    ///    ao conteúdo original, se existiam; jogados na lixeira
    ///    (nunca apagados de vez), se não existiam ainda.
    /// 3. Os arquivos que o checkpoint lista: escritos com o conteúdo
    ///    guardado nele.
    ///
    /// Nenhum erro de gravação é ignorado — cada arquivo que falhar
    /// aparece na mensagem final, com o motivo.
    fn perform_restore(&mut self, history_dir: &Path, checkpoint: &CheckpointManifest) {
        let tracked = nas_history::list_tracked(history_dir).unwrap_or_default();

        let pre_restore_files: Vec<(String, Vec<u8>)> = tracked
            .iter()
            .filter_map(|e| {
                let content = self.current_content_for(&e.path)?;
                Some((e.path.clone(), content))
            })
            .collect();
        if !pre_restore_files.is_empty() {
            let name = format!("antes de restaurar {}", checkpoint.name);
            let _ = nas_history::create_checkpoint(
                history_dir,
                &name,
                Author::User,
                nas_history::now_unix(),
                true,
                &pre_restore_files,
            );
        }

        let mut restored = Vec::new();
        let mut trashed = Vec::new();
        let mut skipped = Vec::new();
        let mut failed = Vec::new();

        let in_checkpoint: HashSet<&str> =
            checkpoint.files.iter().map(|f| f.path.as_str()).collect();
        for entry in tracked
            .iter()
            .filter(|e| !in_checkpoint.contains(e.path.as_str()))
        {
            if !entry.existed {
                match self.trash_file(&entry.path) {
                    Ok(()) => trashed.push(entry.path.clone()),
                    Err(err) => failed.push((entry.path.clone(), err.to_string())),
                }
                continue;
            }
            restore_tracked_entry(
                self,
                history_dir,
                entry,
                &mut restored,
                &mut skipped,
                &mut failed,
            );
        }

        for file in &checkpoint.files {
            let Some(hash) = &file.content_hash else {
                skipped.push(file.path.clone());
                continue;
            };
            match read_object_as_text(history_dir, hash) {
                Some(content) => match self.restore_file(&file.path, &content) {
                    Ok(()) => restored.push(file.path.clone()),
                    Err(err) => failed.push((file.path.clone(), err.to_string())),
                },
                None => skipped.push(file.path.clone()),
            }
        }

        self.flush_history();
        self.status_message = Some(restore_summary_message(
            &checkpoint.name,
            &restored,
            &trashed,
            &skipped,
            &failed,
        ));
    }

    /// O conteúdo atual de `path` — do buffer aberto, se houver um; senão,
    /// do próprio arquivo em disco. `None` se não há buffer aberto e o
    /// arquivo não existe (ou não pode ser lido) em disco.
    fn current_content_for(&self, path: &str) -> Option<Vec<u8>> {
        if let Some(buffer) = self
            .buffers
            .iter()
            .find(|b| buffer_name(&b.document) == path)
        {
            return Some(buffer.document.full_text().into_bytes());
        }
        std::fs::read(path).ok()
    }

    /// Escreve `content` em `path`: se há um buffer aberto com esse
    /// caminho, substitui o conteúdo dele (via `replace_all`, que
    /// entra no desfazer por tecla do buffer) e salva; senão, abre o
    /// arquivo como um `Document` descartável só pra escrever —
    /// restaurar afeta o arquivo, não só um buffer que porventura
    /// esteja aberto. Em ambos os casos, salvar também grava o
    /// arquivo em disco de uma vez, sem deixar o resultado "só na
    /// memória". Journala como `"restore-checkpoint"`. Propaga
    /// qualquer erro de gravação (ex.: sem permissão de escrita) —
    /// nunca ignorado, quem chama reporta qual arquivo falhou e por
    /// quê.
    fn restore_file(&mut self, path: &str, content: &str) -> io::Result<()> {
        if let Some(index) = self
            .buffers
            .iter()
            .position(|b| buffer_name(&b.document) == path)
        {
            let change = self.buffers[index].document.replace_all(content);
            self.buffers[index].document.save()?;
            self.journal_record_for(
                path.to_string(),
                "restore-checkpoint",
                change.from,
                change.to,
                change.removed,
                change.inserted,
            );
            return Ok(());
        }
        let mut doc = Document::open(path)?;
        let change = doc.replace_all(content);
        doc.save()?;
        self.journal_record_for(
            path.to_string(),
            "restore-checkpoint",
            change.from,
            change.to,
            change.removed,
            change.inserted,
        );
        Ok(())
    }

    /// Move `path` pra lixeira do sistema (nunca apaga de vez) — um
    /// arquivo rastreado que o checkpoint sendo restaurado não
    /// conhece e que não existia originalmente.
    fn trash_file(&self, path: &str) -> io::Result<()> {
        nas_history::move_to_trash(Path::new(path))
    }

    /// `Ctrl+t z`: apaga o histórico do projeto inteiro — só se
    /// `input` for exatamente "apagar" (docs/decisions/0005 §5). Reabre
    /// um journal novo na pasta recriada, pra que edições seguintes
    /// continuem sendo registradas.
    fn confirm_delete_history(&mut self, input: &str) {
        let Some(history_dir) = self.history_dir() else {
            self.status_message = Some("Histórico não disponível.".to_string());
            return;
        };
        match nas_history::delete_project_history(&history_dir, input.trim()) {
            Ok(()) => {
                self.status_message = Some("Histórico apagado.".to_string());
                // `delete_project_history` removeu a pasta inteira —
                // `history_dir()` de novo a recria (ensure_history_dir
                // é idempotente) antes de abrir o journal nela.
                if let Some(recreated) = self.history_dir()
                    && let Ok(journal) = Journal::open(&recreated)
                    && let Ok(mut guard) = self.history.lock()
                {
                    *guard = Some(journal);
                }
            }
            Err(_) => {
                self.status_message =
                    Some("Confirmação incorreta — histórico não foi apagado.".to_string());
            }
        }
    }
}

/// Mensagem de status pra um checkpoint recém-criado — avisa quais
/// arquivos ficaram de fora de `objects/` (grandes demais ou
/// não-UTF8), se algum (docs/decisions/0005 §5: "nunca falha
/// silenciosamente nem finge que o checkpoint capturou tudo").
/// Os caminhos rastreados que `checkpoint` não lista — tocados pela
/// primeira vez depois dele; `perform_restore` reverte ao original ou
/// joga na lixeira cada um, e `restore_checkpoint` avisa sobre eles
/// antes de pedir confirmação.
fn files_not_in_checkpoint(history_dir: &Path, checkpoint: &CheckpointManifest) -> Vec<String> {
    let in_checkpoint: HashSet<&str> = checkpoint.files.iter().map(|f| f.path.as_str()).collect();
    nas_history::list_tracked(history_dir)
        .unwrap_or_default()
        .into_iter()
        .map(|e| e.path)
        .filter(|p| !in_checkpoint.contains(p.as_str()))
        .collect()
}

/// O conteúdo de um objeto do histórico como texto — `None` se não
/// puder ser lido ou não for UTF-8 válido (não deveria acontecer:
/// objetos só guardam conteúdo que já passou por esse mesmo crivo ao
/// serem criados, mas um erro de E/S na leitura ainda é possível).
fn read_object_as_text(history_dir: &Path, hash: &str) -> Option<String> {
    nas_history::read_object(history_dir, hash)
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
}

/// Restaura um arquivo rastreado (não listado pelo checkpoint sendo
/// restaurado) ao seu conteúdo original, acumulando o resultado num
/// dos três baldes — usado só pelo caso "existia" de
/// `App::perform_restore`, o caso "não existia" (lixeira) é tratado
/// ali mesmo.
fn restore_tracked_entry(
    app: &mut App,
    history_dir: &Path,
    entry: &OriginalEntry,
    restored: &mut Vec<String>,
    skipped: &mut Vec<String>,
    failed: &mut Vec<(String, String)>,
) {
    let Some(hash) = &entry.content_hash else {
        skipped.push(entry.path.clone());
        return;
    };
    match read_object_as_text(history_dir, hash) {
        Some(content) => match app.restore_file(&entry.path, &content) {
            Ok(()) => restored.push(entry.path.clone()),
            Err(err) => failed.push((entry.path.clone(), err.to_string())),
        },
        None => skipped.push(entry.path.clone()),
    }
}

/// Mensagem final de uma restauração — nunca esconde o que deu
/// errado: cada arquivo que falhou aparece com o motivo
/// (docs/decisions/0005, correção: "nenhum erro de gravação pode ser
/// ignorado").
fn restore_summary_message(
    checkpoint_name: &str,
    restored: &[String],
    trashed: &[String],
    skipped: &[String],
    failed: &[(String, String)],
) -> String {
    let mut message = format!(
        "Restaurado o checkpoint \"{checkpoint_name}\" ({} arquivo(s))",
        restored.len()
    );
    if !trashed.is_empty() {
        message.push_str(&format!(
            "; pra lixeira (não existiam neste checkpoint): {}",
            trashed.join(", ")
        ));
    }
    if !skipped.is_empty() {
        message.push_str(&format!(
            "; ficaram de fora (sem conteúdo guardado): {}",
            skipped.join(", ")
        ));
    }
    if !failed.is_empty() {
        let details: Vec<String> = failed
            .iter()
            .map(|(path, err)| format!("{path} ({err})"))
            .collect();
        message.push_str(&format!("; FALHARAM: {}", details.join(", ")));
    }
    message.push('.');
    message
}

fn checkpoint_created_message(created: &nas_history::CreatedCheckpoint) -> String {
    if created.excluded.is_empty() {
        return format!(
            "Checkpoint \"{}\" criado com {} arquivo(s).",
            created.manifest.name,
            created.manifest.files.len()
        );
    }
    let excluded: Vec<String> = created
        .excluded
        .iter()
        .map(|e| {
            let reason = match e.reason {
                ExcludedReason::TooLarge => "grande demais",
                ExcludedReason::NotUtf8 => "não-UTF8",
            };
            format!("{} ({reason})", e.path)
        })
        .collect();
    format!(
        "Checkpoint \"{}\" criado; ficaram de fora do conteúdo: {}.",
        created.manifest.name,
        excluded.join(", ")
    )
}

/// Nome de exibição de um autor, pra autoria legível no terminal.
fn format_author(author: &Author) -> String {
    match author {
        Author::User => "você".to_string(),
        Author::Ai { agent, .. } => format!("IA ({agent})"),
        Author::Extension { name } => format!("extensão ({name})"),
        Author::External => "externo".to_string(),
    }
}

/// `secs` (desde a época Unix, UTC) como `AAAA-MM-DD HH:MM:SS` —
/// implementado sem nenhuma dependência de data (a única nova que
/// docs/decisions/0005 autoriza é `similar`, em `nas-history`, pro
/// diff de linhas). O algoritmo (`civil_from_days`) é o de Howard
/// Hinnant pra converter dias desde a época em ano/mês/dia
/// proléptico-gregoriano, amplamente reproduzido e verificado.
fn format_timestamp(secs: u64) -> String {
    let days = (secs / 86400) as i64;
    let secs_of_day = secs % 86400;
    let (year, month, day) = civil_from_days(days);
    let hour = secs_of_day / 3600;
    let minute = (secs_of_day % 3600) / 60;
    let second = secs_of_day % 60;
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02}")
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = if month <= 2 { y + 1 } else { y };
    (year, month, day)
}

fn verb_label(verb: Verb) -> &'static str {
    match verb {
        Verb::Delete => "Apagar",
        Verb::Copy => "Copiar",
        Verb::Cut => "Recortar",
    }
}

fn to_region_object(object: Object) -> RegionObject {
    match object {
        Object::CharLeft => RegionObject::CharLeft,
        Object::CharRight => RegionObject::CharRight,
        Object::LineStart => RegionObject::LineStart,
        Object::LineEnd => RegionObject::LineEnd,
        Object::LineDown => RegionObject::LineDown,
        Object::LineUp => RegionObject::LineUp,
        Object::BufferEnd => RegionObject::BufferEnd,
        Object::BufferStart => RegionObject::BufferStart,
        Object::WordBackward => RegionObject::WordBackward,
        Object::WordForward => RegionObject::WordForward,
        Object::ParagraphForward => RegionObject::ParagraphForward,
        Object::ParagraphBackward => RegionObject::ParagraphBackward,
        Object::CurrentLine => RegionObject::CurrentLine,
        Object::WholeBuffer => RegionObject::WholeBuffer,
    }
}

/// O nome de exibição de um buffer sem arquivo associado — nunca um
/// caminho real, então nunca rastreado por checkpoint (ver
/// `journal_record_for`).
const UNNAMED_BUFFER: &str = "[sem nome]";

/// Nome de exibição de um buffer: o caminho do arquivo, ou
/// `[sem nome]` se ainda não tiver um — usado na barra de status, no
/// prompt de trocar de buffer e nas mensagens de confirmação.
pub fn buffer_name(document: &Document) -> String {
    document
        .path()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| UNNAMED_BUFFER.to_string())
}

/// Traduz um `KeyEvent` do crossterm para a tecla neutra do nas-keys.
/// `None` para teclas que o nas-keys não precisa conhecer (F-keys,
/// PageUp/Down, modificadores sozinhos, etc.).
fn translate_key(key: KeyEvent) -> Option<Key> {
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let code = match key.code {
        KeyCode::Char(c) => NasKeyCode::Char(c),
        KeyCode::Enter => NasKeyCode::Enter,
        KeyCode::Esc => NasKeyCode::Escape,
        KeyCode::Backspace => NasKeyCode::Backspace,
        KeyCode::Delete => NasKeyCode::Delete,
        KeyCode::Tab => NasKeyCode::Tab,
        KeyCode::Left => NasKeyCode::Left,
        KeyCode::Right => NasKeyCode::Right,
        KeyCode::Up => NasKeyCode::Up,
        KeyCode::Down => NasKeyCode::Down,
        _ => return None,
    };
    Some(Key { code, alt })
}

/// Resolve o caminho digitado no prompt de abrir arquivo: `~` (ou
/// `~/...`) expande pro diretório home; um caminho relativo é resolvido
/// a partir do diretório onde o nas foi iniciado — como o nas nunca
/// troca de diretório durante a execução, isso é só o diretório atual
/// do processo. Um caminho absoluto passa direto.
fn expand_path(input: &str) -> PathBuf {
    let path = Path::new(input);
    if let Ok(rest) = path.strip_prefix("~")
        && let Some(home) = std::env::var_os("HOME")
    {
        return PathBuf::from(home).join(rest);
    }
    if path.is_relative()
        && let Ok(cwd) = std::env::current_dir()
    {
        return cwd.join(path);
    }
    path.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;
    use nas_editor::Document;
    use ratatui::crossterm::event::KeyEvent;
    use std::fs;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn alt_key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::ALT)
    }

    fn ctrl_key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::CONTROL)
    }

    fn text(app: &App) -> String {
        (0..app.document().len_lines())
            .map(|i| app.document().line(i).into_owned())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn type_text(app: &mut App, s: &str) {
        app.handle_key(key(KeyCode::Enter)); // Visual -> Insert
        for c in s.chars() {
            app.handle_key(key(KeyCode::Char(c)));
        }
        app.handle_key(alt_key(KeyCode::Char(' '))); // Insert -> Visual
    }

    #[test]
    fn verbo_mais_objeto_e_desfazer_com_u_pelo_estado_visual() {
        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        type_text(&mut app, "abc");
        app.handle_key(key(KeyCode::Char('g')));
        app.handle_key(key(KeyCode::Char('g'))); // início do buffer
        assert_eq!(text(&app), "abc");

        app.handle_key(key(KeyCode::Char('d')));
        app.handle_key(key(KeyCode::Char(';'))); // d ; : apaga o 'a'
        assert_eq!(text(&app), "bc");
        assert_eq!(app.clipboard, "a");

        app.handle_key(key(KeyCode::Char('u'))); // u: desfaz
        assert_eq!(text(&app), "abc");
    }

    #[test]
    fn ctrl_mais_tecla_nao_mapeada_nao_e_reinterpretado_sem_o_ctrl() {
        // Ctrl+j não é parte do vocabulário do nas-keys; não deve mover o
        // cursor como se fosse só 'j' sem o Ctrl.
        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        type_text(&mut app, "abc");
        let position_before = app.document().cursor_position();

        app.handle_key(ctrl_key(KeyCode::Char('j')));

        assert_eq!(app.document().cursor_position(), position_before);
    }

    #[test]
    fn busca_incremental_pula_pro_match_e_cancelar_restaura_o_cursor() {
        // "ola mundo ola de novo": origem na posição 4 (início de
        // "mundo"), pra que o match de "ola" (posição 10, a segunda
        // ocorrência) seja claramente diferente da origem.
        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        type_text(&mut app, "ola mundo ola de novo");
        app.handle_key(key(KeyCode::Char('g')));
        app.handle_key(key(KeyCode::Char('g'))); // início do buffer
        for _ in 0..4 {
            app.handle_key(key(KeyCode::Char(';'))); // move pra posição 4
        }
        let origin = app.document().cursor_offset();
        assert_eq!(origin, 4);

        app.handle_key(key(KeyCode::Char(' '))); // líder
        app.handle_key(key(KeyCode::Char('f'))); // SPC f: busca
        assert!(app.prompt.is_some());

        for c in "ola".chars() {
            app.handle_key(key(KeyCode::Char(c)));
        }
        // achou a segunda ocorrência de "ola", a partir da origem
        assert_eq!(app.document().cursor_offset(), 10);

        app.handle_key(key(KeyCode::Esc));
        assert!(app.prompt.is_none());
        assert_eq!(app.document().cursor_offset(), origin);
    }

    #[test]
    fn busca_confirmada_com_enter_deixa_o_cursor_no_match() {
        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        type_text(&mut app, "ola mundo");
        app.handle_key(key(KeyCode::Char('g')));
        app.handle_key(key(KeyCode::Char('g')));

        app.handle_key(key(KeyCode::Char(' ')));
        app.handle_key(key(KeyCode::Char('f')));
        for c in "mundo".chars() {
            app.handle_key(key(KeyCode::Char(c)));
        }
        app.handle_key(key(KeyCode::Enter));

        assert!(app.prompt.is_none());
        assert_eq!(app.document().cursor_offset(), 4); // "ola " tem 4 chars
    }

    #[test]
    fn durante_o_prompt_teclas_nunca_chegam_na_maquina_modal() {
        // 'j' durante o prompt deve digitar 'j' no prompt, nunca mover o
        // cursor do documento como comando visual.
        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        type_text(&mut app, "abc");
        app.handle_key(key(KeyCode::Char('g')));
        app.handle_key(key(KeyCode::Char('g')));
        let position_before = app.document().cursor_position();

        app.handle_key(ctrl_key(KeyCode::Char('o'))); // Ctrl+o: abre o prompt de arquivo
        app.handle_key(key(KeyCode::Char('j')));
        app.handle_key(key(KeyCode::Char('k')));

        assert_eq!(app.prompt.as_ref().unwrap().input, "jk");
        assert_eq!(app.document().cursor_position(), position_before);

        app.handle_key(key(KeyCode::Esc));
        assert!(app.prompt.is_none());
    }

    #[test]
    fn ctrl_o_abre_arquivo_com_alteracoes_nao_salvas_em_buffer_novo() {
        let dir = tempfile::tempdir().expect("diretório temporário");
        let path = dir.path().join("outro.txt");

        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        type_text(&mut app, "abc");
        assert!(app.document().is_modified());

        app.handle_key(ctrl_key(KeyCode::Char('o')));
        for c in path.to_str().unwrap().chars() {
            app.handle_key(key(KeyCode::Char(c)));
        }
        app.handle_key(key(KeyCode::Enter));

        assert!(app.prompt.is_none());
        assert_eq!(app.buffers.len(), 2);
        assert_eq!(app.active, 1);
        assert_eq!(text(&app), "", "o buffer novo começa vazio");
    }

    #[test]
    fn ctrl_o_com_arquivo_ja_aberto_troca_pro_buffer_em_vez_de_duplicar() {
        let dir = tempfile::tempdir().expect("diretório temporário");
        let path = dir.path().join("existente.txt");
        std::fs::write(&path, "conteudo").unwrap();

        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        let open = |app: &mut App| {
            app.handle_key(ctrl_key(KeyCode::Char('o')));
            for c in path.to_str().unwrap().chars() {
                app.handle_key(key(KeyCode::Char(c)));
            }
            app.handle_key(key(KeyCode::Enter));
        };
        open(&mut app);
        assert_eq!(app.buffers.len(), 2);
        let first_open_index = app.active;

        app.handle_key(key(KeyCode::Char('g')));
        app.handle_key(key(KeyCode::Char('g')));
        open(&mut app); // abre o mesmo caminho de novo

        assert_eq!(app.buffers.len(), 2, "não deveria duplicar o buffer");
        assert_eq!(app.active, first_open_index);
    }

    #[test]
    fn ctrl_b_troca_de_buffer_pelo_nome() {
        let dir = tempfile::tempdir().expect("diretório temporário");
        let path = dir.path().join("segundo.txt");

        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        app.handle_key(ctrl_key(KeyCode::Char('o')));
        for c in path.to_str().unwrap().chars() {
            app.handle_key(key(KeyCode::Char(c)));
        }
        app.handle_key(key(KeyCode::Enter));
        assert_eq!(app.active, 1);

        app.handle_key(ctrl_key(KeyCode::Char('b')));
        assert_eq!(app.prompt.as_ref().unwrap().kind, PromptKind::SwitchBuffer);
        for c in "sem nome".chars() {
            app.handle_key(key(KeyCode::Char(c)));
        }
        app.handle_key(key(KeyCode::Enter));

        assert_eq!(app.active, 0, "deveria ter voltado pro buffer [sem nome]");
    }

    #[test]
    fn ctrl_b_sem_match_mostra_mensagem_e_nao_troca() {
        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        app.handle_key(ctrl_key(KeyCode::Char('b')));
        for c in "nao existe".chars() {
            app.handle_key(key(KeyCode::Char(c)));
        }
        app.handle_key(key(KeyCode::Enter));

        assert_eq!(app.active, 0);
        assert!(app.status_message.is_some());
    }

    #[test]
    fn o_e_b_nao_sao_mais_comandos_do_visual() {
        // o/b saíram do Visual na reforma de teclas — agora só existem
        // como Ctrl+o/Ctrl+b (classe sistema).
        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        app.handle_key(key(KeyCode::Char('o')));
        assert!(app.prompt.is_none());
        app.handle_key(key(KeyCode::Char('b')));
        assert!(app.prompt.is_none());
    }

    #[test]
    fn ctrl_q_lista_os_buffers_com_alteracoes_nao_salvas() {
        let dir = tempfile::tempdir().expect("diretório temporário");
        let path = dir.path().join("terceiro.txt");

        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        type_text(&mut app, "abc"); // modifica o buffer [sem nome]

        app.handle_key(ctrl_key(KeyCode::Char('o')));
        for c in path.to_str().unwrap().chars() {
            app.handle_key(key(KeyCode::Char(c)));
        }
        app.handle_key(key(KeyCode::Enter));
        type_text(&mut app, "def"); // modifica o segundo buffer também

        app.handle_key(ctrl_key(KeyCode::Char('q')));

        assert!(!app.should_quit);
        let message = app.status_message.as_ref().unwrap();
        assert!(message.contains("[sem nome]"));
        assert!(message.contains(path.to_str().unwrap()));

        app.handle_key(ctrl_key(KeyCode::Char('q'))); // confirma
        assert!(app.should_quit);
    }

    #[test]
    fn ctrl_q_sem_alteracoes_sai_direto() {
        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        app.handle_key(ctrl_key(KeyCode::Char('q')));
        assert!(app.should_quit);
    }

    #[test]
    fn ctrl_k_fecha_buffer_sem_alteracoes_direto() {
        let dir = tempfile::tempdir().expect("diretório temporário");
        let path = dir.path().join("fechar.txt");

        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        app.handle_key(ctrl_key(KeyCode::Char('o')));
        for c in path.to_str().unwrap().chars() {
            app.handle_key(key(KeyCode::Char(c)));
        }
        app.handle_key(key(KeyCode::Enter));
        assert_eq!(app.buffers.len(), 2);

        app.handle_key(ctrl_key(KeyCode::Char('k')));
        assert_eq!(app.buffers.len(), 1);
        assert_eq!(app.active, 0);
    }

    #[test]
    fn ctrl_k_com_alteracoes_pede_confirmacao() {
        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        type_text(&mut app, "abc");

        app.handle_key(ctrl_key(KeyCode::Char('k')));
        assert_eq!(app.buffers.len(), 1, "não deveria ter fechado ainda");
        assert!(app.status_message.is_some());

        app.handle_key(ctrl_key(KeyCode::Char('k'))); // confirma
        assert_eq!(app.buffers.len(), 1);
        assert_eq!(
            text(&app),
            "",
            "fechar o único buffer deixa um vazio no lugar"
        );
    }

    #[test]
    fn ctrl_k_no_ultimo_buffer_deixa_um_vazio_no_lugar() {
        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        app.handle_key(ctrl_key(KeyCode::Char('k')));
        assert_eq!(app.buffers.len(), 1);
        assert_eq!(app.active, 0);
        assert_eq!(app.document().path(), None);
    }

    #[test]
    fn expande_til_para_o_diretorio_home() {
        let home = std::env::var_os("HOME").expect("precisa de HOME pra este teste");
        assert_eq!(
            expand_path("~/notas/arquivo.txt"),
            PathBuf::from(&home).join("notas/arquivo.txt")
        );
        assert_eq!(expand_path("~"), PathBuf::from(&home));
    }

    #[test]
    fn resolve_caminho_relativo_a_partir_do_diretorio_atual() {
        let cwd = std::env::current_dir().expect("diretório atual deveria existir");
        assert_eq!(expand_path("arquivo.txt"), cwd.join("arquivo.txt"));
        assert_eq!(
            expand_path("sub/dir/arquivo.txt"),
            cwd.join("sub/dir/arquivo.txt")
        );
    }

    #[test]
    fn caminho_absoluto_fica_inalterado() {
        assert_eq!(
            expand_path("/tmp/arquivo.txt"),
            PathBuf::from("/tmp/arquivo.txt")
        );
    }

    #[test]
    fn selecionar_copiar_e_colar() {
        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        type_text(&mut app, "ola mundo");
        app.handle_key(key(KeyCode::Char('g')));
        app.handle_key(key(KeyCode::Char('g')));
        app.handle_key(key(KeyCode::Char('s'))); // marca a seleção
        for _ in 0..3 {
            app.handle_key(key(KeyCode::Char(';')));
        }
        assert_eq!(app.document().selection_range(), Some((0, 3)));

        app.handle_key(key(KeyCode::Char('c'))); // copia (com seleção, age na hora)
        assert_eq!(app.clipboard, "ola");
        assert_eq!(app.pending_clipboard_write.take(), Some("ola".to_string()));
        assert_eq!(
            app.document().selection_range(),
            None,
            "copiar deveria descartar a seleção"
        );

        app.handle_key(key(KeyCode::Char('K'))); // fim do buffer
        app.handle_key(key(KeyCode::Char('y'))); // pede pra colar
        assert!(app.pending_paste);
        app.apply_pending_paste(); // main.rs faria isso depois de tentar o clipboard do sistema

        assert_eq!(text(&app), "ola mundoola");
    }

    #[test]
    fn recortar_remove_o_trecho_e_pode_ser_colado_de_volta() {
        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        type_text(&mut app, "ola mundo");
        app.handle_key(key(KeyCode::Char('g')));
        app.handle_key(key(KeyCode::Char('g')));
        app.handle_key(key(KeyCode::Char('s')));
        for _ in 0..4 {
            app.handle_key(key(KeyCode::Char(';'))); // seleciona "ola "
        }

        app.handle_key(key(KeyCode::Char('x'))); // recorta (com seleção)
        assert_eq!(text(&app), "mundo");
        assert_eq!(app.clipboard, "ola ");

        app.handle_key(key(KeyCode::Char('y'))); // pede pra colar de volta
        app.apply_pending_paste();
        assert_eq!(text(&app), "ola mundo");
    }

    #[test]
    fn apagar_com_selecao_nao_copia_pra_area_de_transferencia() {
        // Diferente de x (recortar), d com seleção só apaga — não é
        // kill-style (docs/keys.md §4.2: a célula "com seleção" de d
        // não menciona área de transferência).
        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        type_text(&mut app, "ola mundo");
        app.handle_key(key(KeyCode::Char('g')));
        app.handle_key(key(KeyCode::Char('g')));
        app.handle_key(key(KeyCode::Char('s')));
        for _ in 0..3 {
            app.handle_key(key(KeyCode::Char(';')));
        }

        app.handle_key(key(KeyCode::Char('d')));
        assert_eq!(text(&app), " mundo");
        assert_eq!(
            app.clipboard, "",
            "d com seleção não deveria tocar o clipboard"
        );
    }

    #[test]
    fn colar_com_selecao_ativa_substitui_a_selecao() {
        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        type_text(&mut app, "ola mundo");
        app.handle_key(key(KeyCode::Char('g')));
        app.handle_key(key(KeyCode::Char('g')));
        app.handle_key(key(KeyCode::Char('s')));
        for _ in 0..3 {
            app.handle_key(key(KeyCode::Char(';'))); // seleciona "ola"
        }

        app.clipboard = "oi".to_string();
        app.handle_key(key(KeyCode::Char('y')));
        app.apply_pending_paste();

        assert_eq!(text(&app), "oi mundo");
    }

    #[test]
    fn esc_desfaz_a_selecao_ativa_no_documento() {
        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        type_text(&mut app, "ola mundo");
        app.handle_key(key(KeyCode::Char('g')));
        app.handle_key(key(KeyCode::Char('g')));
        app.handle_key(key(KeyCode::Char('s')));
        for _ in 0..3 {
            app.handle_key(key(KeyCode::Char(';')));
        }
        assert_eq!(app.document().selection_range(), Some((0, 3)));

        app.handle_key(key(KeyCode::Esc));

        assert_eq!(app.document().selection_range(), None);
        assert_eq!(
            text(&app),
            "ola mundo",
            "Esc só desfaz a seleção, não apaga nada"
        );
    }

    #[test]
    fn y_marca_pending_paste_sem_inserir_nada_sozinho() {
        // A inserção de verdade só acontece em apply_pending_paste,
        // depois que main.rs tenta atualizar o clipboard a partir do
        // sistema — y por si só não insere nada.
        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        app.clipboard = "preexistente".to_string();
        app.handle_key(key(KeyCode::Char('y')));

        assert!(app.pending_paste);
        assert_eq!(text(&app), "");
    }

    #[test]
    fn apply_pending_paste_cola_o_que_estiver_em_clipboard_no_momento() {
        // Simula main.rs: aperta y, "atualiza" clipboard a partir da
        // área de transferência do sistema (aqui, só trocando o campo
        // direto), e só então aplica.
        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        app.handle_key(key(KeyCode::Char('y')));
        assert!(app.pending_paste);

        app.clipboard = "do sistema".to_string();
        app.apply_pending_paste();

        assert_eq!(text(&app), "do sistema");
        assert!(!app.pending_paste);
    }

    #[test]
    fn apply_pending_paste_sem_pedido_pendente_nao_faz_nada() {
        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        app.clipboard = "nao deveria colar".to_string();
        app.apply_pending_paste();
        assert_eq!(text(&app), "");
    }

    #[test]
    fn select_all_seleciona_tudo_antes_de_copiar() {
        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        type_text(&mut app, "abc");
        app.handle_key(key(KeyCode::Char('S')));
        app.handle_key(key(KeyCode::Char('c')));
        assert_eq!(app.clipboard, "abc");
    }

    #[test]
    fn copiar_sem_selecao_espera_um_objeto() {
        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        type_text(&mut app, "abc");
        app.handle_key(key(KeyCode::Char('g')));
        app.handle_key(key(KeyCode::Char('g')));
        app.handle_key(key(KeyCode::Char('c'))); // sem seleção: vira verbo pendente
        assert_eq!(app.clipboard, "");
        app.handle_key(key(KeyCode::Char(';'))); // objeto: 1 caractere à frente
        assert_eq!(app.clipboard, "a");
    }

    #[test]
    fn bracketed_paste_e_sempre_texto_nunca_comando_do_visual() {
        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        type_text(&mut app, "abc");
        app.handle_key(key(KeyCode::Char('g')));
        app.handle_key(key(KeyCode::Char('g'))); // cursor no início, estado Visual

        // "dd" seria o verbo repetido se passasse pela máquina modal
        // (apagaria a linha inteira); colado, deve só inserir o texto.
        app.handle_paste("dd".to_string());

        assert_eq!(text(&app), "ddabc");
        assert_eq!(app.document().cursor_position().column, 2);
    }

    #[test]
    fn bracketed_paste_vai_pro_prompt_quando_ativo() {
        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        type_text(&mut app, "abc");
        app.handle_key(ctrl_key(KeyCode::Char('o'))); // abre o prompt de abrir arquivo

        app.handle_paste("/tmp/colado.txt".to_string());

        assert_eq!(app.prompt.as_ref().unwrap().input, "/tmp/colado.txt");
        assert_eq!(text(&app), "abc", "o documento não deve ter sido tocado");
    }

    #[test]
    fn verbo_mais_objeto_copia_pra_area_de_transferencia() {
        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        type_text(&mut app, "ola mundo");
        app.handle_key(key(KeyCode::Char('g')));
        app.handle_key(key(KeyCode::Char('g')));

        app.handle_key(key(KeyCode::Char('d')));
        app.handle_key(key(KeyCode::Char('w'))); // d w ; : apaga "ola"
        app.handle_key(key(KeyCode::Char(';')));
        assert_eq!(text(&app), " mundo");
        assert_eq!(app.clipboard, "ola");

        app.handle_key(key(KeyCode::Char('d')));
        app.handle_key(key(KeyCode::Char(':'))); // d : apaga até o fim da linha
        assert_eq!(text(&app), "");
        assert_eq!(app.clipboard, " mundo");
    }

    #[test]
    fn apagar_arquivo_inteiro_precisa_de_confirmacao_e_copia_pro_clipboard() {
        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        type_text(&mut app, "ola mundo");

        app.handle_key(key(KeyCode::Char('d')));
        app.handle_key(key(KeyCode::Char('a'))); // primeiro "d a": só avisa
        assert_eq!(text(&app), "ola mundo", "não deveria ter apagado ainda");
        assert!(app.status_message.is_some());

        app.handle_key(key(KeyCode::Char('d')));
        app.handle_key(key(KeyCode::Char('a'))); // segundo "d a": confirma
        assert_eq!(text(&app), "");
        assert_eq!(
            app.clipboard, "ola mundo",
            "d a deveria copiar o conteúdo antes de apagar, como qualquer outro objeto"
        );
    }

    #[test]
    fn apagar_arquivo_inteiro_e_cancelado_por_qualquer_outra_acao() {
        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        type_text(&mut app, "ola mundo");

        app.handle_key(key(KeyCode::Char('d')));
        app.handle_key(key(KeyCode::Char('a'))); // primeiro "d a"

        app.handle_key(key(KeyCode::Char('g')));
        app.handle_key(key(KeyCode::Char('g'))); // qualquer outra ação cancela

        app.handle_key(key(KeyCode::Char('d')));
        app.handle_key(key(KeyCode::Char('a'))); // "d a" de novo: é um primeiro pedido, não confirma
        assert_eq!(
            text(&app),
            "ola mundo",
            "a confirmação deveria ter sido cancelada pelo 'g g'"
        );
    }

    #[test]
    fn recortar_arquivo_inteiro_tambem_pede_confirmacao() {
        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        type_text(&mut app, "ola mundo");

        app.handle_key(key(KeyCode::Char('x')));
        app.handle_key(key(KeyCode::Char('a')));
        assert_eq!(text(&app), "ola mundo");

        app.handle_key(key(KeyCode::Char('x')));
        app.handle_key(key(KeyCode::Char('a')));
        assert_eq!(text(&app), "");
        assert_eq!(app.clipboard, "ola mundo");
    }

    #[test]
    fn copiar_arquivo_inteiro_nao_precisa_de_confirmacao() {
        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        type_text(&mut app, "ola mundo");

        app.handle_key(key(KeyCode::Char('c')));
        app.handle_key(key(KeyCode::Char('a')));
        assert_eq!(text(&app), "ola mundo", "copiar nunca apaga nada");
        assert_eq!(app.clipboard, "ola mundo");
    }

    #[test]
    fn goto_line_vai_para_a_linha_pedida() {
        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        type_text(&mut app, "um\ndois\ntres");

        app.handle_key(key(KeyCode::Char('g')));
        app.handle_key(key(KeyCode::Char('l')));
        assert_eq!(app.prompt.as_ref().unwrap().kind, PromptKind::GotoLine);
        for c in "2".chars() {
            app.handle_key(key(KeyCode::Char(c)));
        }
        app.handle_key(key(KeyCode::Enter));

        assert_eq!(app.document().cursor_position().line, 1);
    }

    #[test]
    fn unidade_sozinha_move_por_palavra_e_paragrafo() {
        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        type_text(&mut app, "ola mundo\n\ndois");
        app.handle_key(key(KeyCode::Char('g')));
        app.handle_key(key(KeyCode::Char('g')));

        app.handle_key(key(KeyCode::Char('w')));
        app.handle_key(key(KeyCode::Char(';'))); // w ; : próxima palavra
        assert_eq!(app.document().cursor_offset(), 3);

        app.handle_key(key(KeyCode::Char('W')));
        app.handle_key(key(KeyCode::Char('k'))); // W k: próximo parágrafo
        assert_eq!(app.document().cursor_position().line, 2);

        app.handle_key(key(KeyCode::Char('W')));
        app.handle_key(key(KeyCode::Char('l'))); // W l: parágrafo anterior
        assert_eq!(app.document().cursor_position().line, 0);
    }

    #[test]
    fn p_sozinho_nao_faz_nada() {
        // A unidade parágrafo virou "W" (Shift+w); "p" fica reservado.
        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        type_text(&mut app, "abc");
        app.handle_key(key(KeyCode::Char('g')));
        app.handle_key(key(KeyCode::Char('g')));
        let position_before = app.document().cursor_position();

        app.handle_key(key(KeyCode::Char('p')));

        assert_eq!(app.document().cursor_position(), position_before);
        assert!(app.status_message.is_none());
    }

    #[test]
    fn repetir_o_ultimo_composto_na_posicao_atual_do_cursor() {
        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        type_text(&mut app, "ola mundo bom");
        app.handle_key(key(KeyCode::Char('g')));
        app.handle_key(key(KeyCode::Char('g')));

        app.handle_key(key(KeyCode::Char('d')));
        app.handle_key(key(KeyCode::Char('w')));
        app.handle_key(key(KeyCode::Char(';'))); // d w ; : apaga "ola"
        assert_eq!(text(&app), " mundo bom");

        // o cursor agora está no espaço antes de "mundo" — repetir "d w
        // ;" dali apaga esse espaço e "mundo" (o objeto é recalculado
        // na posição atual, não replica literalmente o que foi apagado
        // antes).
        app.handle_key(key(KeyCode::Char('.'))); // repete na posição atual
        assert_eq!(text(&app), " bom");
    }

    fn attached_history(app: &mut App, dir: &std::path::Path) {
        let journal = Journal::open(dir).expect("abrir journal");
        app.attach_history(Arc::new(Mutex::new(Some(journal))));
    }

    /// Como `attached_history`, mas também liga o projeto (necessário
    /// pros comandos de checkpoint, que precisam de `history_dir()` —
    /// não só do journal) — devolve a pasta de histórico de verdade,
    /// pra quem testa ler os checkpoints direto por `nas_history`.
    fn attached_full_history(
        app: &mut App,
        project_dir: &std::path::Path,
        state_home_dir: &std::path::Path,
    ) -> PathBuf {
        let project = ProjectId::discover(project_dir).expect("descobrir projeto");
        let history_dir = project
            .ensure_history_dir(state_home_dir)
            .expect("criar pasta de histórico");
        let journal = Journal::open(&history_dir).expect("abrir journal");
        app.attach_history(Arc::new(Mutex::new(Some(journal))));
        app.attach_project(project, state_home_dir.to_path_buf());
        history_dir
    }

    fn type_str(app: &mut App, s: &str) {
        for c in s.chars() {
            app.handle_key(key(KeyCode::Char(c)));
        }
    }

    #[test]
    fn format_timestamp_bate_com_datas_conhecidas() {
        // conferido com `date -u -d @<segundos>`.
        assert_eq!(format_timestamp(0), "1970-01-01 00:00:00");
        assert_eq!(format_timestamp(1_609_459_200), "2021-01-01 00:00:00");
        assert_eq!(format_timestamp(1_732_000_000), "2024-11-19 07:06:40");
    }

    fn journal_entries(dir: &std::path::Path) -> Vec<nas_history::JournalEntry> {
        nas_history::read_entries(&nas_history::journal_path(dir)).expect("ler journal")
    }

    #[test]
    fn editar_e_sair_grava_no_journal_com_autor_e_comando() {
        let dir = tempfile::tempdir().expect("diretório temporário");
        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        attached_history(&mut app, dir.path());

        type_text(&mut app, "ola");

        app.handle_key(ctrl_key(KeyCode::Char('q'))); // alterações pendentes: só avisa
        assert!(!app.should_quit);
        app.handle_key(ctrl_key(KeyCode::Char('q'))); // confirma: sai e força o flush

        assert!(app.should_quit);
        let entries = journal_entries(dir.path());
        assert!(
            !entries.is_empty(),
            "a edição deveria ter chegado ao journal ao sair"
        );
        assert!(entries.iter().all(|e| e.author == Author::User));
        assert!(
            entries.iter().any(|e| e.inserted.contains('o')),
            "entradas: {entries:?}"
        );
    }

    #[test]
    fn desfazer_tambem_e_journalado_como_autor_user() {
        let dir = tempfile::tempdir().expect("diretório temporário");
        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        attached_history(&mut app, dir.path());

        type_text(&mut app, "ab");
        app.handle_key(key(KeyCode::Char('u'))); // desfaz a digitação de 'ab'
        app.flush_history();

        let entries = journal_entries(dir.path());
        let undo_entry = entries
            .iter()
            .find(|e| e.command == "undo")
            .expect("deveria haver uma entrada 'undo'");
        assert_eq!(undo_entry.author, Author::User);
    }

    #[test]
    fn verbo_mais_objeto_e_journalado_com_o_comando_canonico() {
        let dir = tempfile::tempdir().expect("diretório temporário");
        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        attached_history(&mut app, dir.path());

        type_text(&mut app, "ola mundo");
        app.handle_key(key(KeyCode::Char('g')));
        app.handle_key(key(KeyCode::Char('g')));
        app.handle_key(key(KeyCode::Char('d')));
        app.handle_key(key(KeyCode::Char('w'))); // d w ; : apaga "ola"
        app.handle_key(key(KeyCode::Char(';')));
        app.flush_history();

        let entries = journal_entries(dir.path());
        assert!(
            entries.iter().any(|e| e.command == "delete:word-forward"),
            "entradas: {entries:?}"
        );
    }

    #[test]
    fn ctrl_t_n_cria_checkpoint_com_buffers_abertos_que_mudaram() {
        let project_dir = tempfile::tempdir().expect("diretório temporário");
        let state_home = tempfile::tempdir().expect("diretório temporário");
        let file_path = project_dir.path().join("arquivo.txt");

        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        let history_dir = attached_full_history(&mut app, project_dir.path(), state_home.path());

        app.handle_key(ctrl_key(KeyCode::Char('o')));
        type_str(&mut app, file_path.to_str().unwrap());
        app.handle_key(key(KeyCode::Enter));
        type_text(&mut app, "ola mundo");

        app.handle_key(ctrl_key(KeyCode::Char('t')));
        app.handle_key(key(KeyCode::Char('n')));
        assert_eq!(
            app.prompt.as_ref().unwrap().kind,
            PromptKind::CheckpointName
        );
        type_str(&mut app, "meu-checkpoint");
        app.handle_key(key(KeyCode::Enter));

        assert!(
            app.status_message.as_ref().unwrap().contains("criado"),
            "{:?}",
            app.status_message
        );
        let checkpoints = nas_history::list_checkpoints(&history_dir).unwrap();
        assert_eq!(checkpoints.len(), 1);
        assert_eq!(checkpoints[0].files.len(), 1);
        assert_eq!(checkpoints[0].files[0].path, file_path.to_str().unwrap());
    }

    #[test]
    fn ctrl_t_n_sem_nada_mudado_nao_cria_checkpoint() {
        let project_dir = tempfile::tempdir().expect("diretório temporário");
        let state_home = tempfile::tempdir().expect("diretório temporário");

        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        let history_dir = attached_full_history(&mut app, project_dir.path(), state_home.path());

        // o único buffer é "[sem nome]" (sem caminho) — não entra em
        // nenhum checkpoint.
        app.handle_key(ctrl_key(KeyCode::Char('t')));
        app.handle_key(key(KeyCode::Char('n')));
        type_str(&mut app, "cp");
        app.handle_key(key(KeyCode::Enter));

        assert!(
            nas_history::list_checkpoints(&history_dir)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn ctrl_t_l_mostra_overlay_com_checkpoints_e_qualquer_tecla_fecha() {
        let project_dir = tempfile::tempdir().expect("diretório temporário");
        let state_home = tempfile::tempdir().expect("diretório temporário");
        let file_path = project_dir.path().join("arquivo.txt");

        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        attached_full_history(&mut app, project_dir.path(), state_home.path());

        app.handle_key(ctrl_key(KeyCode::Char('o')));
        type_str(&mut app, file_path.to_str().unwrap());
        app.handle_key(key(KeyCode::Enter));
        type_text(&mut app, "ola");
        app.handle_key(ctrl_key(KeyCode::Char('t')));
        app.handle_key(key(KeyCode::Char('n')));
        type_str(&mut app, "primeiro");
        app.handle_key(key(KeyCode::Enter));

        app.handle_key(ctrl_key(KeyCode::Char('t')));
        app.handle_key(key(KeyCode::Char('l')));
        let overlay = app.overlay.as_ref().expect("deveria ter aberto o overlay");
        assert!(overlay.lines.iter().any(|l| l.contains("primeiro")));

        app.handle_key(key(KeyCode::Esc));
        assert!(app.overlay.is_none());
    }

    #[test]
    fn ctrl_t_e_mostra_diff_entre_os_dois_ultimos_checkpoints() {
        let project_dir = tempfile::tempdir().expect("diretório temporário");
        let state_home = tempfile::tempdir().expect("diretório temporário");
        let file_path = project_dir.path().join("arquivo.txt");

        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        attached_full_history(&mut app, project_dir.path(), state_home.path());

        app.handle_key(ctrl_key(KeyCode::Char('o')));
        type_str(&mut app, file_path.to_str().unwrap());
        app.handle_key(key(KeyCode::Enter));
        type_text(&mut app, "versao um");
        app.handle_key(ctrl_key(KeyCode::Char('t')));
        app.handle_key(key(KeyCode::Char('n')));
        type_str(&mut app, "cp1");
        app.handle_key(key(KeyCode::Enter));

        type_text(&mut app, " e dois");
        app.handle_key(ctrl_key(KeyCode::Char('t')));
        app.handle_key(key(KeyCode::Char('n')));
        type_str(&mut app, "cp2");
        app.handle_key(key(KeyCode::Enter));

        app.handle_key(ctrl_key(KeyCode::Char('t')));
        app.handle_key(key(KeyCode::Char('e')));
        let overlay = app.overlay.as_ref().expect("deveria ter aberto o overlay");
        assert!(overlay.lines.iter().any(|l| l.contains("dois")));
    }

    #[test]
    fn ctrl_t_a_mostra_autoria_do_arquivo_atual() {
        let project_dir = tempfile::tempdir().expect("diretório temporário");
        let state_home = tempfile::tempdir().expect("diretório temporário");
        let file_path = project_dir.path().join("arquivo.txt");

        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        attached_full_history(&mut app, project_dir.path(), state_home.path());

        app.handle_key(ctrl_key(KeyCode::Char('o')));
        type_str(&mut app, file_path.to_str().unwrap());
        app.handle_key(key(KeyCode::Enter));
        type_text(&mut app, "ola");

        app.handle_key(ctrl_key(KeyCode::Char('t')));
        app.handle_key(key(KeyCode::Char('a')));
        let overlay = app.overlay.as_ref().expect("deveria ter aberto o overlay");
        assert!(overlay.lines.iter().any(|l| l.contains("você")));
        assert!(overlay.lines.iter().any(|l| l.contains("insert")));
    }

    #[test]
    fn ctrl_t_r_com_alteracoes_nao_salvas_pede_confirmacao_e_repetir_restaura() {
        let project_dir = tempfile::tempdir().expect("diretório temporário");
        let state_home = tempfile::tempdir().expect("diretório temporário");
        let file_path = project_dir.path().join("arquivo.txt");

        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        let history_dir = attached_full_history(&mut app, project_dir.path(), state_home.path());

        app.handle_key(ctrl_key(KeyCode::Char('o')));
        type_str(&mut app, file_path.to_str().unwrap());
        app.handle_key(key(KeyCode::Enter));
        type_text(&mut app, "versao um");
        app.handle_key(ctrl_key(KeyCode::Char('t')));
        app.handle_key(key(KeyCode::Char('n')));
        type_str(&mut app, "estavel");
        app.handle_key(key(KeyCode::Enter));

        app.handle_key(key(KeyCode::Char('g')));
        app.handle_key(key(KeyCode::Char('g')));
        app.handle_key(key(KeyCode::Char('d')));
        app.handle_key(key(KeyCode::Char('a'))); // d a: primeiro apaga tudo (pede confirmação própria)
        app.handle_key(key(KeyCode::Char('d')));
        app.handle_key(key(KeyCode::Char('a'))); // confirma
        type_text(&mut app, "quebrado"); // buffer agora tem alterações não salvas

        app.handle_key(ctrl_key(KeyCode::Char('t')));
        app.handle_key(key(KeyCode::Char('r')));
        type_str(&mut app, "estavel");
        app.handle_key(key(KeyCode::Enter));
        assert_eq!(
            text(&app),
            "quebrado",
            "a primeira tentativa deveria só avisar, sem restaurar ainda"
        );
        assert!(app.status_message.as_ref().unwrap().contains("Ctrl+t r"));

        app.handle_key(ctrl_key(KeyCode::Char('t')));
        app.handle_key(key(KeyCode::Char('r')));
        type_str(&mut app, "estavel");
        app.handle_key(key(KeyCode::Enter)); // repete com o mesmo nome: confirma

        assert_eq!(text(&app), "versao um");
        assert!(!app.document().is_modified(), "restaurar também salva");

        let checkpoints = nas_history::list_checkpoints(&history_dir).unwrap();
        assert!(
            checkpoints
                .iter()
                .any(|c| c.automatic && c.name.contains("antes de restaurar")),
            "deveria ter criado um checkpoint automático antes de restaurar: {checkpoints:?}"
        );
        assert_eq!(
            fs::read_to_string(&file_path).unwrap(),
            "versao um",
            "restaurar também escreve o arquivo em disco"
        );
    }

    #[test]
    fn restaurar_tambem_afeta_arquivo_que_nao_esta_aberto_como_buffer() {
        let project_dir = tempfile::tempdir().expect("diretório temporário");
        let state_home = tempfile::tempdir().expect("diretório temporário");
        let file_path = project_dir.path().join("fechado.txt");

        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        attached_full_history(&mut app, project_dir.path(), state_home.path());

        app.handle_key(ctrl_key(KeyCode::Char('o')));
        type_str(&mut app, file_path.to_str().unwrap());
        app.handle_key(key(KeyCode::Enter));
        type_text(&mut app, "conteudo original");
        app.handle_key(ctrl_key(KeyCode::Char('t')));
        app.handle_key(key(KeyCode::Char('n')));
        type_str(&mut app, "cp");
        app.handle_key(key(KeyCode::Enter));

        // salva (criar o checkpoint não limpa "alterações não
        // salvas") e fecha o buffer, depois muda o arquivo em disco
        // por fora do nas.
        app.handle_key(ctrl_key(KeyCode::Char('s')));
        app.handle_key(ctrl_key(KeyCode::Char('k')));
        assert_eq!(app.buffers.len(), 1);
        fs::write(&file_path, "mudou por fora").unwrap();

        app.handle_key(ctrl_key(KeyCode::Char('t')));
        app.handle_key(key(KeyCode::Char('r')));
        type_str(&mut app, "cp");
        app.handle_key(key(KeyCode::Enter));

        assert_eq!(fs::read_to_string(&file_path).unwrap(), "conteudo original");
    }

    #[test]
    fn ctrl_t_z_exige_a_confirmacao_exata_antes_de_apagar() {
        let project_dir = tempfile::tempdir().expect("diretório temporário");
        let state_home = tempfile::tempdir().expect("diretório temporário");
        let file_path = project_dir.path().join("arquivo.txt");

        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        let history_dir = attached_full_history(&mut app, project_dir.path(), state_home.path());

        app.handle_key(ctrl_key(KeyCode::Char('o')));
        type_str(&mut app, file_path.to_str().unwrap());
        app.handle_key(key(KeyCode::Enter));
        type_text(&mut app, "ola");
        app.handle_key(ctrl_key(KeyCode::Char('t')));
        app.handle_key(key(KeyCode::Char('n')));
        type_str(&mut app, "cp");
        app.handle_key(key(KeyCode::Enter));
        assert_eq!(
            nas_history::list_checkpoints(&history_dir).unwrap().len(),
            1
        );

        app.handle_key(ctrl_key(KeyCode::Char('t')));
        app.handle_key(key(KeyCode::Char('z')));
        assert_eq!(
            app.prompt.as_ref().unwrap().kind,
            PromptKind::DeleteHistoryConfirm
        );
        type_str(&mut app, "sim, apagar");
        app.handle_key(key(KeyCode::Enter));

        assert_eq!(
            nas_history::list_checkpoints(&history_dir).unwrap().len(),
            1,
            "confirmação errada não deveria ter apagado nada"
        );

        app.handle_key(ctrl_key(KeyCode::Char('t')));
        app.handle_key(key(KeyCode::Char('z')));
        type_str(&mut app, "apagar");
        app.handle_key(key(KeyCode::Enter));

        assert!(
            nas_history::list_checkpoints(&history_dir)
                .unwrap()
                .is_empty()
        );

        // o histórico continua funcionando depois de apagado — o
        // journal foi reaberto numa pasta recriada.
        type_text(&mut app, "mais texto");
        app.flush_history();
        let entries = journal_entries(&history_dir);
        assert!(!entries.is_empty(), "o journal deveria ter sido reaberto");
    }

    #[test]
    fn restaurar_um_checkpoint_antigo_tambem_reverte_arquivo_tocado_depois_dele() {
        let project_dir = tempfile::tempdir().expect("diretório temporário");
        let state_home = tempfile::tempdir().expect("diretório temporário");
        let x_path = project_dir.path().join("x.txt");
        let y_path = project_dir.path().join("y.txt");
        // y.txt já existe no disco (conteúdo original) antes de o nas
        // tocar nele — é o "arquivo novo" (novo pra este checkpoint,
        // não necessariamente um arquivo que nunca existiu).
        fs::write(&y_path, "y original").unwrap();

        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        attached_full_history(&mut app, project_dir.path(), state_home.path());

        // checkpoint A: só x.txt foi tocado até aqui.
        app.handle_key(ctrl_key(KeyCode::Char('o')));
        type_str(&mut app, x_path.to_str().unwrap());
        app.handle_key(key(KeyCode::Enter));
        type_text(&mut app, "x conteudo");
        app.handle_key(ctrl_key(KeyCode::Char('t')));
        app.handle_key(key(KeyCode::Char('n')));
        type_str(&mut app, "checkpoint-a");
        app.handle_key(key(KeyCode::Enter));

        // só agora y.txt é editado por primeira vez — depois de A.
        app.handle_key(ctrl_key(KeyCode::Char('o')));
        type_str(&mut app, y_path.to_str().unwrap());
        app.handle_key(key(KeyCode::Enter));
        app.handle_key(key(KeyCode::Char('K'))); // fim do buffer
        type_text(&mut app, " mudou");

        // checkpoint B: agora y.txt também está rastreado.
        app.handle_key(ctrl_key(KeyCode::Char('t')));
        app.handle_key(key(KeyCode::Char('n')));
        type_str(&mut app, "checkpoint-b");
        app.handle_key(key(KeyCode::Enter));

        // restaurar A: y.txt não está no manifesto de A (só passou a
        // ser rastreado depois) — primeira tentativa só avisa, sem
        // restaurar nada ainda (y.txt nunca foi salvo, então checar o
        // disco não provaria nada aqui — ele já é "y original" desde
        // o início; o que importa é que o buffer em memória, ainda
        // modificado, não tenha sido tocado).
        app.handle_key(ctrl_key(KeyCode::Char('t')));
        app.handle_key(key(KeyCode::Char('r')));
        type_str(&mut app, "checkpoint-a");
        app.handle_key(key(KeyCode::Enter));
        assert_eq!(
            text(&app),
            "y original mudou",
            "a primeira tentativa não deveria ter restaurado nada"
        );
        assert!(app.document().is_modified());
        assert!(
            app.status_message.as_ref().unwrap().contains("Ctrl+t r"),
            "{:?}",
            app.status_message
        );

        app.handle_key(ctrl_key(KeyCode::Char('t')));
        app.handle_key(key(KeyCode::Char('r')));
        type_str(&mut app, "checkpoint-a");
        app.handle_key(key(KeyCode::Enter)); // confirma

        assert_eq!(
            fs::read_to_string(&x_path).unwrap(),
            "x conteudo",
            "x.txt já estava no checkpoint A"
        );
        assert_eq!(
            fs::read_to_string(&y_path).unwrap(),
            "y original",
            "y.txt deveria ter voltado ao conteúdo que tinha quando A foi criado"
        );
    }

    #[test]
    fn checkpoint_inclui_arquivo_editado_salvo_e_fechado() {
        let project_dir = tempfile::tempdir().expect("diretório temporário");
        let state_home = tempfile::tempdir().expect("diretório temporário");
        let file_path = project_dir.path().join("fechado.txt");

        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        let history_dir = attached_full_history(&mut app, project_dir.path(), state_home.path());

        app.handle_key(ctrl_key(KeyCode::Char('o')));
        type_str(&mut app, file_path.to_str().unwrap());
        app.handle_key(key(KeyCode::Enter));
        type_text(&mut app, "conteudo");
        app.handle_key(ctrl_key(KeyCode::Char('s'))); // salva
        app.handle_key(ctrl_key(KeyCode::Char('k'))); // fecha (já salvo, sem confirmação)
        assert_eq!(
            app.buffers.len(),
            1,
            "deveria ter fechado direto, sem pedir confirmação"
        );

        app.handle_key(ctrl_key(KeyCode::Char('t')));
        app.handle_key(key(KeyCode::Char('n')));
        type_str(&mut app, "cp");
        app.handle_key(key(KeyCode::Enter));

        let checkpoints = nas_history::list_checkpoints(&history_dir).unwrap();
        assert_eq!(checkpoints.len(), 1);
        assert!(
            checkpoints[0]
                .files
                .iter()
                .any(|f| f.path == file_path.to_str().unwrap()),
            "o checkpoint deveria incluir o arquivo fechado: {:?}",
            checkpoints[0].files
        );
    }

    #[test]
    #[cfg(unix)]
    fn restaurar_arquivo_sem_permissao_de_escrita_mostra_o_erro() {
        use std::os::unix::fs::PermissionsExt;

        let project_dir = tempfile::tempdir().expect("diretório temporário");
        let state_home = tempfile::tempdir().expect("diretório temporário");
        let readonly_subdir = project_dir.path().join("somente-leitura");
        fs::create_dir(&readonly_subdir).unwrap();
        let file_path = readonly_subdir.join("arquivo.txt");

        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        attached_full_history(&mut app, project_dir.path(), state_home.path());

        app.handle_key(ctrl_key(KeyCode::Char('o')));
        type_str(&mut app, file_path.to_str().unwrap());
        app.handle_key(key(KeyCode::Enter));
        type_text(&mut app, "conteudo original");
        app.handle_key(ctrl_key(KeyCode::Char('t')));
        app.handle_key(key(KeyCode::Char('n')));
        type_str(&mut app, "cp");
        app.handle_key(key(KeyCode::Enter));

        app.handle_key(ctrl_key(KeyCode::Char('s')));
        app.handle_key(ctrl_key(KeyCode::Char('k')));
        // a escrita atômica precisa criar um arquivo temporário na
        // mesma pasta antes de renomear por cima do original — sem
        // permissão de escrita na pasta, isso falha.
        fs::set_permissions(&readonly_subdir, fs::Permissions::from_mode(0o555)).unwrap();

        app.handle_key(ctrl_key(KeyCode::Char('t')));
        app.handle_key(key(KeyCode::Char('r')));
        type_str(&mut app, "cp");
        app.handle_key(key(KeyCode::Enter));

        let message = app
            .status_message
            .as_ref()
            .expect("deveria ter uma mensagem de status");
        assert!(message.contains("FALHARAM"), "{message:?}");
        assert!(message.contains("arquivo.txt"), "{message:?}");

        // devolve a permissão — o Drop do tempdir precisa conseguir
        // remover o conteúdo da pasta.
        fs::set_permissions(&readonly_subdir, fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[test]
    fn trocar_de_buffer_detecta_mudanca_externa_e_avisa() {
        let project_dir = tempfile::tempdir().expect("diretório temporário");
        let state_home = tempfile::tempdir().expect("diretório temporário");
        let a_path = project_dir.path().join("a.txt");
        let b_path = project_dir.path().join("b.txt");
        fs::write(&a_path, "a original").unwrap();
        fs::write(&b_path, "b original").unwrap();

        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        attached_full_history(&mut app, project_dir.path(), state_home.path());

        app.handle_key(ctrl_key(KeyCode::Char('o')));
        type_str(&mut app, a_path.to_str().unwrap());
        app.handle_key(key(KeyCode::Enter));
        app.handle_key(ctrl_key(KeyCode::Char('o')));
        type_str(&mut app, b_path.to_str().unwrap());
        app.handle_key(key(KeyCode::Enter));

        std::thread::sleep(std::time::Duration::from_millis(10));
        fs::write(&a_path, "a mudou por fora").unwrap();

        app.handle_key(ctrl_key(KeyCode::Char('b'))); // Ctrl+b: trocar de buffer
        type_str(&mut app, "a.txt");
        app.handle_key(key(KeyCode::Enter));

        assert!(app.document().external_change_pending());
        assert!(
            app.status_message
                .as_ref()
                .unwrap()
                .contains("mudou no disco"),
            "{:?}",
            app.status_message
        );
    }

    #[test]
    fn salvar_com_mudanca_externa_pede_confirmacao_e_journala_a_mudanca_externa() {
        let project_dir = tempfile::tempdir().expect("diretório temporário");
        let state_home = tempfile::tempdir().expect("diretório temporário");
        let path = project_dir.path().join("arquivo.txt");
        fs::write(&path, "original").unwrap();

        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        let history_dir = attached_full_history(&mut app, project_dir.path(), state_home.path());

        app.handle_key(ctrl_key(KeyCode::Char('o')));
        type_str(&mut app, path.to_str().unwrap());
        app.handle_key(key(KeyCode::Enter));
        app.handle_key(key(KeyCode::Char('K'))); // fim do buffer
        type_text(&mut app, " e editado"); // edição local não salva

        std::thread::sleep(std::time::Duration::from_millis(10));
        fs::write(&path, "mudou por fora").unwrap();

        app.handle_key(ctrl_key(KeyCode::Char('s'))); // primeira tentativa: só avisa
        assert!(
            app.status_message
                .as_ref()
                .unwrap()
                .contains("Ctrl+S de novo"),
            "{:?}",
            app.status_message
        );
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "mudou por fora",
            "não deveria ter sobrescrito ainda"
        );

        app.handle_key(ctrl_key(KeyCode::Char('s'))); // confirma
        assert_eq!(fs::read_to_string(&path).unwrap(), "original e editado");

        app.flush_history();
        let entries = journal_entries(&history_dir);
        assert!(
            entries
                .iter()
                .any(|e| e.author == Author::External && e.inserted == "mudou por fora"),
            "entradas: {entries:?}"
        );
    }

    #[test]
    fn ctrl_r_recarrega_e_confirma_antes_de_descartar_edicoes_locais() {
        let project_dir = tempfile::tempdir().expect("diretório temporário");
        let state_home = tempfile::tempdir().expect("diretório temporário");
        let path = project_dir.path().join("arquivo.txt");
        fs::write(&path, "original").unwrap();

        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        attached_full_history(&mut app, project_dir.path(), state_home.path());

        app.handle_key(ctrl_key(KeyCode::Char('o')));
        type_str(&mut app, path.to_str().unwrap());
        app.handle_key(key(KeyCode::Enter));
        app.handle_key(key(KeyCode::Char('K')));
        type_text(&mut app, " edicao local");

        app.handle_key(ctrl_key(KeyCode::Char('r'))); // primeira vez: avisa
        assert_eq!(text(&app), "original edicao local");
        assert!(
            app.status_message
                .as_ref()
                .unwrap()
                .contains("Ctrl+R de novo"),
            "{:?}",
            app.status_message
        );

        app.handle_key(ctrl_key(KeyCode::Char('r'))); // confirma
        assert_eq!(text(&app), "original");
        assert!(!app.document().is_modified());
    }

    #[test]
    fn handle_focus_gained_confere_todos_os_buffers_abertos() {
        let project_dir = tempfile::tempdir().expect("diretório temporário");
        let state_home = tempfile::tempdir().expect("diretório temporário");
        let a_path = project_dir.path().join("a.txt");
        fs::write(&a_path, "original").unwrap();

        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        attached_full_history(&mut app, project_dir.path(), state_home.path());

        app.handle_key(ctrl_key(KeyCode::Char('o')));
        type_str(&mut app, a_path.to_str().unwrap());
        app.handle_key(key(KeyCode::Enter));

        std::thread::sleep(std::time::Duration::from_millis(10));
        fs::write(&a_path, "mudou por fora").unwrap();

        app.handle_focus_gained();

        assert!(app.document().external_change_pending());
        assert!(
            app.status_message
                .as_ref()
                .unwrap()
                .contains("mudou no disco")
        );
    }

    #[test]
    fn mudanca_externa_nao_marca_arquivo_como_rastreado() {
        let project_dir = tempfile::tempdir().expect("diretório temporário");
        let state_home = tempfile::tempdir().expect("diretório temporário");
        let path = project_dir.path().join("arquivo.txt");
        fs::write(&path, "original").unwrap();

        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        let history_dir = attached_full_history(&mut app, project_dir.path(), state_home.path());

        app.handle_key(ctrl_key(KeyCode::Char('o')));
        type_str(&mut app, path.to_str().unwrap());
        app.handle_key(key(KeyCode::Enter));

        std::thread::sleep(std::time::Duration::from_millis(10));
        fs::write(&path, "mudou por fora").unwrap();
        app.check_external_changes();

        assert!(
            nas_history::list_tracked(&history_dir).unwrap().is_empty(),
            "uma mudança externa não deveria, por si só, iniciar o rastreamento do arquivo"
        );
    }
}
