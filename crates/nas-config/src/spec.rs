//! A tabela única de opções de `config.toml` — nome, padrão e se
//! vale no projeto, tudo num lugar só (ADR 0007, Parte 2, "cada
//! opção declara, numa tabela única..."). Adicionar uma opção nova é
//! adicionar uma linha aqui — nunca espalhar `match`es por outros
//! módulos, mesmo princípio de dados declarativos que `nas-keys` já
//! segue pro keymap (ADR 0003).
//!
//! As três opções abaixo (além de `keyboard_layout`, que já existe
//! via `NAS_KEYBOARD_LAYOUT`) tornam configurável um comportamento
//! que o nas **já tem**, hoje, com um valor fixo — nunca uma
//! funcionalidade nova: `Tab` já insere algo (hoje sempre um
//! caractere de tabulação); `indent_with_spaces`/`tab_width` só
//! parametrizam o quê. A régua de linha já aparece sempre;
//! `line_numbers` só deixa desligar.

use crate::value::OptionValue;
use toml_edit::Item;

/// Uma opção de `config.toml`: nome, valor padrão (o que o nas já
/// faz sem nenhum arquivo), se `.nas/config.toml` (projeto) pode
/// defini-la, e como validar/converter o item TOML bruto.
pub(crate) struct OptionSpec {
    pub name: &'static str,
    pub default: OptionValue,
    pub project_allowed: bool,
    pub parse: fn(&Item) -> Result<OptionValue, String>,
}

/// A tabela única — uma função, não uma `const`, porque o valor
/// padrão de `keyboard_layout` precisa de uma `String` alocada (TOML
/// não distingue `&'static str` de `String`; `OptionValue` só tem um
/// jeito de guardar texto). O custo de montar isto de novo a cada
/// chamada é irrelevante — configuração carrega uma vez na abertura
/// do nas, nunca em laço.
pub(crate) fn options() -> Vec<OptionSpec> {
    vec![
        OptionSpec {
            name: "keyboard_layout",
            default: OptionValue::Str("qwerty".to_string()),
            project_allowed: false,
            parse: parse_keyboard_layout,
        },
        OptionSpec {
            name: "tab_width",
            default: OptionValue::Int(4),
            project_allowed: true,
            parse: parse_tab_width,
        },
        OptionSpec {
            name: "indent_with_spaces",
            default: OptionValue::Bool(false),
            project_allowed: true,
            parse: parse_bool,
        },
        OptionSpec {
            name: "line_numbers",
            default: OptionValue::Bool(true),
            project_allowed: false,
            parse: parse_bool,
        },
    ]
}

pub(crate) fn names() -> Vec<&'static str> {
    options().iter().map(|spec| spec.name).collect()
}

pub(crate) fn find(name: &str) -> Option<OptionSpec> {
    options().into_iter().find(|spec| spec.name == name)
}

fn parse_bool(item: &Item) -> Result<OptionValue, String> {
    item.as_bool()
        .map(OptionValue::Bool)
        .ok_or_else(|| "esperava um booleano (true ou false)".to_string())
}

fn parse_tab_width(item: &Item) -> Result<OptionValue, String> {
    let value = item
        .as_integer()
        .ok_or_else(|| "esperava um número inteiro".to_string())?;
    if !(1..=16).contains(&value) {
        return Err(format!("esperava um número entre 1 e 16, recebeu {value}"));
    }
    Ok(OptionValue::Int(value))
}

fn parse_keyboard_layout(item: &Item) -> Result<OptionValue, String> {
    let value = item
        .as_str()
        .ok_or_else(|| "esperava um texto".to_string())?;
    if nas_keys::Layout::from_name(value).is_none() {
        return Err(format!(
            "layout de teclado desconhecido {value:?} — use \"qwerty\" ou \"qwerty-abnt\""
        ));
    }
    Ok(OptionValue::Str(value.to_string()))
}
