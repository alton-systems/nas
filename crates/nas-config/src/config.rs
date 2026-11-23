use std::collections::HashMap;

use nas_keys::Layout;

use crate::spec;
use crate::value::OptionValue;

/// A configuração resolvida — o que sobra depois de ler, validar e
/// sobrepor as camadas (padrão < usuário < projeto, ADR 0007 §2).
/// `Default` é exatamente o comportamento do nas sem nenhum arquivo
/// de configuração — nunca muda silenciosamente depois que uma opção
/// é adicionada à tabela única (ADR 0007 §3, "sem arquivos,
/// comportamento idêntico ao de hoje"), porque vem do mesmo `default`
/// que `crate::spec::OPTIONS` já declara pra cada opção.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Config {
    pub keyboard_layout: Layout,
    pub tab_width: usize,
    pub indent_with_spaces: bool,
    pub line_numbers: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config::from_values(&HashMap::new())
    }
}

impl Config {
    /// Monta a configuração a partir do mapa resolvido (nome →
    /// valor) que `crate::loader` produz depois de aplicar as
    /// camadas — qualquer opção ausente do mapa cai no `default` que
    /// `crate::spec::OPTIONS` declara pra ela.
    pub(crate) fn from_values(values: &HashMap<String, OptionValue>) -> Self {
        let get = |name: &str| -> OptionValue {
            values.get(name).cloned().unwrap_or_else(|| {
                spec::find(name)
                    .expect("opção declarada em OPTIONS")
                    .default
                    .clone()
            })
        };

        let keyboard_layout = get("keyboard_layout")
            .as_str()
            .and_then(Layout::from_name)
            .unwrap_or(Layout::Qwerty);
        let tab_width = get("tab_width").as_int().unwrap_or(4) as usize;
        let indent_with_spaces = get("indent_with_spaces").as_bool().unwrap_or(false);
        let line_numbers = get("line_numbers").as_bool().unwrap_or(true);

        Config {
            keyboard_layout,
            tab_width,
            indent_with_spaces,
            line_numbers,
        }
    }
}
