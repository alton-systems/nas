//! Documento do editor: envolve `helix-core` (rope, transações, histórico).
//!
//! Este é o único módulo do crate que importa `helix_core` diretamente — ver
//! docs/decisions/0001-helix-como-biblioteca.md. O resto do nas (nas-term,
//! nas-keys) só vê os tipos definidos aqui (`Document`, `Position`), nunca
//! `helix_core::Rope`, `Transaction` ou `History`.

use std::borrow::Cow;
use std::fmt;
use std::fs;
use std::io::{self, BufReader, BufWriter};
use std::path::{Path, PathBuf};

use helix_core::chars::{CharCategory, categorize_char};
use helix_core::line_ending::LineEnding;
use helix_core::{Rope, Selection, Tendril, Transaction, graphemes, history, line_ending};

/// Erro ao abrir um arquivo: além de erros de E/S comuns, um arquivo que
/// não é UTF-8 válido é recusado — nunca chega a existir um `Document` com
/// conteúdo corrompido, então salvar não tem como sobrescrever o original.
#[derive(Debug)]
pub enum OpenError {
    Io(io::Error),
    NotUtf8,
}

impl fmt::Display for OpenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OpenError::Io(err) => write!(f, "{err}"),
            OpenError::NotUtf8 => write!(f, "o arquivo não é UTF-8 válido"),
        }
    }
}

impl std::error::Error for OpenError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            OpenError::Io(err) => Some(err),
            OpenError::NotUtf8 => None,
        }
    }
}

impl From<OpenError> for io::Error {
    fn from(err: OpenError) -> io::Error {
        match err {
            OpenError::Io(err) => err,
            OpenError::NotUtf8 => io::Error::new(io::ErrorKind::InvalidData, err.to_string()),
        }
    }
}

/// Posição lógica do cursor no documento: linha e coluna em caracteres,
/// ambas 0-indexadas.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Position {
    pub line: usize,
    pub column: usize,
}

/// O objeto de um verbo (ver `nas_keys::Object`, docs/keys.md §4.3):
/// a região sobre a qual um verbo age, relativa ao cursor atual. Este
/// é o tipo equivalente do lado do documento — `nas-keys` não conhece
/// `Document`, então quem chama (`nas-term`) traduz de um pro outro.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegionObject {
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

/// Um buffer de texto editável, com desfazer.
pub struct Document {
    rope: Rope,
    cursor: usize,
    /// Onde a seleção começou (`set-mark-command`) — `None` quando não
    /// há seleção ativa. O cursor é a outra ponta; mover o cursor
    /// normalmente estende a seleção, sem precisar de nenhum estado
    /// extra, igual ao helix_core::Selection/Range de duas pontas.
    selection_anchor: Option<usize>,
    history: history::History,
    path: Option<PathBuf>,
    modified: bool,
    line_ending: LineEnding,
}

impl Document {
    /// Um buffer vazio, sem arquivo associado.
    pub fn empty() -> Self {
        Self {
            rope: Rope::new(),
            cursor: 0,
            selection_anchor: None,
            history: history::History::default(),
            path: None,
            modified: false,
            line_ending: LineEnding::LF,
        }
    }

    /// Abre o arquivo em `path`. Se o arquivo não existir ainda, retorna um
    /// buffer vazio com o caminho associado (para criar o arquivo ao
    /// salvar). Recusa arquivos que não são UTF-8 válido — nesse caso quem
    /// chama decide o que mostrar, mas nenhum `Document` chega a existir
    /// com o conteúdo inválido.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, OpenError> {
        let path = path.as_ref();
        let rope = match fs::File::open(path) {
            Ok(file) => Rope::from_reader(BufReader::new(file)).map_err(|err| {
                if err.kind() == io::ErrorKind::InvalidData {
                    OpenError::NotUtf8
                } else {
                    OpenError::Io(err)
                }
            })?,
            Err(err) if err.kind() == io::ErrorKind::NotFound => Rope::new(),
            Err(err) => return Err(OpenError::Io(err)),
        };
        let line_ending = line_ending::auto_detect_line_ending(&rope).unwrap_or(LineEnding::LF);
        tracing::info!(path = %path.display(), "documento aberto");
        Ok(Self {
            rope,
            cursor: 0,
            selection_anchor: None,
            history: history::History::default(),
            path: Some(path.to_path_buf()),
            modified: false,
            line_ending,
        })
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub fn is_modified(&self) -> bool {
        self.modified
    }

    /// Escreve o buffer no caminho associado, atomicamente (arquivo
    /// temporário na mesma pasta + fsync + rename por cima do original,
    /// preservando as permissões). Retorna `false` se o documento não
    /// tiver um caminho (ainda não há "salvar como").
    pub fn save(&mut self) -> io::Result<bool> {
        let Some(path) = &self.path else {
            return Ok(false);
        };
        write_atomic(path, &self.rope)?;
        self.modified = false;
        tracing::info!(path = %path.display(), "documento salvo");
        Ok(true)
    }

    pub fn len_lines(&self) -> usize {
        self.rope.len_lines()
    }

    /// Texto da linha `idx` (0-indexada), sem o terminador de linha.
    pub fn line(&self, idx: usize) -> Cow<'_, str> {
        let start = self.rope.line_to_char(idx);
        let end = line_ending::line_end_char_index(&self.rope.slice(..), idx);
        Cow::from(self.rope.slice(start..end))
    }

    /// Linha e coluna (em caracteres) do cursor, 0-indexadas.
    pub fn cursor_position(&self) -> Position {
        let line = self.rope.char_to_line(self.cursor);
        let line_start = self.rope.line_to_char(line);
        let column = grapheme_count(&self.rope, line_start, self.cursor);
        Position { line, column }
    }

    /// Coluna visual (em células de terminal) do cursor na linha atual —
    /// difere da coluna em caracteres quando há caracteres largos (CJK,
    /// emoji) antes do cursor.
    pub fn cursor_visual_column(&self) -> usize {
        let line = self.rope.char_to_line(self.cursor);
        let line_start = self.rope.line_to_char(line);
        grapheme_width_between(&self.rope, line_start, self.cursor)
    }

    pub fn move_left(&mut self) {
        self.cursor = graphemes::prev_grapheme_boundary(self.rope.slice(..), self.cursor);
    }

    pub fn move_right(&mut self) {
        self.cursor = graphemes::next_grapheme_boundary(self.rope.slice(..), self.cursor);
    }

    pub fn move_up(&mut self) {
        let line = self.rope.char_to_line(self.cursor);
        if line == 0 {
            return;
        }
        self.move_to_visual_column_on_line(line - 1, self.cursor_visual_column());
    }

    pub fn move_down(&mut self) {
        let line = self.rope.char_to_line(self.cursor);
        if line + 1 >= self.rope.len_lines() {
            return;
        }
        self.move_to_visual_column_on_line(line + 1, self.cursor_visual_column());
    }

    /// Vai para o início da linha atual (coluna 0), como `beginning-of-line`.
    pub fn move_to_line_start(&mut self) {
        let line = self.rope.char_to_line(self.cursor);
        self.cursor = self.rope.line_to_char(line);
    }

    /// Vai para o fim da linha atual, antes do terminador de linha, como
    /// `end-of-line`.
    pub fn move_to_line_end(&mut self) {
        let line = self.rope.char_to_line(self.cursor);
        self.cursor = line_ending::line_end_char_index(&self.rope.slice(..), line);
    }

    /// Vai para o início do buffer, como `beginning-of-buffer`.
    pub fn move_to_buffer_start(&mut self) {
        self.cursor = 0;
    }

    /// Vai para o fim do buffer, como `end-of-buffer`.
    pub fn move_to_buffer_end(&mut self) {
        self.cursor = self.rope.len_chars();
    }

    /// Posição do cursor em caracteres, desde o início do documento —
    /// para guardar/restaurar de onde uma busca começou.
    pub fn cursor_offset(&self) -> usize {
        self.cursor
    }

    /// Move o cursor para uma posição em caracteres, sem passar do fim
    /// do documento.
    pub fn set_cursor(&mut self, offset: usize) {
        self.cursor = offset.min(self.rope.len_chars());
    }

    /// Acha a primeira ocorrência de `query` a partir de `from`
    /// (inclusive), dando a volta pro início do documento se não achar
    /// antes do fim — como `isearch-forward`, mas sem parar quando a
    /// busca "sai" do documento. `None` se `query` for vazia ou não
    /// aparecer em lugar nenhum.
    pub fn find_from(&self, from: usize, query: &str) -> Option<usize> {
        let query: Vec<char> = query.chars().collect();
        if query.is_empty() {
            return None;
        }
        let len = self.rope.len_chars();
        if len == 0 {
            return None;
        }
        let from = from.min(len);
        (from..len)
            .chain(0..from)
            .find(|&pos| self.matches_at(pos, &query))
    }

    /// Marca o início de uma seleção no cursor atual — como
    /// `set-mark-command`. Mover o cursor depois disso estende a
    /// seleção; chamar de novo move a marca pro cursor atual.
    pub fn start_selection(&mut self) {
        self.selection_anchor = Some(self.cursor);
    }

    /// Seleciona o documento inteiro — como `mark-whole-buffer`.
    pub fn select_all(&mut self) {
        self.selection_anchor = Some(0);
        self.cursor = self.rope.len_chars();
    }

    /// Descarta a seleção ativa, se houver.
    pub fn clear_selection(&mut self) {
        self.selection_anchor = None;
    }

    /// Início e fim (em caracteres, start ≤ end) da seleção ativa, se
    /// houver uma.
    pub fn selection_range(&self) -> Option<(usize, usize)> {
        self.selection_anchor.map(|anchor| {
            if anchor <= self.cursor {
                (anchor, self.cursor)
            } else {
                (self.cursor, anchor)
            }
        })
    }

    /// O texto selecionado, se houver uma seleção (mesmo que vazia).
    pub fn selected_text(&self) -> Option<String> {
        let (start, end) = self.selection_range()?;
        Some(self.rope.slice(start..end).to_string())
    }

    /// Apaga o texto selecionado e descarta a seleção — como
    /// `kill-region`. Não faz nada se não houver seleção.
    pub fn delete_selection(&mut self) {
        let Some((start, end)) = self.selection_range() else {
            return;
        };
        self.delete_range(start, end);
        self.cursor = start;
        self.selection_anchor = None;
    }

    /// Posição absoluta (em caracteres) do início da linha `idx` — para
    /// quem renderiza a tela saber onde cada linha visível começa, ex.
    /// pra destacar a seleção.
    pub fn line_start_offset(&self, idx: usize) -> usize {
        self.rope.line_to_char(idx)
    }

    /// Vai para o início da linha `line` (1-indexada, como a maioria dos
    /// editores mostra o número de linha) — como `goto-line`. Números
    /// fora do documento vão para a primeira ou última linha.
    pub fn goto_line(&mut self, line: usize) {
        let target = line.saturating_sub(1).min(self.rope.len_lines() - 1);
        self.cursor = self.rope.line_to_char(target);
    }

    /// Vai para o próximo limite de palavra — como `forward-word`.
    pub fn move_word_forward(&mut self) {
        self.cursor = self.next_word_boundary(self.cursor);
    }

    /// Vai para o limite de palavra anterior — como `backward-word`.
    pub fn move_word_backward(&mut self) {
        self.cursor = self.prev_word_boundary(self.cursor);
    }

    /// Vai para o início do próximo parágrafo (a primeira linha não
    /// vazia depois de uma linha vazia) — como `forward-paragraph`.
    pub fn move_paragraph_forward(&mut self) {
        self.cursor = self.adjacent_paragraph_boundary(true);
    }

    /// Vai para o início do parágrafo anterior — como `backward-paragraph`.
    pub fn move_paragraph_backward(&mut self) {
        self.cursor = self.adjacent_paragraph_boundary(false);
    }

    /// A região (início, fim — início ≤ fim, em caracteres) que `object`
    /// descreve a partir do cursor atual — docs/keys.md §4.3. Pura: não
    /// move o cursor nem altera o documento, então serve tanto pra
    /// apagar/recortar (via [`Document::delete_region`]) quanto pra
    /// copiar (via [`Document::text_in`], sem apagar nada).
    pub fn region_for(&self, object: RegionObject) -> (usize, usize) {
        match object {
            RegionObject::CharLeft => (
                graphemes::prev_grapheme_boundary(self.rope.slice(..), self.cursor),
                self.cursor,
            ),
            RegionObject::CharRight => (
                self.cursor,
                graphemes::next_grapheme_boundary(self.rope.slice(..), self.cursor),
            ),
            RegionObject::LineStart => (self.rope.line_to_char(self.current_line()), self.cursor),
            RegionObject::LineEnd => (
                self.cursor,
                line_ending::line_end_char_index(&self.rope.slice(..), self.current_line()),
            ),
            RegionObject::WordBackward => (self.prev_word_boundary(self.cursor), self.cursor),
            RegionObject::WordForward => (self.cursor, self.next_word_boundary(self.cursor)),
            RegionObject::LineDown => {
                self.line_region(self.current_line(), self.current_line() + 1)
            }
            RegionObject::LineUp => {
                self.line_region(self.current_line().saturating_sub(1), self.current_line())
            }
            RegionObject::BufferEnd => {
                self.line_region(self.current_line(), self.rope.len_lines() - 1)
            }
            RegionObject::BufferStart => self.line_region(0, self.current_line()),
            RegionObject::ParagraphForward => {
                let target = self
                    .rope
                    .char_to_line(self.adjacent_paragraph_boundary(true));
                self.line_region(self.current_line(), target)
            }
            RegionObject::ParagraphBackward => {
                let target = self
                    .rope
                    .char_to_line(self.adjacent_paragraph_boundary(false));
                self.line_region(target, self.current_line())
            }
            RegionObject::CurrentLine => self.line_region(self.current_line(), self.current_line()),
            RegionObject::WholeBuffer => (0, self.rope.len_chars()),
        }
    }

    /// O texto entre `from` e `to` (em caracteres), sem alterar o
    /// documento nem o cursor — para copiar uma região.
    pub fn text_in(&self, from: usize, to: usize) -> String {
        self.rope.slice(from..to).to_string()
    }

    /// Apaga o texto entre `from` e `to` (em caracteres) e devolve o
    /// que foi apagado, para quem chama copiar pra área de
    /// transferência — para apagar ou recortar uma região. O cursor
    /// vai para `from`.
    pub fn delete_region(&mut self, from: usize, to: usize) -> String {
        let text = self.rope.slice(from..to).to_string();
        self.delete_range(from, to);
        self.cursor = from;
        text
    }

    fn current_line(&self) -> usize {
        self.rope.char_to_line(self.cursor)
    }

    /// Linhas inteiras de `line_a` até `line_b` (em qualquer ordem),
    /// inclusive, com a quebra de linha de cada uma — exceto a última,
    /// se for a última linha do documento (sem quebra final).
    fn line_region(&self, line_a: usize, line_b: usize) -> (usize, usize) {
        let (lo, hi) = if line_a <= line_b {
            (line_a, line_b)
        } else {
            (line_b, line_a)
        };
        let start = self.rope.line_to_char(lo);
        let end = if hi + 1 < self.rope.len_lines() {
            self.rope.line_to_char(hi + 1)
        } else {
            self.rope.len_chars()
        };
        (start, end)
    }

    fn next_word_boundary(&self, from: usize) -> usize {
        let len = self.rope.len_chars();
        let mut pos = from;
        while pos < len && categorize_char(self.rope.char(pos)) != CharCategory::Word {
            pos += 1;
        }
        while pos < len && categorize_char(self.rope.char(pos)) == CharCategory::Word {
            pos += 1;
        }
        pos
    }

    fn prev_word_boundary(&self, from: usize) -> usize {
        let mut pos = from;
        while pos > 0 && categorize_char(self.rope.char(pos - 1)) != CharCategory::Word {
            pos -= 1;
        }
        while pos > 0 && categorize_char(self.rope.char(pos - 1)) == CharCategory::Word {
            pos -= 1;
        }
        pos
    }

    fn is_line_blank(&self, line: usize) -> bool {
        self.rope.line_to_char(line) == line_ending::line_end_char_index(&self.rope.slice(..), line)
    }

    /// Posição do início do parágrafo adjacente (`forward`: o próximo;
    /// senão, o anterior) — um parágrafo termina numa linha vazia.
    fn adjacent_paragraph_boundary(&self, forward: bool) -> usize {
        let total = self.rope.len_lines();
        let mut line = self.rope.char_to_line(self.cursor);
        if forward {
            while line < total && !self.is_line_blank(line) {
                line += 1;
            }
            while line < total && self.is_line_blank(line) {
                line += 1;
            }
            if line >= total {
                self.rope.len_chars()
            } else {
                self.rope.line_to_char(line)
            }
        } else {
            // Primeiro pula linhas em branco logo antes (o "vão" entre
            // parágrafos), depois pula o parágrafo em si — pra parar no
            // começo dele, não no meio.
            while line > 0 && self.is_line_blank(line - 1) {
                line -= 1;
            }
            while line > 0 && !self.is_line_blank(line - 1) {
                line -= 1;
            }
            self.rope.line_to_char(line)
        }
    }

    fn matches_at(&self, pos: usize, query: &[char]) -> bool {
        if pos + query.len() > self.rope.len_chars() {
            return false;
        }
        self.rope
            .chars_at(pos)
            .take(query.len())
            .eq(query.iter().copied())
    }

    fn move_to_visual_column_on_line(&mut self, line: usize, target_column: usize) {
        let line_start = self.rope.line_to_char(line);
        let line_end = line_ending::line_end_char_index(&self.rope.slice(..), line);

        let mut pos = line_start;
        let mut width = 0;
        while pos < line_end {
            let next = graphemes::next_grapheme_boundary(self.rope.slice(..), pos);
            let grapheme_width = grapheme_width_between(&self.rope, pos, next);
            if width + grapheme_width > target_column {
                break;
            }
            width += grapheme_width;
            pos = next;
        }
        self.cursor = pos;
    }

    /// Insere uma quebra de linha no estilo do arquivo aberto (CRLF se o
    /// arquivo já usava CRLF, LF por padrão em buffers novos).
    pub fn insert_newline(&mut self) {
        let selection = Selection::point(self.cursor);
        let original = history::State {
            doc: self.rope.clone(),
            selection: selection.clone(),
        };
        let text: Tendril = self.line_ending.as_str().into();
        let transaction = Transaction::insert(&self.rope, &selection, text);
        self.apply(transaction, original);
        self.cursor = graphemes::next_grapheme_boundary(self.rope.slice(..), self.cursor);
    }

    pub fn insert_char(&mut self, ch: char) {
        let selection = Selection::point(self.cursor);
        let original = history::State {
            doc: self.rope.clone(),
            selection: selection.clone(),
        };
        let mut text = Tendril::new();
        text.push(ch);
        let transaction = Transaction::insert(&self.rope, &selection, text);
        self.apply(transaction, original);
        self.cursor = graphemes::next_grapheme_boundary(self.rope.slice(..), self.cursor);
    }

    pub fn delete_backward(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let from = graphemes::prev_grapheme_boundary(self.rope.slice(..), self.cursor);
        let to = self.cursor;
        self.delete_range(from, to);
        self.cursor = from;
    }

    pub fn delete_forward(&mut self) {
        let to = graphemes::next_grapheme_boundary(self.rope.slice(..), self.cursor);
        if to == self.cursor {
            return;
        }
        self.delete_range(self.cursor, to);
    }

    fn delete_range(&mut self, from: usize, to: usize) {
        let original = history::State {
            doc: self.rope.clone(),
            selection: Selection::point(self.cursor),
        };
        let transaction = Transaction::delete(&self.rope, [(from, to)].into_iter());
        self.apply(transaction, original);
    }

    fn apply(&mut self, transaction: Transaction, original: history::State) {
        transaction.apply(&mut self.rope);
        self.history.commit_revision(&transaction, &original);
        self.modified = true;
        // Qualquer edição invalida uma seleção que estivesse ativa — sem
        // isso, o destaque continuaria crescendo junto com o texto digitado.
        self.selection_anchor = None;
    }

    /// Desfaz a última edição. Retorna `false` se não havia nada a desfazer.
    pub fn undo(&mut self) -> bool {
        let Some(inversion) = self.history.undo() else {
            return false;
        };
        inversion.apply(&mut self.rope);
        self.cursor = self.cursor.min(self.rope.len_chars());
        self.modified = true;
        self.selection_anchor = None;
        true
    }
}

/// Escreve `rope` em `path` atomicamente: cria um arquivo temporário na
/// mesma pasta do arquivo *real* (seguindo a cadeia de symlinks, se
/// `path` for um link — para não substituir o link por um arquivo
/// comum, o que quebraria ferramentas como o GNU stow), copia as
/// permissões do arquivo original se ele existir, sincroniza o conteúdo
/// em disco (fsync) e só então substitui o arquivo real por rename. Se
/// o arquivo real ainda não existir, usa 0o666 como os demais editores
/// (ex.: `File::create`) — o kernel aplica o umask do processo por
/// cima, então o resultado típico é 0o644.
fn write_atomic(path: &Path, rope: &Rope) -> io::Result<()> {
    let real_path = resolve_symlink_target(path)?;
    let dir = match real_path.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir,
        _ => Path::new("."),
    };

    let mut builder = tempfile::Builder::new();
    match fs::metadata(&real_path) {
        Ok(metadata) => {
            builder.permissions(metadata.permissions());
        }
        Err(_) => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                builder.permissions(fs::Permissions::from_mode(0o666));
            }
        }
    }
    let tmp = builder.tempfile_in(dir)?;

    rope.write_to(BufWriter::new(tmp.as_file()))?;
    tmp.as_file().sync_all()?;
    tmp.persist(&real_path).map_err(|err| err.error)?;

    // fsync do diretório é uma garantia extra de durabilidade do rename;
    // não é crítico o bastante para falhar o salvamento se não der.
    if let Ok(dir_file) = fs::File::open(dir) {
        let _ = dir_file.sync_all();
    }

    Ok(())
}

/// Se `path` for um symlink, segue a cadeia inteira até o arquivo real
/// (para o rename do salvamento acontecer nele, mantendo o link
/// intacto). Se não for um symlink (ou não existir ainda), devolve o
/// próprio `path`.
fn resolve_symlink_target(path: &Path) -> io::Result<PathBuf> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => fs::canonicalize(path),
        _ => Ok(path.to_path_buf()),
    }
}

/// Conta quantos graphemes existem entre `from` e `to` (em caracteres).
fn grapheme_count(rope: &Rope, from: usize, to: usize) -> usize {
    let slice = rope.slice(..);
    let mut pos = from;
    let mut count = 0;
    while pos < to {
        pos = graphemes::next_grapheme_boundary(slice, pos);
        count += 1;
    }
    count
}

/// Soma a largura visual (em células de terminal) dos graphemes entre
/// `from` e `to` (em caracteres).
fn grapheme_width_between(rope: &Rope, from: usize, to: usize) -> usize {
    let slice = rope.slice(..);
    let mut pos = from;
    let mut width = 0;
    while pos < to {
        let next = graphemes::next_grapheme_boundary(slice, pos);
        let text = Cow::from(rope.slice(pos..next));
        width += graphemes::grapheme_width(&text);
        pos = next;
    }
    width
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc_text(doc: &Document) -> String {
        doc.rope.to_string()
    }

    #[test]
    fn insere_apaga_e_desfaz_com_transaction_e_history() {
        let mut doc = Document::empty();
        for ch in "ola mundo".chars() {
            doc.insert_char(ch);
        }
        assert_eq!(doc_text(&doc), "ola mundo");
        assert!(doc.is_modified());

        // apaga "mundo" (5 caracteres) com backspace
        for _ in 0.."mundo".chars().count() {
            doc.delete_backward();
        }
        assert_eq!(doc_text(&doc), "ola ");

        // desfaz as 5 remoções, uma por revisão
        for _ in 0.."mundo".chars().count() {
            assert!(doc.undo());
        }
        assert_eq!(doc_text(&doc), "ola mundo");

        // desfaz também as 9 inserções, até a raiz do histórico
        for _ in 0.."ola mundo".chars().count() {
            assert!(doc.undo());
        }
        assert_eq!(doc_text(&doc), "");
        assert!(!doc.undo(), "não há mais nada para desfazer na raiz");
    }

    #[test]
    fn delete_forward_remove_o_caractere_a_frente_do_cursor() {
        let mut doc = Document::empty();
        for ch in "abc".chars() {
            doc.insert_char(ch);
        }
        doc.move_left();
        doc.move_left();
        doc.delete_forward(); // remove o "b", que fica entre "a" e "c"
        assert_eq!(doc_text(&doc), "ac");
    }

    #[test]
    fn navega_por_graphemes_com_largura_visual_correta() {
        // "é", "ç", "ã" são 1 caractere e 1 coluna visual; "🙂" e os
        // caracteres CJK ("日", "本", "語") são 1 caractere mas 2 colunas
        // visuais (confirmado com helix_core::graphemes::grapheme_width).
        let mut doc = Document::empty();
        let text = "café 🙂日本語";
        for ch in text.chars() {
            doc.insert_char(ch);
        }
        assert_eq!(doc_text(&doc), text);

        let grapheme_count = text.chars().count();
        for _ in 0..grapheme_count {
            doc.move_left();
        }
        assert_eq!(doc.cursor_position(), Position { line: 0, column: 0 });
        assert_eq!(doc.cursor_visual_column(), 0);

        // "café " = 5 caracteres e 5 colunas visuais
        for _ in 0..5 {
            doc.move_right();
        }
        assert_eq!(doc.cursor_position().column, 5);
        assert_eq!(doc.cursor_visual_column(), 5);

        // "🙂" é 1 caractere, mas ocupa 2 colunas visuais
        doc.move_right();
        assert_eq!(doc.cursor_position().column, 6);
        assert_eq!(doc.cursor_visual_column(), 7);

        // "日" também é 1 caractere e ocupa 2 colunas visuais
        doc.move_right();
        assert_eq!(doc.cursor_position().column, 7);
        assert_eq!(doc.cursor_visual_column(), 9);
    }

    #[test]
    fn move_up_e_down_preservam_a_coluna_visual_entre_linhas() {
        let mut doc = Document::empty();
        for ch in "日本語\nab".chars() {
            doc.insert_char(ch);
        }
        // cursor no fim de "ab" (linha 1, coluna visual 2)
        assert_eq!(doc.cursor_position(), Position { line: 1, column: 2 });

        doc.move_up();
        // na linha 0, coluna visual 2 cai no meio de "本" (que ocupa as
        // colunas 2-3); como não cabe inteiro, o cursor fica antes dele
        assert_eq!(doc.cursor_position(), Position { line: 0, column: 1 });
        assert_eq!(doc.cursor_visual_column(), 2);

        doc.move_down();
        assert_eq!(doc.cursor_position(), Position { line: 1, column: 2 });
    }

    #[test]
    fn move_to_line_e_buffer_start_end() {
        let mut doc = Document::empty();
        for ch in "um\ndois\ntres".chars() {
            doc.insert_char(ch);
        }
        // cursor no fim de "tres" (linha 2)
        doc.move_to_line_start();
        assert_eq!(doc.cursor_position(), Position { line: 2, column: 0 });

        doc.move_to_line_end();
        assert_eq!(doc.cursor_position(), Position { line: 2, column: 4 });

        doc.move_to_buffer_start();
        assert_eq!(doc.cursor_position(), Position { line: 0, column: 0 });

        doc.move_to_buffer_end();
        assert_eq!(doc.cursor_position(), Position { line: 2, column: 4 });
    }

    #[test]
    fn cursor_offset_e_set_cursor() {
        let mut doc = Document::empty();
        for ch in "abc".chars() {
            doc.insert_char(ch);
        }
        assert_eq!(doc.cursor_offset(), 3);
        doc.set_cursor(1);
        assert_eq!(doc.cursor_offset(), 1);
        doc.set_cursor(100); // além do fim: fica no fim
        assert_eq!(doc.cursor_offset(), 3);
    }

    #[test]
    fn find_from_acha_a_frente_do_cursor() {
        let mut doc = Document::empty();
        for ch in "ola mundo ola de novo".chars() {
            doc.insert_char(ch);
        }
        // a primeira ocorrência de "ola" a partir de 4 (depois do primeiro
        // "ola ") é a segunda ocorrência, em "ola de novo"
        assert_eq!(doc.find_from(4, "ola"), Some(10));
    }

    #[test]
    fn find_from_da_a_volta_pro_inicio_se_nao_achar_antes_do_fim() {
        let mut doc = Document::empty();
        for ch in "ola mundo".chars() {
            doc.insert_char(ch);
        }
        // só há uma ocorrência de "ola", antes de onde a busca começa —
        // dá a volta e encontra no início.
        assert_eq!(doc.find_from(5, "ola"), Some(0));
    }

    #[test]
    fn find_from_nao_acha_o_que_nao_existe_ou_consulta_vazia() {
        let mut doc = Document::empty();
        for ch in "ola mundo".chars() {
            doc.insert_char(ch);
        }
        assert_eq!(doc.find_from(0, "xyz"), None);
        assert_eq!(doc.find_from(0, ""), None);
    }

    #[test]
    fn selecao_estende_com_o_movimento_do_cursor() {
        let mut doc = Document::empty();
        for ch in "ola mundo".chars() {
            doc.insert_char(ch);
        }
        doc.move_to_buffer_start();
        doc.start_selection();
        assert_eq!(doc.selection_range(), Some((0, 0)));

        for _ in 0..3 {
            doc.move_right();
        }
        assert_eq!(doc.selection_range(), Some((0, 3)));
        assert_eq!(doc.selected_text(), Some("ola".to_string()));
    }

    #[test]
    fn selecao_funciona_com_a_ancora_depois_do_cursor_tambem() {
        let mut doc = Document::empty();
        for ch in "ola mundo".chars() {
            doc.insert_char(ch);
        }
        // âncora em 9 (fim), cursor volta pro início: intervalo (0, 9)
        doc.start_selection();
        doc.move_to_buffer_start();
        assert_eq!(doc.selection_range(), Some((0, 9)));
        assert_eq!(doc.selected_text(), Some("ola mundo".to_string()));
    }

    #[test]
    fn select_all_seleciona_o_documento_inteiro() {
        let mut doc = Document::empty();
        for ch in "ola mundo".chars() {
            doc.insert_char(ch);
        }
        doc.move_to_buffer_start();
        doc.select_all();
        assert_eq!(doc.selection_range(), Some((0, 9)));
    }

    #[test]
    fn editar_descarta_a_selecao_ativa() {
        let mut doc = Document::empty();
        for ch in "ola mundo".chars() {
            doc.insert_char(ch);
        }
        doc.move_to_buffer_start();
        doc.start_selection();
        doc.move_right();
        assert!(doc.selection_range().is_some());

        doc.insert_char('X');
        assert_eq!(
            doc.selection_range(),
            None,
            "editar deveria descartar a seleção, senão o destaque cresceria junto com o texto"
        );
    }

    #[test]
    fn delete_selection_apaga_o_trecho_e_descarta_a_selecao() {
        let mut doc = Document::empty();
        for ch in "ola mundo".chars() {
            doc.insert_char(ch);
        }
        doc.move_to_buffer_start();
        doc.start_selection();
        for _ in 0..4 {
            doc.move_right(); // seleciona "ola "
        }
        doc.delete_selection();

        assert_eq!(doc_text(&doc), "mundo");
        assert_eq!(doc.selection_range(), None);
        assert_eq!(doc.cursor_offset(), 0);
    }

    #[test]
    fn delete_selection_sem_selecao_nao_faz_nada() {
        let mut doc = Document::empty();
        for ch in "ola".chars() {
            doc.insert_char(ch);
        }
        doc.delete_selection();
        assert_eq!(doc_text(&doc), "ola");
    }

    #[test]
    fn goto_line_vai_para_o_inicio_da_linha_1_indexada() {
        let mut doc = Document::empty();
        for ch in "um\ndois\ntres".chars() {
            doc.insert_char(ch);
        }
        doc.goto_line(2);
        assert_eq!(doc.cursor_position(), Position { line: 1, column: 0 });

        doc.goto_line(100); // além do fim: fica na última linha
        assert_eq!(doc.cursor_position(), Position { line: 2, column: 0 });

        doc.goto_line(0); // abaixo do início: fica na primeira
        assert_eq!(doc.cursor_position(), Position { line: 0, column: 0 });
    }

    #[test]
    fn move_word_forward_e_backward() {
        let mut doc = Document::empty();
        for ch in "ola mundo, tudo bem?".chars() {
            doc.insert_char(ch);
        }
        doc.move_to_buffer_start();

        doc.move_word_forward();
        assert_eq!(doc.cursor_offset(), 3); // depois de "ola"
        doc.move_word_forward();
        assert_eq!(doc.cursor_offset(), 9); // depois de "mundo", antes da vírgula

        doc.move_word_backward();
        assert_eq!(doc.cursor_offset(), 4); // início de "mundo"
    }

    #[test]
    fn move_paragraph_forward_e_backward() {
        let mut doc = Document::empty();
        for ch in "um\n\ndois\ntres\n\nquatro".chars() {
            doc.insert_char(ch);
        }
        doc.move_to_buffer_start();

        doc.move_paragraph_forward();
        assert_eq!(doc.cursor_position(), Position { line: 2, column: 0 }); // início de "dois"

        doc.move_paragraph_forward();
        assert_eq!(doc.cursor_position(), Position { line: 5, column: 0 }); // início de "quatro"

        doc.move_paragraph_backward();
        assert_eq!(doc.cursor_position(), Position { line: 2, column: 0 }); // volta pra "dois"
    }

    #[test]
    fn region_for_movimentos_horizontais() {
        let mut doc = Document::empty();
        for ch in "ola mundo, tudo bem?".chars() {
            doc.insert_char(ch);
        }
        doc.set_cursor(4); // início de "mundo"

        assert_eq!(doc.region_for(RegionObject::CharLeft), (3, 4));
        assert_eq!(doc.region_for(RegionObject::CharRight), (4, 5));
        assert_eq!(doc.region_for(RegionObject::LineStart), (0, 4));
        assert_eq!(doc.region_for(RegionObject::LineEnd), (4, 20));
        assert_eq!(doc.region_for(RegionObject::WordBackward), (0, 4));
        assert_eq!(doc.region_for(RegionObject::WordForward), (4, 9));
    }

    #[test]
    fn region_for_movimentos_verticais() {
        let mut doc = Document::empty();
        for ch in "um\ndois\ntres\nquatro".chars() {
            doc.insert_char(ch);
        }
        doc.goto_line(2); // "dois"

        assert_eq!(doc.region_for(RegionObject::LineDown), (3, 13)); // "dois\ntres\n"
        assert_eq!(doc.region_for(RegionObject::LineUp), (0, 8)); // "um\ndois\n"
        assert_eq!(doc.region_for(RegionObject::BufferEnd), (3, 19)); // "dois\ntres\nquatro"
        assert_eq!(doc.region_for(RegionObject::BufferStart), (0, 8)); // "um\ndois\n"
        assert_eq!(doc.region_for(RegionObject::CurrentLine), (3, 8)); // "dois\n"
    }

    #[test]
    fn region_for_paragrafos() {
        let mut doc = Document::empty();
        for ch in "um\n\ndois\ntres\n\nquatro".chars() {
            doc.insert_char(ch);
        }
        doc.move_to_buffer_start();

        // do início de "um" até o início de "dois" (a linha em branco
        // entre eles também entra, inclusive)
        assert_eq!(doc.region_for(RegionObject::ParagraphForward), (0, 9));

        doc.goto_line(4); // "tres"
        // de "tres" (linha 3) de volta até o início de "dois" (linha 2)
        assert_eq!(doc.region_for(RegionObject::ParagraphBackward), (4, 14));
    }

    #[test]
    fn region_for_whole_buffer() {
        let mut doc = Document::empty();
        for ch in "ola mundo".chars() {
            doc.insert_char(ch);
        }
        assert_eq!(doc.region_for(RegionObject::WholeBuffer), (0, 9));
    }

    #[test]
    fn text_in_nao_apaga_nem_move_o_cursor() {
        let mut doc = Document::empty();
        for ch in "ola mundo".chars() {
            doc.insert_char(ch);
        }
        doc.move_to_buffer_start();
        assert_eq!(doc.text_in(0, 3), "ola");
        assert_eq!(doc_text(&doc), "ola mundo");
        assert_eq!(doc.cursor_offset(), 0);
    }

    #[test]
    fn delete_region_apaga_e_devolve_o_texto_com_cursor_no_inicio() {
        let mut doc = Document::empty();
        for ch in "ola mundo".chars() {
            doc.insert_char(ch);
        }
        doc.move_to_buffer_start();
        let removed = doc.delete_region(4, 9);
        assert_eq!(removed, "mundo");
        assert_eq!(doc_text(&doc), "ola ");
        assert_eq!(doc.cursor_offset(), 4);
    }

    #[test]
    fn delete_region_do_arquivo_inteiro() {
        let mut doc = Document::empty();
        for ch in "ola mundo".chars() {
            doc.insert_char(ch);
        }
        let (from, to) = doc.region_for(RegionObject::WholeBuffer);
        let removed = doc.delete_region(from, to);
        assert_eq!(removed, "ola mundo");
        assert_eq!(doc_text(&doc), "");
        assert_eq!(doc.cursor_offset(), 0);
    }

    #[test]
    fn abrir_arquivo_inexistente_cria_buffer_vazio_com_caminho() {
        let dir = tempfile::tempdir().expect("diretório temporário");
        let path = dir.path().join("novo.txt");

        let mut doc = Document::open(&path).expect("abrir arquivo inexistente não deve falhar");
        assert_eq!(doc_text(&doc), "");
        assert!(!doc.is_modified());

        for ch in "oi".chars() {
            doc.insert_char(ch);
        }
        assert!(doc.save().expect("salvar deve funcionar"));
        assert_eq!(fs::read_to_string(&path).unwrap(), "oi");
    }

    #[test]
    #[cfg(unix)]
    fn salvar_e_atomico_e_preserva_permissoes() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("diretório temporário");
        let path = dir.path().join("atomic.txt");
        fs::write(&path, "conteúdo original\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();

        let mut doc = Document::open(&path).expect("abrir deve funcionar");
        doc.insert_char('X');
        assert!(doc.save().expect("salvar deve funcionar"));

        assert_eq!(fs::read_to_string(&path).unwrap(), "Xconteúdo original\n");
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(
            mode, 0o600,
            "permissões do arquivo original devem ser preservadas"
        );
    }

    #[test]
    #[cfg(unix)]
    fn salvar_atraves_de_symlink_preserva_o_link() {
        let dir = tempfile::tempdir().expect("diretório temporário");
        let real = dir.path().join("symlink_real.txt");
        let link = dir.path().join("symlink_link.txt");
        fs::write(&real, "conteúdo original\n").unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();

        let mut doc = Document::open(&link).expect("abrir pelo link deve funcionar");
        doc.insert_char('X');
        assert!(doc.save().expect("salvar deve funcionar"));

        let link_metadata = fs::symlink_metadata(&link).unwrap();
        assert!(
            link_metadata.file_type().is_symlink(),
            "o link não deveria ter sido substituído por um arquivo comum"
        );
        assert_eq!(fs::read_link(&link).unwrap(), real);
        assert_eq!(
            fs::read_to_string(&real).unwrap(),
            "Xconteúdo original\n",
            "o conteúdo deve ter mudado no arquivo real"
        );
    }

    #[test]
    #[cfg(unix)]
    fn salvar_arquivo_novo_aplica_o_mesmo_umask_que_file_create() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("diretório temporário");

        // Em vez de fixar um umask (processo inteiro, não por thread — e
        // os testes correm em paralelo), usamos File::create como
        // referência do que o umask atual do processo realmente produz.
        let reference_path = dir.path().join("umask_referencia.txt");
        fs::File::create(&reference_path).unwrap();
        let expected_mode = fs::metadata(&reference_path).unwrap().permissions().mode() & 0o777;
        let _ = fs::remove_file(&reference_path);

        let path = dir.path().join("umask_novo.txt");
        let mut doc = Document::open(&path).expect("abrir arquivo inexistente não deve falhar");
        doc.insert_char('X');
        assert!(doc.save().expect("salvar deve funcionar"));

        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(
            mode, expected_mode,
            "deveria aplicar o mesmo umask do processo que File::create aplica"
        );
    }

    #[test]
    fn preserva_crlf_e_usa_crlf_em_linhas_novas() {
        let dir = tempfile::tempdir().expect("diretório temporário");
        let path = dir.path().join("crlf.txt");
        fs::write(&path, "linha um\r\nlinha dois\r\n").unwrap();

        let mut doc = Document::open(&path).expect("abrir deve funcionar");
        doc.insert_newline();
        doc.insert_char('X');
        assert!(doc.save().expect("salvar deve funcionar"));

        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "\r\nXlinha um\r\nlinha dois\r\n"
        );
    }

    #[test]
    fn preserva_ausencia_de_quebra_de_linha_final() {
        let dir = tempfile::tempdir().expect("diretório temporário");
        let path = dir.path().join("sem_quebra.txt");
        fs::write(&path, "sem quebra no final").unwrap();

        let mut doc = Document::open(&path).expect("abrir deve funcionar");
        doc.insert_char('X');
        assert!(doc.save().expect("salvar deve funcionar"));

        let saved = fs::read_to_string(&path).unwrap();
        assert_eq!(saved, "Xsem quebra no final");
        assert!(!saved.ends_with('\n'));
    }

    #[test]
    fn recusa_arquivo_que_nao_e_utf8_valido() {
        let dir = tempfile::tempdir().expect("diretório temporário");
        let path = dir.path().join("binario.bin");
        fs::write(&path, [0xFF, 0xFE, 0x00, 0x01]).unwrap();

        match Document::open(&path) {
            Err(OpenError::NotUtf8) => {}
            Ok(_) => panic!("esperava recusa por UTF-8 inválido, mas o arquivo abriu"),
            Err(other) => panic!("esperava OpenError::NotUtf8, obteve outro erro: {other}"),
        }
    }
}
