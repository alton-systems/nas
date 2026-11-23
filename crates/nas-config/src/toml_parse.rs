//! Lê e valida um `config.toml` (de usuário ou de projeto) — ADR
//! 0007 §3/§4: toda chave e todo valor são checados contra
//! `crate::spec::OPTIONS` antes de qualquer coisa ser aplicada;
//! `.nas/config.toml` (projeto) recusa qualquer opção que não seja
//! `project_allowed`.

use std::collections::HashMap;
use std::path::Path;

use toml_edit::ImDocument;

use crate::error::{ConfigError, line_col};
use crate::spec;
use crate::suggest;
use crate::value::OptionValue;

/// `is_project`: `true` pra `.nas/config.toml` — recusa qualquer
/// opção que não seja `project_allowed` (ADR 0007 §4), em vez de
/// ignorá-la em silêncio.
///
/// Usa `ImDocument`, não `DocumentMut`: só o documento imutável
/// preserva o `span()` (byte-offset) de cada chave/valor — virar
/// `DocumentMut` (editável) descarta os spans de propósito, já que
/// editar invalidaria a maioria deles. Aqui só lemos, nunca editamos,
/// então o imutável é o certo, e é o único que dá linha/coluna de
/// verdade pro erro (ADR 0007 §3).
pub(crate) fn parse_config_toml(
    file: &Path,
    source: &str,
    is_project: bool,
) -> Result<HashMap<String, OptionValue>, Vec<ConfigError>> {
    let doc: ImDocument<String> = source.parse().map_err(|err: toml_edit::TomlError| {
        let (line, column) = err
            .span()
            .map(|span| line_col(source, span.start))
            .unwrap_or((1, 1));
        vec![ConfigError::at(
            file,
            line,
            column,
            format!("TOML inválido: {}", err.message()),
        )]
    })?;

    let mut errors = Vec::new();
    let mut values = HashMap::new();
    let known_names = spec::names();

    for (key, item) in doc.iter() {
        let key_span = doc.get_key_value(key).and_then(|(k, _)| k.span());
        let (line, column) = key_span
            .map(|span| line_col(source, span.start))
            .unwrap_or((1, 1));

        let Some(option) = spec::find(key) else {
            let mut error =
                ConfigError::at(file, line, column, format!("chave desconhecida: {key:?}"));
            if let Some(suggestion) = suggest::closest(key, &known_names) {
                error = error.with_suggestion(suggestion);
            }
            errors.push(error);
            continue;
        };

        if is_project && !option.project_allowed {
            errors.push(ConfigError::at(
                file,
                line,
                column,
                format!("a opção {key:?} só vale na configuração do usuário, não no projeto"),
            ));
            continue;
        }

        match (option.parse)(item) {
            Ok(value) => {
                values.insert(key.to_string(), value);
            }
            Err(message) => {
                let (vline, vcolumn) = item
                    .span()
                    .map(|span| line_col(source, span.start))
                    .unwrap_or((line, column));
                errors.push(ConfigError::at(
                    file,
                    vline,
                    vcolumn,
                    format!("{key}: {message}"),
                ));
            }
        }
    }

    if errors.is_empty() {
        Ok(values)
    } else {
        Err(errors)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn file() -> PathBuf {
        PathBuf::from("config.toml")
    }

    #[test]
    fn aceita_as_opcoes_validas() {
        let values =
            parse_config_toml(&file(), "tab_width = 2\nindent_with_spaces = true\n", false)
                .expect("deveria validar");
        assert_eq!(values.get("tab_width"), Some(&OptionValue::Int(2)));
        assert_eq!(
            values.get("indent_with_spaces"),
            Some(&OptionValue::Bool(true))
        );
    }

    #[test]
    fn chave_desconhecida_sugere_a_mais_parecida() {
        let errors = parse_config_toml(&file(), "tabwidth = 2\n", false).unwrap_err();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].line, Some(1));
        assert_eq!(errors[0].suggestion.as_deref(), Some("tab_width"));
    }

    #[test]
    fn valor_invalido_aponta_linha_e_coluna_certas() {
        let errors =
            parse_config_toml(&file(), "line_numbers = true\ntab_width = 99\n", false).unwrap_err();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].line, Some(2));
    }

    #[test]
    fn opcao_proibida_no_projeto_e_recusada() {
        let errors = parse_config_toml(&file(), "line_numbers = false\n", true).unwrap_err();
        assert_eq!(errors.len(), 1);
        assert!(
            errors[0]
                .message
                .contains("só vale na configuração do usuário")
        );
    }

    #[test]
    fn opcao_permitida_no_projeto_passa() {
        let values = parse_config_toml(&file(), "tab_width = 2\n", true).expect("deveria validar");
        assert_eq!(values.get("tab_width"), Some(&OptionValue::Int(2)));
    }

    #[test]
    fn toml_malformado_reporta_posicao() {
        let errors = parse_config_toml(&file(), "tab_width = [\n", false).unwrap_err();
        assert_eq!(errors.len(), 1);
        assert!(errors[0].line.is_some());
    }

    #[test]
    fn coleta_todos_os_erros_de_uma_vez() {
        let errors = parse_config_toml(&file(), "tabwidth = 2\nline_numbers = \"sim\"\n", false)
            .unwrap_err();
        assert_eq!(errors.len(), 2);
    }
}
