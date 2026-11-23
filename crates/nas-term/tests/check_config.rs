//! `nas --check-config` roda o binário de verdade — é o único jeito
//! de testar o código de saída do processo (ADR
//! 0007-configuracao-em-camadas.md, Parte 2: "sai com código
//! diferente de zero se houver erro"), que `main()` só produz
//! retornando de verdade do processo, não de uma função Rust comum.

use std::process::Command;

#[test]
fn sem_arquivos_sai_com_sucesso() {
    let dir = tempfile::tempdir().expect("diretório temporário");
    let output = Command::new(env!("CARGO_BIN_EXE_nas"))
        .arg("--check-config")
        .env("XDG_CONFIG_HOME", dir.path())
        .current_dir(dir.path())
        .output()
        .expect("executar nas --check-config");
    assert!(output.status.success());
}

#[test]
fn com_opcao_invalida_sai_com_erro_e_imprime_a_mensagem() {
    let dir = tempfile::tempdir().expect("diretório temporário");
    let nas_dir = dir.path().join("nas");
    std::fs::create_dir_all(&nas_dir).expect("criar pasta de configuração");
    std::fs::write(nas_dir.join("config.toml"), "tab_width = 999\n")
        .expect("escrever config.toml inválido");

    let output = Command::new(env!("CARGO_BIN_EXE_nas"))
        .arg("--check-config")
        .env("XDG_CONFIG_HOME", dir.path())
        .current_dir(dir.path())
        .output()
        .expect("executar nas --check-config");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("tab_width"), "stderr foi: {stderr}");
}
