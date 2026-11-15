//! Configura `tracing` para gravar em arquivo, nunca na tela do TUI.

use std::fs::OpenOptions;
use std::io;
use std::path::PathBuf;
use std::sync::Mutex;

/// Inicializa o logging e devolve o caminho do arquivo de log.
pub fn init() -> io::Result<PathBuf> {
    let path = std::env::temp_dir().join("nas.log");
    let file = OpenOptions::new().create(true).append(true).open(&path)?;

    tracing_subscriber::fmt()
        .with_writer(Mutex::new(file))
        .with_ansi(false)
        .init();

    Ok(path)
}
