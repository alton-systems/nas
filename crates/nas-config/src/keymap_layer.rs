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

    let merged = merge_root(default_value, user_value);
    let merged_source = toml::to_string(&merged).map_err(|err| {
        ConfigError::new(file, format!("erro interno ao combinar o keymap: {err}"))
    })?;

    Keymap::from_toml_str(&merged_source).map_err(|err| ConfigError::new(file, err.to_string()))
}

/// Mescla a raiz do keymap — conhece as seções de
/// `default.toml`/`docs/keys.md` por nome, pra saber até que
/// profundidade mesclar tabela por tabela antes de parar e
/// substituir por inteiro (ver `merge_bindings`). Não é um merge
/// genérico de qualquer árvore TOML — um keymap tem uma forma fixa.
fn merge_root(base: toml::Value, over: toml::Value) -> toml::Value {
    let (mut base, over) = match (base, over) {
        (toml::Value::Table(base), toml::Value::Table(over)) => (base, over),
        (_, over) => return over,
    };

    for (section, over_value) in over {
        let merged = match section.as_str() {
            // Tabelas de grupos nomeados (`prefixes.go`,
            // `objects.word`, `system_prefixes.tempo`) — mais um
            // nível antes de chegar numa tecla de verdade.
            "prefixes" | "objects" | "system_prefixes" => match base.remove(&section) {
                Some(existing) => merge_group_of_groups(existing, over_value),
                None => over_value,
            },
            // `[insert]`/`[visual]`/`[system]`/`[commands]` — tecla
            // (ou nome de comando) direto pra um valor-folha.
            _ => match base.remove(&section) {
                Some(existing) => merge_bindings(existing, over_value),
                None => over_value,
            },
        };
        base.insert(section, merged);
    }
    toml::Value::Table(base)
}

/// Uma tabela de grupos nomeados — `[prefixes]` (`go`, `leader`),
/// `[objects]` (`word`, `paragraph`, `whole`), `[system_prefixes]`
/// (`tempo`). Cada grupo, por sua vez, é uma tabela de tecla →
/// valor-folha, mesclada por [`merge_bindings`].
fn merge_group_of_groups(base: toml::Value, over: toml::Value) -> toml::Value {
    let (mut base, over) = match (base, over) {
        (toml::Value::Table(base), toml::Value::Table(over)) => (base, over),
        (_, over) => return over,
    };
    for (group, over_value) in over {
        let merged = match base.remove(&group) {
            Some(existing) => merge_bindings(existing, over_value),
            None => over_value,
        };
        base.insert(group, merged);
    }
    toml::Value::Table(base)
}

/// Uma tabela de tecla → valor-folha (um atalho: `"move-right"`,
/// `{ prefix = "go" }`, `{ verb = "delete" }`, `{ action = ...,
/// destructive = ... }`; ou, em `[commands]`, nome de comando →
/// descrição). **Cada valor é substituído por inteiro, nunca
/// mesclado campo a campo** — um atalho não é um objeto aberto pra
/// combinar com outro; `g = { verb = "delete" }` por cima de
/// `g = { prefix = "go" }` apaga o `prefix` e fica só com o `verb`,
/// nunca os dois juntos. Uma chave cujo valor em `over` é `false` é
/// **removida** de `base`, em vez de inserida — é como um atalho do
/// padrão deixa de existir (ver `docs/config.md`).
fn merge_bindings(base: toml::Value, over: toml::Value) -> toml::Value {
    let (mut base, over) = match (base, over) {
        (toml::Value::Table(base), toml::Value::Table(over)) => (base, over),
        (_, over) => return over,
    };
    for (key, value) in over {
        if is_removal(&value) {
            base.remove(&key);
            continue;
        }
        base.insert(key, value);
    }
    toml::Value::Table(base)
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

    #[test]
    fn atalho_e_substituido_por_inteiro_nunca_mesclado_campo_a_campo() {
        // No padrão, "g" é { prefix = "go" }. O usuário redefine
        // como { verb = "delete" } — deveria virar só o verbo, sem
        // nenhum traço do "prefix" original sobrevivendo junto.
        let padrao = Keymap::default_keymap();
        assert_eq!(
            padrao.visual.get(&Key::char('g')),
            Some(&Binding::Prefix("go".to_string()))
        );

        let keymap = load_keymap(&file(), Some("[visual]\ng = { verb = \"delete\" }\n"))
            .expect("deveria carregar");
        assert_eq!(
            keymap.visual.get(&Key::char('g')),
            Some(&Binding::Verb(nas_keys::Verb::Delete))
        );
    }
}
