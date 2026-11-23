/// O valor resolvido de uma opção — genérico o bastante pra guardar
/// qualquer uma das opções da tabela única (`crate::spec::OPTIONS`),
/// antes de virar o campo tipado correspondente em [`crate::Config`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OptionValue {
    Bool(bool),
    Int(i64),
    Str(String),
}

impl OptionValue {
    pub(crate) fn as_bool(&self) -> Option<bool> {
        match self {
            OptionValue::Bool(value) => Some(*value),
            _ => None,
        }
    }

    pub(crate) fn as_int(&self) -> Option<i64> {
        match self {
            OptionValue::Int(value) => Some(*value),
            _ => None,
        }
    }

    pub(crate) fn as_str(&self) -> Option<&str> {
        match self {
            OptionValue::Str(value) => Some(value),
            _ => None,
        }
    }
}
