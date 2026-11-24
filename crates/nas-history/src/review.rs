//! Modo de revisão: marcas de trecho (aceitar/rejeitar) e o registro
//! de sessão de revisão — docs/decisions/0008-modo-de-revisao.md §5.
//!
//! Dois arquivos, lado a lado com `journal.jsonl`/`permissions_log.jsonl`
//! em `history_dir`: `review_marks.jsonl` (uma linha por decisão sobre
//! um trecho) e `reviews_log.jsonl` (uma linha por sessão de revisão,
//! início/fim/contagens — a métrica de "controle" do §7 da ADR 0006).
//! Mesmo padrão append-only, sem poda automática, que `permissions_log.jsonl`
//! já usa (ADR 0008 §5: "nunca podada automaticamente — é pequena, só
//! texto").

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::author::Author;

/// O que foi decidido sobre um trecho — ADR 0008 §3.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReviewDecision {
    Accepted,
    Rejected,
}

/// Uma linha de `review_marks.jsonl`: quem revisou, contra qual
/// checkpoint ("antes"), qual arquivo, qual trecho (por `hunk_hash`,
/// ver [`hunk_hash`]) e a decisão — ADR 0008 §5.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReviewMark {
    pub ts: u64,
    /// A mesma serialização de [`Author`] que o journal e os
    /// checkpoints já usam — nunca um texto livre (ADR 0008 §5,
    /// revisão do PR).
    pub reviewer: Author,
    pub from_checkpoint: String,
    pub file: String,
    pub hunk_hash: String,
    pub decision: ReviewDecision,
}

/// Uma linha de `reviews_log.jsonl`: uma sessão de revisão inteira —
/// início, fim (quando ela terminou, de um jeito ou de outro) e as
/// contagens. `accepted + rejected` pode ser menor que `total_hunks`
/// se a revisão foi interrompida antes de terminar — não é um erro
/// (ADR 0008 §5).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReviewLogEntry {
    pub started_at: u64,
    pub ended_at: u64,
    pub from_checkpoint: String,
    pub total_hunks: usize,
    pub accepted: usize,
    pub rejected: usize,
}

/// O caminho de `review_marks.jsonl` dentro de `history_dir`.
pub fn review_marks_path(history_dir: &Path) -> PathBuf {
    history_dir.join("review_marks.jsonl")
}

/// O caminho de `reviews_log.jsonl` dentro de `history_dir`.
pub fn reviews_log_path(history_dir: &Path) -> PathBuf {
    history_dir.join("reviews_log.jsonl")
}

/// Acrescenta uma marca de trecho — mesma trava que todo escritor do
/// histórico já usa (`crate::lock::HistoryLock`), pelo mesmo motivo
/// de `permissions::record_decision`: mais de um processo pode
/// escrever no mesmo projeto ao mesmo tempo.
pub fn record_mark(history_dir: &Path, mark: &ReviewMark) -> io::Result<()> {
    let _lock = crate::lock::HistoryLock::acquire(history_dir)?;
    let line = serde_json::to_string(mark)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(review_marks_path(history_dir))?;
    writeln!(file, "{line}")
}

/// Lê todas as marcas já registradas — vazio se a revisão nunca foi
/// usada neste projeto.
pub fn read_marks(history_dir: &Path) -> io::Result<Vec<ReviewMark>> {
    let content = match fs::read_to_string(review_marks_path(history_dir)) {
        Ok(content) => content,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err),
    };
    content
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| {
            serde_json::from_str(line)
                .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
        })
        .collect()
}

/// As marcas já registradas para uma revisão específica (um
/// `from_checkpoint`) — o que `Ctrl+t v`/`Ctrl+t V` cruzam contra os
/// trechos recalculados ao reabrir (ADR 0008 §5).
pub fn marks_for_checkpoint(
    history_dir: &Path,
    checkpoint_id: &str,
) -> io::Result<Vec<ReviewMark>> {
    Ok(read_marks(history_dir)?
        .into_iter()
        .filter(|mark| mark.from_checkpoint == checkpoint_id)
        .collect())
}

/// Acrescenta um registro de sessão de revisão.
pub fn record_log_entry(history_dir: &Path, entry: &ReviewLogEntry) -> io::Result<()> {
    let _lock = crate::lock::HistoryLock::acquire(history_dir)?;
    let line = serde_json::to_string(entry)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(reviews_log_path(history_dir))?;
    writeln!(file, "{line}")
}

/// Lê todo o registro de sessões de revisão — vazio se o modo de
/// revisão nunca foi usado neste projeto.
pub fn read_log_entries(history_dir: &Path) -> io::Result<Vec<ReviewLogEntry>> {
    let content = match fs::read_to_string(reviews_log_path(history_dir)) {
        Ok(content) => content,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err),
    };
    content
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| {
            serde_json::from_str(line)
                .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
        })
        .collect()
}

/// A identidade de um trecho, pra marcar e reconhecer de novo ao
/// reabrir a mesma revisão — ADR 0008 §5 (revisão do PR): hash de
/// **cinco** campos, não só o texto "antes". Duas inserções puras
/// (lado "antes" vazio nos dois) em lugares diferentes do mesmo
/// arquivo têm textos "depois" e linhas de início diferentes, então
/// não colidem — só o texto "antes" sozinho colidiria.
///
/// Cada campo entra no hash com o próprio tamanho na frente (um
/// `update` do tamanho em bytes little-endian, depois o conteúdo) —
/// evita qualquer ambiguidade de um delimitador que apareça dentro do
/// próprio texto "antes"/"depois".
pub fn hunk_hash(
    path: &str,
    checkpoint_id: &str,
    before: &str,
    after: &str,
    before_start_line: usize,
) -> String {
    let mut hasher = Sha256::new();
    for field in [path, checkpoint_id, before, after] {
        hasher.update((field.len() as u64).to_le_bytes());
        hasher.update(field.as_bytes());
    }
    hasher.update(before_start_line.to_le_bytes());
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// A autoria de um trecho — ADR 0008 §2. `None` quando nenhuma
/// entrada do journal cobre a faixa pedida (histórico podado, ou
/// nunca houve uma entrada ali) — quem chama mostra "autoria
/// desconhecida" nesse caso, em vez de inventar um autor.
/// `[author1, author2, ...]` com mais de um autor distinto significa
/// "misto".
///
/// **Como funciona, sem depender de `helix_core` (nas-history
/// continua independente de `nas-editor`, ADR 0005 §9):** em vez de
/// reconstruir o texto inteiro e usar `ChangeSet`/`map_pos` do helix
/// pra remapear posições, mantém uma lista de intervalos com autor —
/// um "mapa de autoria" sobre o espaço de caracteres — e aplica, em
/// ordem cronológica, o mesmo efeito que cada entrada do journal teve
/// sobre o texto: o intervalo que ela tocou vira dela (substituindo o
/// que havia ali antes, se algo havia), e todo intervalo depois do
/// ponto de edição desloca pela diferença de tamanho
/// (`inserted.len() - (to - from)`). No fim, a posição de cada
/// intervalo já está nas coordenadas do texto **atual** — exatamente
/// o que `ChangeSet`/`map_pos` fariam, só que como uma lista de
/// intervalos em vez do tipo do helix (a ADR 0006 §2.1 decide o
/// mecanismo — compor remapeamentos em ordem cronológica — não exige
/// literalmente o tipo `ChangeSet`).
pub fn attribute_hunk(
    history_dir: &Path,
    file: &str,
    since_ts: u64,
    range: (usize, usize),
) -> io::Result<Vec<Author>> {
    let entries = crate::journal::file_history(history_dir, file)?;

    // Intervalos (início, fim, autor), em coordenadas que avançam
    // junto com cada entrada aplicada — começa vazio: nada é "de um
    // autor" até a primeira entrada depois do checkpoint tocar ali.
    let mut spans: Vec<(usize, usize, Author)> = Vec::new();

    for entry in entries
        .into_iter()
        .filter(|entry| entry.ts_start > since_ts)
    {
        let from = entry.from;
        let to = entry.to;
        let inserted_len = entry.inserted.chars().count();
        let delta = inserted_len as isize - (to - from) as isize;

        let mut next = Vec::with_capacity(spans.len() + 1);
        for (start, end, author) in spans {
            if end <= from {
                // Inteiramente antes da edição: não muda.
                next.push((start, end, author));
            } else if start >= to {
                // Inteiramente depois: desloca pela diferença de
                // tamanho que a edição introduziu.
                next.push((shift(start, delta), shift(end, delta), author));
            } else {
                // Sobrepõe a região editada — mantém só a parte fora
                // de [from, to); a parte dentro foi substituída pelo
                // texto desta entrada (vira autoria desta entrada,
                // tratada abaixo).
                if start < from {
                    next.push((start, from, author.clone()));
                }
                if end > to {
                    next.push((shift(to, delta), shift(end, delta), author));
                }
            }
        }
        next.push((from, from + inserted_len, entry.author));
        spans = next;
    }

    let (range_start, range_end) = range;
    let mut authors = Vec::new();
    for (start, end, author) in spans {
        if start < range_end && end > range_start && !authors.contains(&author) {
            authors.push(author);
        }
    }
    Ok(authors)
}

fn shift(pos: usize, delta: isize) -> usize {
    (pos as isize + delta).max(0) as usize
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::journal::Journal;

    #[test]
    fn hash_de_duas_insercoes_vazias_nao_colide() {
        let a = hunk_hash("f.txt", "ck1", "", "nova funcao a()\n", 10);
        let b = hunk_hash("f.txt", "ck1", "", "nova funcao b()\n", 40);
        assert_ne!(
            a, b,
            "duas inserções diferentes, mesmo 'antes' vazio, não deveriam colidir"
        );
    }

    #[test]
    fn hash_e_estavel_e_determinístico() {
        let a = hunk_hash("f.txt", "ck1", "velho", "novo", 3);
        let b = hunk_hash("f.txt", "ck1", "velho", "novo", 3);
        assert_eq!(a, b);
    }

    #[test]
    fn marca_persiste_e_filtra_por_checkpoint() {
        let dir = tempfile::tempdir().unwrap();
        let mark = ReviewMark {
            ts: 1,
            reviewer: Author::User,
            from_checkpoint: "ck1".to_string(),
            file: "f.txt".to_string(),
            hunk_hash: "abc".to_string(),
            decision: ReviewDecision::Accepted,
        };
        record_mark(dir.path(), &mark).unwrap();

        let other = ReviewMark {
            from_checkpoint: "ck2".to_string(),
            ..mark.clone()
        };
        record_mark(dir.path(), &other).unwrap();

        let marks = marks_for_checkpoint(dir.path(), "ck1").unwrap();
        assert_eq!(marks, vec![mark]);
    }

    #[test]
    fn log_de_sessao_persiste() {
        let dir = tempfile::tempdir().unwrap();
        let entry = ReviewLogEntry {
            started_at: 10,
            ended_at: 20,
            from_checkpoint: "ck1".to_string(),
            total_hunks: 3,
            accepted: 2,
            rejected: 1,
        };
        record_log_entry(dir.path(), &entry).unwrap();
        assert_eq!(read_log_entries(dir.path()).unwrap(), vec![entry]);
    }

    fn ai(session: &str) -> Author {
        Author::Ai {
            agent: "claude-code".to_string(),
            session: session.to_string(),
        }
    }

    #[test]
    fn sem_entrada_nenhuma_devolve_lista_vazia() {
        let dir = tempfile::tempdir().unwrap();
        let authors = attribute_hunk(dir.path(), "f.txt", 0, (0, 10)).unwrap();
        assert!(authors.is_empty());
    }

    #[test]
    fn um_autor_so_quando_so_a_ia_tocou_o_trecho() {
        let dir = tempfile::tempdir().unwrap();
        let mut journal = Journal::open(dir.path()).unwrap();
        journal
            .record(100, ai("s1"), "edit", "f.txt", 0, 0, "", "ola mundo")
            .unwrap();
        journal.flush(100).unwrap();

        let authors = attribute_hunk(dir.path(), "f.txt", 50, (0, 9)).unwrap();
        assert_eq!(authors, vec![ai("s1")]);
    }

    #[test]
    fn misto_quando_ia_e_usuario_tocam_o_mesmo_trecho() {
        let dir = tempfile::tempdir().unwrap();
        let mut journal = Journal::open(dir.path()).unwrap();
        // checkpoint em ts=50; a IA insere "ola mundo" em 0 (9 chars).
        journal
            .record(100, ai("s1"), "edit", "f.txt", 0, 0, "", "ola mundo")
            .unwrap();
        journal.flush(100).unwrap();
        // depois, o usuário edita dentro da mesma região (substitui
        // "mundo" por "terra", em [4, 9) -> ainda dentro do trecho).
        journal
            .record(200, Author::User, "edit", "f.txt", 4, 9, "mundo", "terra")
            .unwrap();
        journal.flush(200).unwrap();

        // o trecho inteiro, nas coordenadas atuais ("ola terra" tem 9
        // chars também, mas isso não importa pro teste: só a faixa).
        let authors = attribute_hunk(dir.path(), "f.txt", 50, (0, 9)).unwrap();
        assert_eq!(authors.len(), 2);
        assert!(authors.contains(&ai("s1")));
        assert!(authors.contains(&Author::User));
    }

    #[test]
    fn entrada_antes_do_checkpoint_nao_conta() {
        let dir = tempfile::tempdir().unwrap();
        let mut journal = Journal::open(dir.path()).unwrap();
        journal
            .record(10, Author::User, "edit", "f.txt", 0, 0, "", "antigo")
            .unwrap();
        journal.flush(10).unwrap();
        journal
            .record(100, ai("s1"), "edit", "f.txt", 0, 0, "", "novo-")
            .unwrap();
        journal.flush(100).unwrap();

        // since_ts = 50: só a entrada em ts=100 (da IA) conta.
        let authors = attribute_hunk(dir.path(), "f.txt", 50, (0, 5)).unwrap();
        assert_eq!(authors, vec![ai("s1")]);
    }
}
