//! Mover um arquivo pra lixeira do sistema (`~/.local/share/Trash`,
//! especificação freedesktop.org) em vez de apagar de vez — usado ao
//! restaurar um checkpoint que não conhece um arquivo criado depois
//! dele (docs/decisions/0005-historico-com-autoria.md §4): "nunca
//! apagados de vez".
//!
//! Implementa só o suficiente da especificação pra ser seguro e
//! interoperável com um gerenciador de lixeira de verdade: move o
//! arquivo pra `Trash/files/`, sem colidir com um nome já lá, e grava
//! o `.trashinfo` correspondente em `Trash/info/` com o caminho
//! original e a data. Não lida com múltiplos pontos de montagem
//! (`$topdir/.Trash`) — só a lixeira "home" (`$XDG_DATA_HOME/Trash`,
//! ou `~/.local/share/Trash` sem a variável), suficiente pro caso de
//! uso aqui (arquivos dentro do projeto, tipicamente no mesmo
//! filesystem do HOME).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Move `path` pra lixeira do sistema, resolvendo `$XDG_DATA_HOME`
/// (ou `~/.local/share` sem a variável) — ver [`move_to_trash_in`].
pub fn move_to_trash(path: &Path) -> io::Result<()> {
    let trash_home = data_home()?.join("Trash");
    move_to_trash_in(path, &trash_home)
}

/// Como [`move_to_trash`], recebendo a pasta da lixeira diretamente —
/// pra ficar testável sem tocar na lixeira de verdade do usuário.
pub fn move_to_trash_in(path: &Path, trash_home: &Path) -> io::Result<()> {
    let files_dir = trash_home.join("files");
    let info_dir = trash_home.join("info");
    fs::create_dir_all(&files_dir)?;
    fs::create_dir_all(&info_dir)?;

    let file_name = path.file_name().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "caminho sem nome de arquivo, não pode ir pra lixeira",
        )
    })?;

    let (trash_name, dest) = unique_destination(&files_dir, file_name)?;
    let original = path
        .canonicalize()
        .unwrap_or_else(|_| std::env::current_dir().unwrap_or_default().join(path));

    let info_path = info_dir.join(format!("{trash_name}.trashinfo"));
    let info_content = format!(
        "[Trash Info]\nPath={}\nDeletionDate={}\n",
        encode_path(&original),
        deletion_date(now_unix()),
    );
    fs::write(&info_path, info_content)?;

    if let Err(err) = fs::rename(path, &dest) {
        let _ = fs::remove_file(&info_path);
        return Err(err);
    }
    Ok(())
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// `$XDG_DATA_HOME`, ou `~/.local/share` sem a variável — mesmo
/// padrão de fallback manual já usado por `state_home` pra
/// `$XDG_STATE_HOME`.
fn data_home() -> io::Result<PathBuf> {
    if let Some(dir) = std::env::var_os("XDG_DATA_HOME")
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
    Ok(PathBuf::from(home).join(".local/share"))
}

/// Um nome livre em `files_dir` pra `file_name` — acrescenta `-2`,
/// `-3`, ... antes da extensão se já existir um arquivo (ou
/// `.trashinfo`) com esse nome, pra nunca sobrescrever um item já na
/// lixeira.
fn unique_destination(
    files_dir: &Path,
    file_name: &std::ffi::OsStr,
) -> io::Result<(String, PathBuf)> {
    let base = file_name.to_string_lossy().into_owned();
    let (stem, ext) = match base.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s.to_string(), format!(".{e}")),
        _ => (base.clone(), String::new()),
    };

    let mut candidate = base.clone();
    let mut n = 2;
    while files_dir.join(&candidate).exists() {
        candidate = format!("{stem}-{n}{ext}");
        n += 1;
    }
    let dest = files_dir.join(&candidate);
    Ok((candidate, dest))
}

/// Escapa um caminho pro formato `.trashinfo` (URI-like, mas a
/// especificação só exige escapar o que quebraria o parsing de uma
/// linha `Key=Value`: `%`, e caracteres de controle como `\n`) —
/// percent-encoding mínimo, sem depender de uma crate de URL só pra
/// isto.
fn encode_path(path: &Path) -> String {
    path.display()
        .to_string()
        .chars()
        .flat_map(|c| {
            if c == '%' || c.is_control() {
                format!("%{:02X}", c as u32).chars().collect::<Vec<_>>()
            } else {
                vec![c]
            }
        })
        .collect()
}

/// `AAAA-MM-DDThh:mm:ss`, como a especificação exige — mesmo
/// algoritmo (Howard Hinnant, dias desde a época -> ano/mês/dia
/// proléptico-gregoriano) usado em `nas-term` pra formatar os
/// timestamps do journal; duplicado aqui (não vale criar uma
/// dependência entre os dois só por uma função de formatação de ~15
/// linhas).
fn deletion_date(secs: u64) -> String {
    let days = (secs / 86400) as i64;
    let secs_of_day = secs % 86400;
    let (year, month, day) = civil_from_days(days);
    let hour = secs_of_day / 3600;
    let minute = (secs_of_day % 3600) / 60;
    let second = secs_of_day % 60;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}")
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn move_to_trash_in_move_o_arquivo_e_grava_o_trashinfo() {
        let project = tempfile::tempdir().unwrap();
        let trash_home = tempfile::tempdir().unwrap();
        let file_path = project.path().join("descartado.txt");
        fs::write(&file_path, "conteudo").unwrap();
        let original_canonical = file_path.canonicalize().unwrap();

        move_to_trash_in(&file_path, trash_home.path()).unwrap();

        assert!(
            !file_path.exists(),
            "o arquivo original deveria ter sido movido"
        );
        let trashed = trash_home.path().join("files").join("descartado.txt");
        assert_eq!(fs::read_to_string(&trashed).unwrap(), "conteudo");

        let info = fs::read_to_string(
            trash_home
                .path()
                .join("info")
                .join("descartado.txt.trashinfo"),
        )
        .unwrap();
        assert!(info.starts_with("[Trash Info]\n"));
        assert!(info.contains("Path="));
        assert!(info.contains(&original_canonical.display().to_string()));
        assert!(info.contains("DeletionDate="));
    }

    #[test]
    fn colisao_de_nome_na_lixeira_ganha_um_sufixo() {
        let project = tempfile::tempdir().unwrap();
        let trash_home = tempfile::tempdir().unwrap();

        let first = project.path().join("a.txt");
        fs::write(&first, "primeiro").unwrap();
        move_to_trash_in(&first, trash_home.path()).unwrap();

        let second = project.path().join("a.txt");
        fs::write(&second, "segundo").unwrap();
        move_to_trash_in(&second, trash_home.path()).unwrap();

        let files_dir = trash_home.path().join("files");
        assert_eq!(
            fs::read_to_string(files_dir.join("a.txt")).unwrap(),
            "primeiro"
        );
        assert_eq!(
            fs::read_to_string(files_dir.join("a-2.txt")).unwrap(),
            "segundo"
        );
    }

    #[test]
    fn arquivo_sem_extensao_tambem_funciona() {
        let project = tempfile::tempdir().unwrap();
        let trash_home = tempfile::tempdir().unwrap();
        let file_path = project.path().join("sem_extensao");
        fs::write(&file_path, "x").unwrap();

        move_to_trash_in(&file_path, trash_home.path()).unwrap();

        assert!(
            trash_home
                .path()
                .join("files")
                .join("sem_extensao")
                .exists()
        );
    }

    #[test]
    fn deletion_date_bate_com_data_conhecida() {
        assert_eq!(deletion_date(0), "1970-01-01T00:00:00");
        assert_eq!(deletion_date(1_609_459_200), "2021-01-01T00:00:00");
    }
}
