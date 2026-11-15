//! Configura `tracing` para gravar em arquivo, nunca na tela do TUI.
//!
//! O arquivo de log vive em `$XDG_STATE_HOME/nas/nas.log`, com fallback
//! para `~/.local/state/nas/nas.log` quando `XDG_STATE_HOME` não está
//! definida.

use std::fs::{self, OpenOptions};
use std::io;
use std::path::PathBuf;
use std::sync::Mutex;

/// Inicializa o logging e devolve o caminho do arquivo de log.
pub fn init() -> io::Result<PathBuf> {
    let dir = state_dir().join("nas");
    fs::create_dir_all(&dir)?;
    let path = dir.join("nas.log");
    let file = OpenOptions::new().create(true).append(true).open(&path)?;

    tracing_subscriber::fmt()
        .with_writer(Mutex::new(file))
        .with_ansi(false)
        .init();

    Ok(path)
}

fn state_dir() -> PathBuf {
    if let Some(xdg_state_home) = std::env::var_os("XDG_STATE_HOME") {
        return PathBuf::from(xdg_state_home);
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    home.join(".local/state")
}
