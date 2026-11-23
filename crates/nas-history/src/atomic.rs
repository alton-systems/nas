//! Escrever um arquivo por inteiro ou não escrever nada — ADR 0006,
//! revisão da etapa 3c: "toda gravação de objeto, manifesto e
//! originais por arquivo temporário + rename atômico". Sem isto, um
//! leitor (o outro processo, ou o mesmo depois de um pane) poderia
//! ver um arquivo pela metade se o processo escritor morresse no
//! meio de um `fs::write` — `rename` dentro do mesmo sistema de
//! arquivos é atômico no POSIX: o leitor vê o conteúdo antigo
//! completo, ou o novo completo, nunca uma mistura.

use std::fs;
use std::io;
use std::path::Path;

/// Escreve `content` em `path` via um arquivo temporário (no mesmo
/// diretório, pra `rename` continuar no mesmo sistema de arquivos) +
/// `rename` por cima do destino final. O nome do temporário inclui o
/// PID — já é chamado com `HistoryLock` adquirida (um escritor por
/// vez, por projeto), então o PID já basta pra nunca colidir com
/// outra chamada concorrente de verdade.
pub(crate) fn write_atomic(path: &Path, content: &[u8]) -> io::Result<()> {
    let file_name = path.file_name().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "caminho sem nome de arquivo pra gravação atômica",
        )
    })?;
    let tmp_name = format!(
        ".{}.tmp-{}",
        file_name.to_string_lossy(),
        std::process::id()
    );
    let tmp_path = path.with_file_name(tmp_name);

    fs::write(&tmp_path, content)?;
    fs::rename(&tmp_path, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escreve_e_substitui_por_inteiro() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("arquivo.txt");

        write_atomic(&path, b"primeiro").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"primeiro");

        write_atomic(&path, b"segundo, mais longo que o primeiro").unwrap();
        assert_eq!(
            fs::read(&path).unwrap(),
            b"segundo, mais longo que o primeiro"
        );

        // nenhum temporário sobra no diretório.
        let leftovers: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp-"))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }
}
