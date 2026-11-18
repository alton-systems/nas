//! O journal: uma entrada por transação (depois de agrupar digitação
//! contínua), gravada em lotes — docs/decisions/0005-historico-com-autoria.md
//! §2 e §3.

use std::fs::{self, File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::author::Author;

/// Duas transações consecutivas do mesmo autor/comando/arquivo se
/// juntam numa entrada só se a diferença entre o fim de uma e o
/// início da outra for menor que isto — docs/decisions/0005 §2.
const GROUPING_WINDOW_SECONDS: u64 = 2;

/// A fila de entradas fechadas é escrita no disco quando atinge este
/// tamanho, ou quando `BATCH_INTERVAL_SECONDS` se passam desde o
/// último flush — o que vier primeiro.
const BATCH_SIZE: usize = 50;
const BATCH_INTERVAL_SECONDS: u64 = 1;

/// Uma entrada do journal: o suficiente pra reconstruir o texto, não
/// só metadados — docs/decisions/0005 §3. `removed` é o texto que
/// estava em `[from, to)` antes; `inserted` é o texto novo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalEntry {
    pub ts_start: u64,
    pub ts_end: u64,
    pub author: Author,
    pub command: String,
    pub file: String,
    pub from: usize,
    pub to: usize,
    pub removed: String,
    pub inserted: String,
}

/// Agora, em segundos desde a época Unix — pra quem chama `record`
/// fora de teste (nas-term, a partir da etapa de integração). Testes
/// não precisam disto: eles controlam o tempo passando valores
/// explícitos.
pub fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// O journal de um projeto: uma entrada "em aberto" (esperando se
/// uma próxima transação é contígua a ela) e uma fila de entradas já
/// fechadas, esperando ser escritas em lote. Não guarda nenhum
/// relógio interno — todo método que precisa saber "agora" recebe
/// isso explicitamente, o que torna o agrupamento e os lotes
/// determinísticos de testar, sem precisar esperar de verdade.
pub struct Journal {
    writer: BufWriter<File>,
    open: Option<JournalEntry>,
    queue: Vec<JournalEntry>,
    last_flush_at: u64,
}

impl Journal {
    /// Abre (criando se preciso) `<history_dir>/journal.jsonl` pra
    /// acrescentar — nunca sobrescreve o que já existe.
    pub fn open(history_dir: &Path) -> io::Result<Self> {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(history_dir.join("journal.jsonl"))?;
        Ok(Self {
            writer: BufWriter::new(file),
            open: None,
            queue: Vec::new(),
            last_flush_at: 0,
        })
    }

    /// Registra uma transação em `now`. Se for contígua com a
    /// entrada aberta (mesmo autor/comando/arquivo, dentro da janela
    /// de tempo, e o intervalo encosta no da entrada aberta — pra
    /// frente ou pra trás), estende a entrada aberta em vez de criar
    /// uma nova. Senão, fecha a aberta (pra fila) e abre esta como a
    /// nova aberta. Pode disparar um flush em lote, se os limites
    /// (§2) forem atingidos.
    #[allow(clippy::too_many_arguments)]
    pub fn record(
        &mut self,
        now: u64,
        author: Author,
        command: impl Into<String>,
        file: impl Into<String>,
        from: usize,
        to: usize,
        removed: impl Into<String>,
        inserted: impl Into<String>,
    ) -> io::Result<()> {
        let command = command.into();
        let file = file.into();
        let removed = removed.into();
        let inserted = inserted.into();

        if let Some(open) = &mut self.open {
            let same_origin = open.author == author && open.command == command && open.file == file;
            let within_window = now.saturating_sub(open.ts_end) < GROUPING_WINDOW_SECONDS;
            if same_origin && within_window {
                let open_end = open.from + open.inserted.chars().count();
                if from == open_end {
                    // contígua pra frente: estende ao final.
                    open.to += to - from;
                    open.removed.push_str(&removed);
                    open.inserted.push_str(&inserted);
                    open.ts_end = now;
                    return self.maybe_flush(now);
                }
                if to == open.from {
                    // contígua pra trás (ex.: apagar com Backspace repetido).
                    open.from = from;
                    open.removed = format!("{removed}{}", open.removed);
                    open.inserted = format!("{inserted}{}", open.inserted);
                    open.ts_end = now;
                    return self.maybe_flush(now);
                }
            }
        }

        self.close_open();
        self.open = Some(JournalEntry {
            ts_start: now,
            ts_end: now,
            author,
            command,
            file,
            from,
            to,
            removed,
            inserted,
        });
        self.maybe_flush(now)
    }

    fn close_open(&mut self) {
        if let Some(entry) = self.open.take() {
            self.queue.push(entry);
        }
    }

    fn maybe_flush(&mut self, now: u64) -> io::Result<()> {
        if self.queue.len() >= BATCH_SIZE
            || now.saturating_sub(self.last_flush_at) >= BATCH_INTERVAL_SECONDS
        {
            self.flush_queue(now)?;
        }
        Ok(())
    }

    fn flush_queue(&mut self, now: u64) -> io::Result<()> {
        for entry in self.queue.drain(..) {
            let line = serde_json::to_string(&entry)
                .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
            writeln!(self.writer, "{line}")?;
        }
        self.writer.flush()?;
        self.last_flush_at = now;
        Ok(())
    }

    /// Força o flush imediato: fecha a entrada aberta (se houver) e
    /// escreve toda a fila no disco. Fechar o nas, fechar um buffer
    /// ou criar um checkpoint precisam disto — docs/decisions/0005 §2.
    pub fn flush(&mut self, now: u64) -> io::Result<()> {
        self.close_open();
        self.flush_queue(now)
    }
}

/// Lê todas as entradas de um `journal.jsonl`. Tolerante a uma última
/// linha truncada (um encerramento abrupto no meio de um append): se
/// só a última linha falhar ao interpretar, ela é ignorada, como se
/// nunca tivesse sido escrita — docs/decisions/0005 §5. Uma falha em
/// qualquer linha anterior é um erro de verdade (não devia acontecer:
/// todo append anterior já tinha sido concluído por inteiro). Um
/// journal inexistente (projeto nunca teve uma transação) devolve uma
/// lista vazia, não um erro.
pub fn read_entries(path: &Path) -> io::Result<Vec<JournalEntry>> {
    let content = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err),
    };

    let lines: Vec<&str> = content.lines().filter(|line| !line.is_empty()).collect();
    let mut entries = Vec::with_capacity(lines.len());
    let last_index = lines.len().saturating_sub(1);

    for (index, line) in lines.iter().enumerate() {
        match serde_json::from_str::<JournalEntry>(line) {
            Ok(entry) => entries.push(entry),
            Err(_) if index == last_index => break,
            Err(err) => return Err(io::Error::new(io::ErrorKind::InvalidData, err)),
        }
    }

    Ok(entries)
}

/// O caminho do `journal.jsonl` dentro da pasta de histórico de um
/// projeto — conveniência pra quem só tem o `history_dir` e precisa
/// chamar [`read_entries`] sem ter um [`Journal`] aberto.
pub fn journal_path(history_dir: &Path) -> PathBuf {
    history_dir.join("journal.jsonl")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user_entry(
        journal: &mut Journal,
        now: u64,
        command: &str,
        from: usize,
        to: usize,
        removed: &str,
        inserted: &str,
    ) {
        journal
            .record(
                now,
                Author::User,
                command,
                "arquivo.txt",
                from,
                to,
                removed,
                inserted,
            )
            .expect("record não deveria falhar");
    }

    #[test]
    fn digitacao_continua_agrupa_numa_entrada_so() {
        let dir = tempfile::tempdir().unwrap();
        let mut journal = Journal::open(dir.path()).unwrap();

        // digita "ab" caractere a caractere, em posições contíguas.
        user_entry(&mut journal, 100, "insert", 10, 10, "", "a");
        user_entry(&mut journal, 100, "insert", 11, 11, "", "b");
        journal.flush(100).unwrap();

        let entries = read_entries(&journal_path(dir.path())).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].from, 10);
        assert_eq!(entries[0].inserted, "ab");
    }

    #[test]
    fn apagar_para_tras_agrupa_na_ordem_certa() {
        let dir = tempfile::tempdir().unwrap();
        let mut journal = Journal::open(dir.path()).unwrap();

        // cursor em 10; backspace apaga [9,10)="x", depois [8,9)="y".
        user_entry(&mut journal, 100, "delete", 9, 10, "x", "");
        user_entry(&mut journal, 100, "delete", 8, 9, "y", "");
        journal.flush(100).unwrap();

        let entries = read_entries(&journal_path(dir.path())).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].from, 8);
        assert_eq!(entries[0].to, 10);
        assert_eq!(entries[0].removed, "yx");
    }

    #[test]
    fn salto_de_posicao_nao_agrupa() {
        let dir = tempfile::tempdir().unwrap();
        let mut journal = Journal::open(dir.path()).unwrap();

        user_entry(&mut journal, 100, "insert", 10, 10, "", "a");
        user_entry(&mut journal, 100, "insert", 50, 50, "", "b"); // não é contígua
        journal.flush(100).unwrap();

        let entries = read_entries(&journal_path(dir.path())).unwrap();
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn comando_diferente_nao_agrupa() {
        let dir = tempfile::tempdir().unwrap();
        let mut journal = Journal::open(dir.path()).unwrap();

        user_entry(&mut journal, 100, "insert", 10, 10, "", "a");
        user_entry(&mut journal, 100, "delete", 11, 12, "x", ""); // comando diferente
        journal.flush(100).unwrap();

        let entries = read_entries(&journal_path(dir.path())).unwrap();
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn autor_diferente_nao_agrupa() {
        let dir = tempfile::tempdir().unwrap();
        let mut journal = Journal::open(dir.path()).unwrap();

        user_entry(&mut journal, 100, "insert", 10, 10, "", "a");
        journal
            .record(
                100,
                Author::Ai {
                    agent: "teste".to_string(),
                    session: "s1".to_string(),
                },
                "insert",
                "arquivo.txt",
                11,
                11,
                "",
                "b",
            )
            .unwrap();
        journal.flush(100).unwrap();

        let entries = read_entries(&journal_path(dir.path())).unwrap();
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn arquivo_diferente_nao_agrupa() {
        let dir = tempfile::tempdir().unwrap();
        let mut journal = Journal::open(dir.path()).unwrap();

        user_entry(&mut journal, 100, "insert", 10, 10, "", "a");
        journal
            .record(100, Author::User, "insert", "outro.txt", 11, 11, "", "b")
            .unwrap();
        journal.flush(100).unwrap();

        let entries = read_entries(&journal_path(dir.path())).unwrap();
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn pausa_longa_nao_agrupa() {
        let dir = tempfile::tempdir().unwrap();
        let mut journal = Journal::open(dir.path()).unwrap();

        user_entry(&mut journal, 100, "insert", 10, 10, "", "a");
        user_entry(&mut journal, 103, "insert", 11, 11, "", "b"); // 3s depois, >= janela de 2s
        journal.flush(103).unwrap();

        let entries = read_entries(&journal_path(dir.path())).unwrap();
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn pausa_curta_ainda_agrupa() {
        let dir = tempfile::tempdir().unwrap();
        let mut journal = Journal::open(dir.path()).unwrap();

        user_entry(&mut journal, 100, "insert", 10, 10, "", "a");
        user_entry(&mut journal, 101, "insert", 11, 11, "", "b"); // 1s depois, < janela de 2s
        journal.flush(101).unwrap();

        let entries = read_entries(&journal_path(dir.path())).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].inserted, "ab");
    }

    #[test]
    fn undo_e_journalado_como_qualquer_outro_comando() {
        let dir = tempfile::tempdir().unwrap();
        let mut journal = Journal::open(dir.path()).unwrap();

        user_entry(&mut journal, 100, "insert", 10, 10, "", "a");
        user_entry(&mut journal, 100, "undo", 10, 11, "a", ""); // desfaz a inserção
        journal.flush(100).unwrap();

        let entries = read_entries(&journal_path(dir.path())).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[1].command, "undo");
        assert_eq!(entries[1].author, Author::User);
    }

    #[test]
    fn flush_por_tamanho_nao_espera_o_tempo() {
        let dir = tempfile::tempdir().unwrap();
        let mut journal = Journal::open(dir.path()).unwrap();

        // BATCH_SIZE + 1 transações sem contiguidade (posições bem
        // separadas, cada uma seu próprio comando), todas no mesmo
        // "now" (sem o flush por tempo ter nenhuma chance de
        // disparar). Cada chamada fecha a anterior na fila antes de
        // abrir a nova — a fila só chega a BATCH_SIZE depois da
        // (BATCH_SIZE + 1)-ésima chamada, e é nesse momento, dentro
        // do próprio record(), que o flush por tamanho dispara.
        for i in 0..=BATCH_SIZE {
            journal
                .record(
                    100,
                    Author::User,
                    format!("cmd-{i}"),
                    "arquivo.txt",
                    i * 1000,
                    i * 1000,
                    "",
                    "x",
                )
                .unwrap();
        }

        // o flush por tamanho já deveria ter escrito BATCH_SIZE
        // entradas, antes de qualquer flush explícito — a última
        // (BATCH_SIZE + 1) continua só "aberta", ainda não escrita.
        let entries_before_final_flush = read_entries(&journal_path(dir.path())).unwrap();
        assert_eq!(
            entries_before_final_flush.len(),
            BATCH_SIZE,
            "o flush por tamanho deveria ter disparado sozinho, sem esperar o flush final"
        );

        journal.flush(100).unwrap();
        let entries = read_entries(&journal_path(dir.path())).unwrap();
        assert_eq!(entries.len(), BATCH_SIZE + 1);
    }

    #[test]
    fn flush_por_tempo_dispara_mesmo_com_fila_pequena() {
        let dir = tempfile::tempdir().unwrap();
        let mut journal = Journal::open(dir.path()).unwrap();

        user_entry(&mut journal, 100, "insert", 10, 10, "", "a");
        user_entry(&mut journal, 200, "insert", 500, 500, "", "b"); // não contígua, >1s depois

        // a primeira entrada (fechada ao abrir a segunda) deveria já
        // ter sido escrita, porque passou muito mais que 1 segundo.
        let entries = read_entries(&journal_path(dir.path())).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].inserted, "a");
    }

    #[test]
    fn journal_inexistente_le_como_lista_vazia() {
        let dir = tempfile::tempdir().unwrap();
        let entries = read_entries(&journal_path(dir.path())).unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn ultima_linha_truncada_e_ignorada_sem_invalidar_o_resto() {
        let dir = tempfile::tempdir().unwrap();
        let path = journal_path(dir.path());

        let mut journal = Journal::open(dir.path()).unwrap();
        user_entry(&mut journal, 100, "insert", 10, 10, "", "a");
        user_entry(&mut journal, 200, "insert", 500, 500, "", "b");
        journal.flush(200).unwrap();
        drop(journal);

        // simula um encerramento abrupto: acrescenta uma linha JSON
        // incompleta, como se o processo tivesse caído no meio do
        // append.
        let mut raw = fs::read_to_string(&path).unwrap();
        raw.push_str(r#"{"ts_start":300,"ts_end":300,"author":{"kind":"use"#);
        fs::write(&path, raw).unwrap();

        let entries = read_entries(&path).unwrap();
        assert_eq!(
            entries.len(),
            2,
            "as duas entradas completas devem continuar lendo certo"
        );
    }

    #[test]
    fn linha_corrompida_no_meio_e_erro_de_verdade() {
        let dir = tempfile::tempdir().unwrap();
        let path = journal_path(dir.path());

        let mut journal = Journal::open(dir.path()).unwrap();
        user_entry(&mut journal, 100, "insert", 10, 10, "", "a");
        user_entry(&mut journal, 200, "insert", 500, 500, "", "b");
        user_entry(&mut journal, 300, "insert", 900, 900, "", "c");
        journal.flush(300).unwrap();
        drop(journal);

        let content = fs::read_to_string(&path).unwrap();
        let mut lines: Vec<&str> = content.lines().collect();
        lines[1] = "isto não é json válido"; // corrompe a linha do meio
        fs::write(&path, lines.join("\n") + "\n").unwrap();

        let result = read_entries(&path);
        assert!(
            result.is_err(),
            "uma linha corrompida que não é a última deveria ser um erro"
        );
    }
}
