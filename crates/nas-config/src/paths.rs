use std::io;
use std::path::{Path, PathBuf};

/// `$XDG_CONFIG_HOME`, ou `~/.config` sem a variável (ou vazia) —
/// mesmo fallback manual que `nas_history::state_home` já usa pra
/// `$XDG_STATE_HOME` (ADR 0005 §5), só que pra configuração em vez
/// de estado.
pub fn config_home() -> io::Result<PathBuf> {
    if let Some(dir) = std::env::var_os("XDG_CONFIG_HOME")
        && !dir.is_empty()
    {
        return Ok(PathBuf::from(dir));
    }
    let home = std::env::var_os("HOME").ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "variável de ambiente HOME não definida",
        )
    })?;
    Ok(PathBuf::from(home).join(".config"))
}

/// `<config_home>/nas/` — onde `config.toml`/`keymap.toml` do
/// usuário ficam (ADR 0007 §1).
pub fn user_config_dir() -> io::Result<PathBuf> {
    Ok(config_home()?.join("nas"))
}

/// `.nas/config.toml` na raiz do projeto (ADR 0007 §2/§4). `root` já
/// deve ser a raiz identificada por `nas_history::ProjectId` (mesmo
/// critério, ADR 0005 §6) — este módulo não descobre a raiz por
/// conta própria, pra não duplicar essa lógica.
pub fn project_config_path(root: &Path) -> PathBuf {
    root.join(".nas").join("config.toml")
}
