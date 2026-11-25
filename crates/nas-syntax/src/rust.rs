//! Extração de símbolos de um arquivo Rust via `tree-sitter-rust`,
//! executado sobre o runtime `tree-house-bindings` em vez da crate
//! oficial `tree-sitter` — ver o módulo deste crate (`lib.rs`) e a
//! ADR (§8, etapa 3f-prep) pelo porquê: a crate oficial vendora sua
//! própria cópia do runtime C do tree-sitter, que colide (símbolos C
//! duplicados no link do binário `nas`) com a cópia que o
//! `helix-core` já traz embutida via `tree-house-bindings`
//! (decisão 0001). Usando o mesmo `tree-house-bindings`, o Cargo
//! unifica numa única cópia compilada — nenhuma duplicação.
//!
//! Só olha pros itens de **topo** do arquivo, mais os métodos dentro
//! de cada `impl` — não desce dentro de `mod`/`trait` (um `mod` é um
//! símbolo só, não uma lista de símbolos aninhados; os métodos de um
//! `trait` são assinaturas sem corpo, fora do escopo desta etapa).

use std::ops::Range;

use tree_house_bindings::{Grammar, Input, Node, Parser};

use crate::{Symbol, SymbolKind};

/// Um `Input` de `tree-house-bindings` sobre um `&str` contíguo — o
/// caso mais simples possível do trait (um só "chunk", sem fronteira
/// nenhuma pra caminhar): `cursor_at` ignora o deslocamento pedido e
/// sempre devolve o mesmo `&str` (cujo próprio `impl Cursor for &str`,
/// de `regex-cursor`, já devolve `chunk()` = o texto inteiro e
/// `offset()` = 0 — exatamente o que faz a leitura do parser acertar
/// o deslocamento certo sem nenhuma lógica extra aqui).
struct StrInput<'a> {
    text: &'a str,
}

impl<'a> Input for StrInput<'a> {
    type Cursor = &'a str;

    fn cursor_at(&mut self, _offset: u32) -> &mut &'a str {
        &mut self.text
    }

    fn eq(&mut self, range1: Range<u32>, range2: Range<u32>) -> bool {
        let bytes = self.text.as_bytes();
        bytes[range1.start as usize..range1.end as usize]
            == bytes[range2.start as usize..range2.end as usize]
    }
}

pub(crate) fn list_symbols(source: &str) -> Vec<Symbol> {
    let grammar: Grammar = tree_sitter_rust::LANGUAGE.try_into().expect(
        "a gramática Rust embutida é incompatível com o runtime embutido (ABI) — ver ADR 0006 §8",
    );
    let mut parser = Parser::new();
    parser
        .set_grammar(grammar)
        .expect("checado acima: a gramática já foi validada como compatível");
    let Some(tree) = parser.parse(StrInput { text: source }, None) else {
        return Vec::new();
    };

    let mut symbols = Vec::new();
    let root = tree.root_node();
    let mut cursor = root.walk();
    if cursor.goto_first_child() {
        loop {
            visit_top_level(cursor.node(), source, &mut symbols);
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
    symbols
}

/// Como `Node::child_by_field_name` da crate oficial `tree-sitter` —
/// `tree-house-bindings` só expõe o nome do campo via `TreeCursor`,
/// não um método direto no `Node`, então caminhamos os filhos à
/// procura do primeiro com este campo.
fn child_by_field_name<'tree>(node: &Node<'tree>, field: &str) -> Option<Node<'tree>> {
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            if cursor.field_name() == Some(field) {
                return Some(cursor.node());
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
    None
}

fn visit_top_level(node: Node, source: &str, out: &mut Vec<Symbol>) {
    match node.kind() {
        "function_item" => push_named(node, source, SymbolKind::Fn, out),
        "struct_item" => push_named(node, source, SymbolKind::Struct, out),
        "enum_item" => push_named(node, source, SymbolKind::Enum, out),
        "trait_item" => push_named(node, source, SymbolKind::Trait, out),
        "const_item" => push_named(node, source, SymbolKind::Const, out),
        "mod_item" => push_named(node, source, SymbolKind::Mod, out),
        "impl_item" => push_impl(node, source, out),
        _ => {}
    }
}

fn push_named(node: Node, source: &str, kind: SymbolKind, out: &mut Vec<Symbol>) {
    let Some(name_node) = child_by_field_name(&node, "name") else {
        return;
    };
    out.push(symbol_for(&node, source, kind, text_of(&name_node, source)));
}

/// O `impl` em si conta como um símbolo (nome = `Tipo`, ou `Trait for
/// Tipo` quando é um impl de trait) — e cada `fn` direta dentro do
/// corpo dele é um método, qualificado `Tipo::nome` (só o tipo, nunca
/// a trait, na qualificação — ADR 0006 §8).
fn push_impl(node: Node, source: &str, out: &mut Vec<Symbol>) {
    let Some(type_node) = child_by_field_name(&node, "type") else {
        return;
    };
    let type_text = text_of(&type_node, source);

    let impl_name = match child_by_field_name(&node, "trait") {
        Some(trait_node) => format!("{} for {type_text}", text_of(&trait_node, source)),
        None => type_text.clone(),
    };
    out.push(symbol_for(&node, source, SymbolKind::Impl, impl_name));

    let Some(body) = child_by_field_name(&node, "body") else {
        return;
    };
    let mut cursor = body.walk();
    if cursor.goto_first_child() {
        loop {
            let child = cursor.node();
            if child.kind() == "function_item"
                && let Some(name_node) = child_by_field_name(&child, "name")
            {
                let qualified = format!("{type_text}::{}", text_of(&name_node, source));
                out.push(symbol_for(&child, source, SymbolKind::Method, qualified));
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
}

fn text_of(node: &Node, source: &str) -> String {
    source[node.start_byte() as usize..node.end_byte() as usize].to_string()
}

fn symbol_for(node: &Node, source: &str, kind: SymbolKind, name: String) -> Symbol {
    let start_line = line_for_byte(source, node.start_byte());
    let end_line = line_for_byte(
        source,
        node.end_byte().saturating_sub(1).max(node.start_byte()),
    );
    Symbol {
        kind,
        name,
        start_line,
        end_line,
        doc_start_line: leading_doc_start(source, start_line),
    }
}

/// A linha (1-indexada) que contém o byte `offset` de `source`. `\n`
/// nunca faz parte de uma sequência UTF-8 multibyte, então contar
/// bytes (em vez de caracteres) é seguro mesmo com acentos no meio da
/// linha.
fn line_for_byte(source: &str, offset: u32) -> usize {
    1 + source.as_bytes()[..offset as usize]
        .iter()
        .filter(|&&b| b == b'\n')
        .count()
}

/// Sobe a partir de `item_start_line` (1-indexada) enquanto a linha
/// imediatamente anterior for um doc comment (`///`, `//!`) ou um
/// atributo (`#[...]`, `#![...]`) — sem atravessar nenhuma linha em
/// branco (um comentário solto antes de uma linha vazia não faz
/// parte do item). Devolve a primeira linha dessa cadeia contígua, ou
/// `item_start_line` se não houver nenhuma — é o que faz
/// `read_symbol` devolver "doc comments e atributos logo acima"
/// (ADR 0006 §8) sem depender da árvore de sintaxe pra isso: no
/// `tree-sitter-rust`, comentários e atributos são nós **irmãos**
/// anteriores ao item, nunca filhos dele — por isso um escaneamento
/// de texto simples é mais direto que caminhar a árvore pra trás.
fn leading_doc_start(source: &str, item_start_line: usize) -> usize {
    let lines: Vec<&str> = source.lines().collect();
    let mut line = item_start_line;
    while line > 1 {
        let above = lines.get(line - 2).copied().unwrap_or("").trim_start();
        if above.starts_with("///")
            || above.starts_with("//!")
            || above.starts_with("#[")
            || above.starts_with("#![")
        {
            line -= 1;
        } else {
            break;
        }
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
/// Doc do Foo
#[derive(Debug)]
pub struct Foo {
    x: i32,
}

impl Foo {
    /// Construtor
    pub fn new() -> Self {
        Foo { x: 0 }
    }

    fn bar(&self) -> i32 {
        self.x
    }
}

impl Default for Foo {
    fn default() -> Self {
        Foo::new()
    }
}

pub trait Greet {
    fn greet(&self);
}

pub enum Color {
    Red,
    Green,
}

pub const MAX: i32 = 10;

pub mod tests_internos {
    fn inner() {}
}

pub fn main_fn() {}
"#;

    fn find<'a>(symbols: &'a [Symbol], name: &str) -> &'a Symbol {
        symbols
            .iter()
            .find(|s| s.name == name)
            .unwrap_or_else(|| panic!("símbolo {name:?} não encontrado: {symbols:?}"))
    }

    #[test]
    fn lista_structs_enums_traits_consts_mods_e_funcoes_de_topo() {
        let symbols = list_symbols(SAMPLE);

        assert_eq!(find(&symbols, "Foo").kind, SymbolKind::Struct);
        assert_eq!(find(&symbols, "Greet").kind, SymbolKind::Trait);
        assert_eq!(find(&symbols, "Color").kind, SymbolKind::Enum);
        assert_eq!(find(&symbols, "MAX").kind, SymbolKind::Const);
        assert_eq!(find(&symbols, "tests_internos").kind, SymbolKind::Mod);
        assert_eq!(find(&symbols, "main_fn").kind, SymbolKind::Fn);
    }

    #[test]
    fn metodos_de_impl_vem_qualificados_com_o_tipo() {
        let symbols = list_symbols(SAMPLE);

        let new_method = find(&symbols, "Foo::new");
        assert_eq!(new_method.kind, SymbolKind::Method);
        let bar_method = find(&symbols, "Foo::bar");
        assert_eq!(bar_method.kind, SymbolKind::Method);
    }

    #[test]
    fn impl_de_trait_nomeia_como_trait_for_tipo() {
        let symbols = list_symbols(SAMPLE);

        let impl_symbol = find(&symbols, "Default for Foo");
        assert_eq!(impl_symbol.kind, SymbolKind::Impl);
        assert_eq!(find(&symbols, "Foo::default").kind, SymbolKind::Method);
    }

    #[test]
    fn nao_desce_dentro_de_mod_nem_de_trait() {
        let symbols = list_symbols(SAMPLE);

        assert!(!symbols.iter().any(|s| s.name == "inner"));
        assert!(!symbols.iter().any(|s| s.name == "greet"));
    }

    #[test]
    fn doc_comment_e_atributo_entram_em_doc_start_line_mas_nao_em_start_line() {
        let symbols = list_symbols(SAMPLE);
        let foo = find(&symbols, "Foo");

        // linha 1 do SAMPLE é vazia (string bruta começa com \n);
        // "/// Doc do Foo" é a linha 2, "#[derive(Debug)]" a 3, e
        // "pub struct Foo {" a 4.
        assert_eq!(foo.start_line, 4);
        assert_eq!(foo.doc_start_line, 2);
    }
}
