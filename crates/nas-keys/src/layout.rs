//! Tradução de layout físico de teclado.
//!
//! Os atalhos do nas-keys são declarados por posição física num teclado
//! US qwerty (ver `keymap/default.toml`): `"j"` sempre significa "a
//! tecla onde um teclado qwerty tem j", não o caractere `j` em si. Num
//! layout físico diferente — o do autor é ABNT2, onde essa mesma posição
//! produz `ç` — a tecla realmente digitada precisa ser traduzida de
//! volta para o equivalente qwerty antes de consultar o keymap. Espelha
//! `nas-layout.el` (`nas-layout-diagrams`, `nas--translate-key`).

use std::collections::HashMap;

use crate::key::{Key, KeyCode};

const QWERTY: &str = "
q w e r t y u i o p
a s d f g h j k l ; '
z x c v b n m , . /

Q W E R T Y U I O P
A S D F G H J K L : \"
Z X C V B N M < > ?
";

const QWERTY_ABNT: &str = "
q w e r t y u i o p
a s d f g h j k l ç ~
z x c v b n m , . ;

Q W E R T Y U I O P
A S D F G H J K L Ç ^
Z X C V B N M < > :
";

/// Layout físico de teclado do usuário. `Qwerty` é a referência contra a
/// qual todo atalho é escrito; os demais traduzem de volta para ela.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    Qwerty,
    QwertyAbnt,
}

impl Layout {
    /// Nomes aceitos como configuração (ex.: variável de ambiente).
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "qwerty" => Some(Layout::Qwerty),
            "qwerty-abnt" => Some(Layout::QwertyAbnt),
            _ => None,
        }
    }

    fn diagram(self) -> &'static str {
        match self {
            Layout::Qwerty => QWERTY,
            Layout::QwertyAbnt => QWERTY_ABNT,
        }
    }

    /// Tabela físico → qwerty para este layout (vazia para `Qwerty`, que
    /// é a própria referência). Monte uma vez e guarde — é a tabela que
    /// [`crate::Machine`] consulta a cada tecla.
    pub fn translation_table(self) -> HashMap<char, char> {
        if self == Layout::Qwerty {
            return HashMap::new();
        }
        let qwerty_tokens = tokens(QWERTY);
        let physical_tokens = tokens(self.diagram());
        assert_eq!(
            qwerty_tokens.len(),
            physical_tokens.len(),
            "o diagrama do layout {self:?} não tem o mesmo número de teclas do qwerty"
        );
        qwerty_tokens
            .into_iter()
            .zip(physical_tokens)
            .filter(|(q, p)| q != p)
            .map(|(q, p)| (p, q))
            .collect()
    }
}

fn tokens(diagram: &str) -> Vec<char> {
    diagram
        .split_whitespace()
        .map(|tok| {
            tok.chars()
                .next()
                .expect("split_whitespace não devolve tokens vazios")
        })
        .collect()
}

/// Traduz uma tecla do layout físico para o equivalente qwerty na mesma
/// posição, usando uma tabela já montada por [`Layout::translation_table`].
/// Só o caractere é afetado — Alt e as demais teclas (RET, setas, ...)
/// não fazem parte dos diagramas de layout.
pub fn to_qwerty(table: &HashMap<char, char>, key: Key) -> Key {
    match key.code {
        KeyCode::Char(c) => {
            let translated = table.get(&c).copied().unwrap_or(c);
            Key {
                code: KeyCode::Char(translated),
                alt: key.alt,
            }
        }
        _ => key,
    }
}

/// O inverso de [`to_qwerty`]: de uma tecla canônica (qwerty) para como
/// ela aparece no layout físico do usuário — só pra **exibição** (ex.:
/// a paleta de comandos mostra "ç" em vez de ";" pra um usuário ABNT2,
/// docs/keys.md "paleta de comandos"). A máquina de estados sempre
/// consulta o keymap em qwerty canônico; essa tradução de volta nunca
/// entra na lógica de comandos, só na tela. Usa a mesma tabela
/// físico→qwerty de `to_qwerty` (a mesma que [`Layout::translation_table`]
/// monta), procurando o caractere físico cujo destino é `c` — correto
/// porque a tabela é uma bijeção entre os caracteres que diferem nos
/// dois diagramas.
pub fn to_physical(table: &HashMap<char, char>, key: Key) -> Key {
    match key.code {
        KeyCode::Char(c) => {
            let physical = table
                .iter()
                .find(|&(_, &qwerty)| qwerty == c)
                .map(|(&physical, _)| physical)
                .unwrap_or(c);
            Key {
                code: KeyCode::Char(physical),
                alt: key.alt,
            }
        }
        _ => key,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qwerty_nao_traduz_nada() {
        let table = Layout::Qwerty.translation_table();
        assert!(table.is_empty());
        assert_eq!(to_qwerty(&table, Key::char(';')), Key::char(';'));
    }

    #[test]
    fn abnt_traduz_ponto_e_virgula_pra_c_cedilha_e_volta() {
        let table = Layout::QwertyAbnt.translation_table();
        // a posição do ';' no qwerty é o 'ç' no ABNT2 — digitar 'ç' deve
        // ser entendido como o ';' canônico.
        assert_eq!(to_qwerty(&table, Key::char('ç')), Key::char(';'));
    }

    #[test]
    fn to_physical_e_o_inverso_de_to_qwerty_para_exibicao() {
        let table = Layout::QwertyAbnt.translation_table();
        // o inverso de ';' -> 'ç': exibir a tecla canônica ';' pro
        // usuário ABNT2 deveria mostrar 'ç'.
        assert_eq!(to_physical(&table, Key::char(';')), Key::char('ç'));
        // qwerty não traduz nada (tabela vazia): o inverso também não.
        let qwerty_table = Layout::Qwerty.translation_table();
        assert_eq!(to_physical(&qwerty_table, Key::char(';')), Key::char(';'));
    }

    #[test]
    fn to_physical_preserva_teclas_fora_do_diagrama_e_o_modificador_alt() {
        let table = Layout::QwertyAbnt.translation_table();
        assert_eq!(to_physical(&table, Key::enter()), Key::enter());
        assert_eq!(
            to_physical(&table, Key::with_alt(KeyCode::Char(';'))),
            Key::with_alt(KeyCode::Char('ç'))
        );
    }

    #[test]
    fn abnt_preserva_letras_iguais_nos_dois_layouts() {
        let table = Layout::QwertyAbnt.translation_table();
        for c in ['j', 'k', 'l', 'd', 'g', 'u'] {
            assert_eq!(to_qwerty(&table, Key::char(c)), Key::char(c));
        }
    }

    #[test]
    fn traducao_preserva_o_modificador_alt() {
        let table = Layout::QwertyAbnt.translation_table();
        assert_eq!(to_qwerty(&table, Key::alt_char('ç')), Key::alt_char(';'));
    }

    #[test]
    fn traducao_nao_afeta_teclas_fora_do_diagrama() {
        let table = Layout::QwertyAbnt.translation_table();
        assert_eq!(to_qwerty(&table, Key::enter()), Key::enter());
        assert_eq!(to_qwerty(&table, Key::escape()), Key::escape());
    }

    #[test]
    fn from_name_reconhece_os_dois_layouts_e_rejeita_o_resto() {
        assert_eq!(Layout::from_name("qwerty"), Some(Layout::Qwerty));
        assert_eq!(Layout::from_name("qwerty-abnt"), Some(Layout::QwertyAbnt));
        assert_eq!(Layout::from_name("dvorak"), None);
    }
}
