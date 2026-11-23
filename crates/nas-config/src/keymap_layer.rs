//! Aplica o `keymap.toml` do usuário por cima de
//! `nas_keys::DEFAULT_TOML` — ADR 0007 §1: "aplicado por cima do
//! default.toml, só com as diferenças". Não existe `keymap.toml` de
//! projeto (ADR 0007 §4) — este módulo só lida com a camada de
//! usuário.

use std::path::Path;

use nas_keys::Keymap;

use crate::error::ConfigError;

/// Remove um atalho que o padrão declarava: `chave = false`, em
/// qualquer tabela de binding (`[visual]`, `[system]`,
/// `[prefixes.*]`, `[system_prefixes.*]`) — documentado em
/// `docs/config.md`.
fn is_removal(value: &toml::Value) -> bool {
    matches!(value, toml::Value::Boolean(false))
}

/// Sem `keymap.toml` de usuário, o keymap é exatamente o padrão —
/// `None` é o caso comum (a maioria não tem o arquivo).
pub(crate) fn load_keymap(file: &Path, user_source: Option<&str>) -> Result<Keymap, ConfigError> {
    let Some(user_source) = user_source else {
        return Ok(Keymap::default_keymap());
    };

    let default_value: toml::Value = toml::from_str(nas_keys::DEFAULT_TOML)
        .expect("keymap/default.toml embutido deveria ser válido");
    let user_value: toml::Value = toml::from_str(user_source)
        .map_err(|err| ConfigError::new(file, format!("TOML inválido: {err}")))?;

    let merged = merge(default_value, user_value);
    let merged_source = toml::to_string(&merged).map_err(|err| {
        ConfigError::new(file, format!("erro interno ao combinar o keymap: {err}"))
    })?;

    Keymap::from_toml_str(&merged_source).map_err(|err| ConfigError::new(file, err.to_string()))
}

/// Merge genérico por valor TOML: tabelas se mesclam recursivamente
/// (cobre `[visual]`, `[prefixes.go]`, `[system_prefixes.tempo]`,
/// ... sem precisar listar cada seção); dentro de uma tabela, uma
/// chave cujo valor em `over` é `false` é **removida** de `base` em
/// vez de inserida — é como um atalho do padrão deixa de existir.
/// Fora de uma tabela (um valor escalar substituindo outro), `over`
/// sempre vence.
fn merge(base: toml::Value, over: toml::Value) -> toml::Value {
    match (base, over) {
        (toml::Value::Table(mut base), toml::Value::Table(over)) => {
            for (key, value) in over {
                if is_removal(&value) {
                    base.remove(&key);
                    continue;
                }
                let merged = match base.remove(&key) {
                    Some(existing) => merge(existing, value),
                    None => value,
                };
                base.insert(key, merged);
            }
            toml::Value::Table(base)
        }
        (_, over) => over,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nas_keys::{Action, Binding, Key};
    use std::path::PathBuf;

    fn file() -> PathBuf {
        PathBuf::from("keymap.toml")
    }

    #[test]
    fn sem_arquivo_de_usuario_e_exatamente_o_padrao() {
        let keymap = load_keymap(&file(), None).expect("deveria carregar");
        assert_eq!(keymap, Keymap::default_keymap());
    }

    #[test]
    fn sobrescreve_um_atalho_existente() {
        let keymap =
            load_keymap(&file(), Some("[visual]\nj = \"move-right\"\n")).expect("deveria carregar");
        assert_eq!(
            keymap.visual.get(&Key::char('j')),
            Some(&Binding::Action(Action::MoveRight))
        );
    }

    #[test]
    fn remove_um_atalho_do_padrao_com_false() {
        let padrao = Keymap::default_keymap();
        assert!(padrao.visual.contains_key(&Key::char('j')));

        let keymap = load_keymap(&file(), Some("[visual]\nj = false\n")).expect("deveria carregar");
        assert!(!keymap.visual.contains_key(&Key::char('j')));
        // o resto do padrão continua intacto.
        assert!(keymap.visual.contains_key(&Key::char('k')));
    }

    #[test]
    fn remove_e_sobrescreve_ao_mesmo_tempo() {
        let keymap = load_keymap(&file(), Some("[visual]\nj = false\nz = \"move-right\"\n"))
            .expect("deveria carregar");
        assert!(!keymap.visual.contains_key(&Key::char('j')));
        assert_eq!(
            keymap.visual.get(&Key::char('z')),
            Some(&Binding::Action(Action::MoveRight))
        );
    }

    #[test]
    fn comando_inexistente_no_keymap_e_recusado() {
        let err = load_keymap(&file(), Some("[visual]\nj = \"voar\"\n")).unwrap_err();
        assert_eq!(err.file, file());
    }

    #[test]
    fn remove_dentro_de_um_prefixo_aninhado() {
        let keymap =
            load_keymap(&file(), Some("[prefixes.go]\ng = false\n")).expect("deveria carregar");
        let go = keymap.prefixes.get("go").expect("prefixo go");
        assert!(!go.contains_key(&Key::char('g')));
    }
}
