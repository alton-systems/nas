//! Orquestra as duas camadas de dados (ADR 0007 §1/§2): lê e valida
//! `config.toml`/`keymap.toml` do usuário e `.nas/config.toml` do
//! projeto. Cada camada é validada **por inteiro e separadamente**
//! (§3, "camadas isoladas"): um erro no projeto nunca descarta a
//! configuração do usuário — só a camada de projeto é ignorada,
//! com o erro devolvido em `Loaded::warnings`. Um erro na camada do
//! usuário (ou no keymap), por outro lado, continua fatal — essa é
//! a única camada de verdade que a abertura normal do nas tem, e
//! continuar com metade dela aplicada seria pior que ficar no
//! padrão embutido.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use nas_keys::Keymap;

use crate::config::Config;
use crate::error::ConfigError;
use crate::value::OptionValue;
use crate::{keymap_layer, paths, toml_parse};

/// O que uma carga completa produz, depois de validar tudo.
#[derive(Debug)]
pub struct Loaded {
    pub config: Config,
    pub keymap: Keymap,
    /// Erros de uma camada que foi **isolada**, não fatal — hoje só
    /// `.nas/config.toml` (projeto): se ele falhar, a carga inteira
    /// ainda é `Ok`, com o padrão e o usuário aplicados, e o(s)
    /// erro(s) do projeto aqui, pra quem chama mostrar na echo area
    /// sem deixar de abrir.
    pub warnings: Vec<ConfigError>,
    /// Os caminhos efetivamente usados (existentes ou não) — pra
    /// `nas-term` saber quais arquivos, se salvos, devem disparar um
    /// recarregamento (ADR 0007, Parte 2, "Recarregar").
    pub user_config_path: Option<PathBuf>,
    pub user_keymap_path: Option<PathBuf>,
    pub project_config_path: Option<PathBuf>,
}

/// Carrega as camadas de dados. `user_dir` é
/// `<config_home>/nas/` ([`crate::user_config_dir`]) — recebido como
/// parâmetro, não resolvido por conta própria aqui, pra esta função
/// continuar testável sem depender de variável de ambiente alguma
/// (mesma separação que `nas_history::ProjectId::discover` versus
/// `nas_history::state_home` já faz). `project_root` é a raiz já
/// identificada pelo chamador (ADR 0005 §6); `None` quando não há
/// projeto — e então não há camada de projeto nenhuma. Caminhos que
/// não existem simplesmente não contribuem nenhum valor — não é erro
/// não ter `config.toml`/`keymap.toml` (ADR 0007, "sem arquivos de
/// configuração, o comportamento continua idêntico ao de hoje").
///
/// Só a camada de usuário (`config.toml`/`keymap.toml`) é fatal —
/// `Err` quando ela (ou o keymap) tiver um erro, mantendo quem chama
/// na última configuração válida. A camada de projeto nunca é fatal:
/// um erro nela vai pra `Loaded::warnings`, e o resto (padrão +
/// usuário) é aplicado normalmente.
pub fn load(
    user_dir: Option<&Path>,
    project_root: Option<&Path>,
) -> Result<Loaded, Vec<ConfigError>> {
    let mut fatal_errors = Vec::new();
    let mut values: HashMap<String, OptionValue> = HashMap::new();

    let user_config_path = user_dir.map(|dir| dir.join("config.toml"));
    let user_keymap_path = user_dir.map(|dir| dir.join("keymap.toml"));

    if let Some(path) = &user_config_path
        && let Some(source) = read_if_exists(path, &mut fatal_errors)
    {
        match toml_parse::parse_config_toml(path, &source, false) {
            Ok(parsed) => values.extend(parsed),
            Err(mut parse_errors) => fatal_errors.append(&mut parse_errors),
        }
    }

    let user_keymap_source = user_keymap_path
        .as_ref()
        .and_then(|path| read_if_exists(path, &mut fatal_errors));

    // Camada de projeto: isolada de propósito (ADR 0007 §3/§4) — os
    // erros dela vão pra um balde próprio, nunca pra `fatal_errors`,
    // e só os valores dela (nunca os do usuário) somem se ela falhar.
    let mut warnings = Vec::new();
    let project_config_path = project_root.map(paths::project_config_path);
    if let Some(path) = &project_config_path
        && let Some(source) = read_if_exists(path, &mut warnings)
    {
        match toml_parse::parse_config_toml(path, &source, true) {
            Ok(parsed) => values.extend(parsed),
            Err(mut parse_errors) => warnings.append(&mut parse_errors),
        }
    }

    let keymap_file = user_keymap_path
        .clone()
        .unwrap_or_else(|| Path::new("keymap.toml").to_path_buf());
    let keymap = match keymap_layer::load_keymap(&keymap_file, user_keymap_source.as_deref()) {
        Ok(keymap) => Some(keymap),
        Err(err) => {
            fatal_errors.push(err);
            None
        }
    };

    if !fatal_errors.is_empty() {
        return Err(fatal_errors);
    }

    Ok(Loaded {
        config: Config::from_values(&values),
        keymap: keymap.expect("sem erros fatais, o keymap deveria ter carregado"),
        warnings,
        user_config_path,
        user_keymap_path,
        project_config_path,
    })
}

/// Lê `path` se existir; um erro de E/S real (sem permissão, etc. —
/// não "não existe") entra em `errors` e devolve `None`, pra não
/// fingir que o arquivo simplesmente não estava lá.
fn read_if_exists(path: &Path, errors: &mut Vec<ConfigError>) -> Option<String> {
    match std::fs::read_to_string(path) {
        Ok(source) => Some(source),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
        Err(err) => {
            errors.push(ConfigError::new(
                path,
                format!("não foi possível ler: {err}"),
            ));
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nas_keys::Layout;
    use std::fs;

    #[test]
    fn sem_nenhum_arquivo_e_o_padrao() {
        let loaded = load(None, None).expect("deveria carregar");
        assert_eq!(loaded.config, Config::default());
        assert_eq!(loaded.keymap, Keymap::default_keymap());
        assert!(loaded.warnings.is_empty());
    }

    #[test]
    fn usuario_sobrescreve_o_padrao_e_projeto_sobrescreve_o_usuario() {
        let user_dir = tempfile::tempdir().unwrap();
        fs::write(user_dir.path().join("config.toml"), "tab_width = 2\n").unwrap();

        let project = tempfile::tempdir().unwrap();
        let project_nas_dir = project.path().join(".nas");
        fs::create_dir_all(&project_nas_dir).unwrap();
        fs::write(project_nas_dir.join("config.toml"), "tab_width = 8\n").unwrap();

        // só o usuário: vale o do usuário.
        let loaded = load(Some(user_dir.path()), None).expect("deveria carregar");
        assert_eq!(loaded.config.tab_width, 2);

        // usuário + projeto: o projeto vence.
        let loaded = load(Some(user_dir.path()), Some(project.path())).expect("deveria carregar");
        assert_eq!(loaded.config.tab_width, 8);
    }

    #[test]
    fn arquivo_de_usuario_invalido_nao_aplica_nada_e_devolve_os_erros() {
        let user_dir = tempfile::tempdir().unwrap();
        fs::write(user_dir.path().join("config.toml"), "tab_width = 999\n").unwrap();

        let errors = load(Some(user_dir.path()), None).expect_err("deveria recusar");
        assert_eq!(errors.len(), 1);
    }

    #[test]
    fn ausencia_de_arquivos_nao_e_erro() {
        let user_dir = tempfile::tempdir().unwrap();
        // nem `config.toml` nem `keymap.toml` existem dentro de
        // `user_dir` — nada a ler.
        let loaded = load(Some(user_dir.path()), None).expect("ausência não deveria falhar");
        assert_eq!(loaded.config, Config::default());
    }

    #[test]
    fn projeto_invalido_nao_descarta_a_configuracao_do_usuario() {
        let user_dir = tempfile::tempdir().unwrap();
        fs::write(
            user_dir.path().join("config.toml"),
            "keyboard_layout = \"qwerty-abnt\"\n",
        )
        .unwrap();

        let project = tempfile::tempdir().unwrap();
        let project_nas_dir = project.path().join(".nas");
        fs::create_dir_all(&project_nas_dir).unwrap();
        // "keyboard_layout" não vale em `.nas/config.toml` — opção
        // proibida, mas só essa camada deveria ser descartada.
        fs::write(
            project_nas_dir.join("config.toml"),
            "keyboard_layout = \"qwerty\"\n",
        )
        .unwrap();

        let loaded = load(Some(user_dir.path()), Some(project.path()))
            .expect("projeto inválido não deveria impedir a carga");

        assert_eq!(loaded.config.keyboard_layout, Layout::QwertyAbnt);
        assert_eq!(loaded.warnings.len(), 1);
        assert!(
            loaded.warnings[0]
                .message
                .contains("só vale na configuração do usuário")
        );
    }
}
