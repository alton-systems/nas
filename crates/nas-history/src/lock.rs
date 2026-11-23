//! Trava entre processos pro histórico de um projeto — ADR 0006,
//! revisão da etapa 3c: `nas-term` e `nas --mcp` podem escrever ao
//! mesmo tempo no journal, no armazenamento, nos originais e nos
//! checkpoints do mesmo projeto.
//!
//! **Uma trava só, pro projeto inteiro** — não uma por recurso
//! (journal, originals, checkpoints, objects). Com uma trava só, não
//! existe ordem de aquisição pra documentar nem deadlock pra evitar:
//! nunca há uma segunda trava esperando a primeira, porque nunca
//! existe uma segunda. Qualquer escritor deste histórico — `flush`
//! do journal, `ensure_tracked`, criar ou podar um checkpoint —
//! adquire esta mesma trava antes de tocar o disco.

use std::fs::{File, OpenOptions};
use std::io;
use std::path::Path;

/// `<history_dir>/.lock`, aberto (ou criado) e travado
/// exclusivamente — bloqueia até conseguir, nunca falha por "já
/// travado". Liberada automaticamente quando sai de escopo (fechar o
/// arquivo já libera o `flock` subjacente).
pub struct HistoryLock {
    _file: File,
}

impl HistoryLock {
    pub fn acquire(history_dir: &Path) -> io::Result<Self> {
        let path = history_dir.join(".lock");
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)?;
        lock_exclusive(&file)?;
        Ok(HistoryLock { _file: file })
    }
}

#[cfg(unix)]
fn lock_exclusive(file: &File) -> io::Result<()> {
    use std::os::fd::AsRawFd;
    // SAFETY: `fd` vem de um `File` válido que continua vivo durante a
    // chamada (empréstimo de `file`); `flock(2)` com `LOCK_EX` só lê o
    // descritor e bloqueia a chamadora, sem tocar a memória do processo.
    let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
    if result != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(not(unix))]
fn lock_exclusive(_file: &File) -> io::Result<()> {
    // Sem flock fora do Unix — o nas não tem build nem CI fora dele
    // hoje (CLAUDE.md); documentado como lacuna, não fingido.
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn uma_segunda_trava_espera_a_primeira_liberar() {
        let dir = tempfile::tempdir().unwrap();
        let dir_path = dir.path().to_path_buf();

        let first = HistoryLock::acquire(&dir_path).expect("primeira trava");

        let (tx, rx) = mpsc::channel();
        let dir_for_thread = dir_path.clone();
        let handle = std::thread::spawn(move || {
            // Só devolve depois de conseguir a trava — se a primeira
            // ainda estivesse presa, isto bloquearia pra sempre.
            let _second = HistoryLock::acquire(&dir_for_thread).expect("segunda trava");
            tx.send(()).unwrap();
        });

        // A segunda trava não deveria conseguir enquanto a primeira
        // está presa.
        assert!(rx.recv_timeout(Duration::from_millis(200)).is_err());

        drop(first);
        rx.recv_timeout(Duration::from_secs(5))
            .expect("a segunda trava deveria liberar pouco depois da primeira");
        handle.join().unwrap();
    }
}
