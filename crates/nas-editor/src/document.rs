//! Documento do editor: envolve `helix-core` (rope, transações, histórico).
//!
//! Este é o único módulo do crate que importa `helix_core` diretamente — ver
//! docs/decisions/0001-helix-como-biblioteca.md. O resto do nas (nas-term,
//! nas-keys) só vê os tipos definidos aqui (`Document`, `Position`), nunca
//! `helix_core::Rope`, `Transaction` ou `History`.

use std::borrow::Cow;
use std::fs;
use std::io::{self, BufReader, BufWriter};
use std::path::{Path, PathBuf};

use helix_core::{Rope, Selection, Tendril, Transaction, graphemes, history, line_ending};

/// Posição lógica do cursor no documento: linha e coluna em caracteres,
/// ambas 0-indexadas.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Position {
    pub line: usize,
    pub column: usize,
}

/// Um buffer de texto editável, com desfazer.
pub struct Document {
    rope: Rope,
    cursor: usize,
    history: history::History,
    path: Option<PathBuf>,
    modified: bool,
}

impl Document {
    /// Um buffer vazio, sem arquivo associado.
    pub fn empty() -> Self {
        Self {
            rope: Rope::new(),
            cursor: 0,
            history: history::History::default(),
            path: None,
            modified: false,
        }
    }

    /// Abre o arquivo em `path`. Se o arquivo não existir ainda, retorna um
    /// buffer vazio com o caminho associado (para criar o arquivo ao salvar).
    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref();
        let rope = match fs::File::open(path) {
            Ok(file) => Rope::from_reader(BufReader::new(file))?,
            Err(err) if err.kind() == io::ErrorKind::NotFound => Rope::new(),
            Err(err) => return Err(err),
        };
        tracing::info!(path = %path.display(), "documento aberto");
        Ok(Self {
            rope,
            cursor: 0,
            history: history::History::default(),
            path: Some(path.to_path_buf()),
            modified: false,
        })
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub fn is_modified(&self) -> bool {
        self.modified
    }

    /// Escreve o buffer no caminho associado. Retorna `false` se o
    /// documento não tiver um caminho (ainda não há "salvar como").
    pub fn save(&mut self) -> io::Result<bool> {
        let Some(path) = &self.path else {
            return Ok(false);
        };
        self.rope
            .write_to(BufWriter::new(fs::File::create(path)?))?;
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
    }

    /// Desfaz a última edição. Retorna `false` se não havia nada a desfazer.
    pub fn undo(&mut self) -> bool {
        let Some(inversion) = self.history.undo() else {
            return false;
        };
        inversion.apply(&mut self.rope);
        self.cursor = self.cursor.min(self.rope.len_chars());
        self.modified = true;
        true
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
    fn abrir_arquivo_inexistente_cria_buffer_vazio_com_caminho() {
        let dir = std::env::temp_dir().join(format!("nas-editor-test-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("novo.txt");
        let _ = fs::remove_file(&path);

        let mut doc = Document::open(&path).expect("abrir arquivo inexistente não deve falhar");
        assert_eq!(doc_text(&doc), "");
        assert!(!doc.is_modified());

        for ch in "oi".chars() {
            doc.insert_char(ch);
        }
        assert!(doc.save().expect("salvar deve funcionar"));
        assert_eq!(fs::read_to_string(&path).unwrap(), "oi");

        let _ = fs::remove_file(&path);
        let _ = fs::remove_dir(&dir);
    }
}
