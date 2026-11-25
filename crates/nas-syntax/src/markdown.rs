//! Títulos de Markdown como símbolos (ADR 0006 §8, etapa 3f-prep) —
//! só cabeçalhos ATX (`# título`, até `######`); não precisa de uma
//! gramática Tree-sitter pra algo tão simples de reconhecer por
//! linha.

use crate::{Symbol, SymbolKind};

pub(crate) fn list_symbols(source: &str) -> Vec<Symbol> {
    let lines: Vec<&str> = source.lines().collect();
    let headings: Vec<(usize, usize, String)> = lines
        .iter()
        .enumerate()
        .filter_map(|(idx, line)| {
            parse_atx_heading(line).map(|(level, text)| (idx + 1, level, text))
        })
        .collect();
    let total = lines.len();

    headings
        .iter()
        .enumerate()
        .map(|(i, (start_line, level, text))| {
            // O fim de uma seção é a linha anterior ao próximo
            // título do mesmo nível ou mais raso (`##` termina em
            // `##`/`#`, nunca em `###`) — ou o fim do arquivo, se
            // não houver nenhum.
            let end_line = headings[i + 1..]
                .iter()
                .find(|(_, other_level, _)| other_level <= level)
                .map(|(next_line, ..)| next_line - 1)
                .unwrap_or(total);
            Symbol {
                kind: SymbolKind::Heading,
                name: text.clone(),
                start_line: *start_line,
                end_line,
                doc_start_line: *start_line,
            }
        })
        .collect()
}

/// `(nível, texto)` se `line` for um título ATX — precisa de 1 a 6
/// `#` seguidos de espaço (ou fim de linha); `###foo` não é título.
/// Remove `#` de fechamento opcionais (`## título ##`), como a
/// especificação de Markdown permite.
fn parse_atx_heading(line: &str) -> Option<(usize, String)> {
    let trimmed = line.trim_start();
    let level = trimmed.chars().take_while(|&c| c == '#').count();
    if level == 0 || level > 6 {
        return None;
    }
    let rest = &trimmed[level..];
    if !rest.is_empty() && !rest.starts_with(' ') && !rest.starts_with('\t') {
        return None;
    }
    let text = rest.trim().trim_end_matches('#').trim().to_string();
    Some((level, text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titulos_de_niveis_diferentes_tem_secoes_aninhadas() {
        let source = "# A\ntexto a\n## B\ntexto b\n## C\ntexto c\n# D\ntexto d\n";
        let symbols = list_symbols(source);
        let names: Vec<_> = symbols.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["A", "B", "C", "D"]);

        let a = &symbols[0];
        assert_eq!((a.start_line, a.end_line), (1, 6));
        let b = &symbols[1];
        assert_eq!((b.start_line, b.end_line), (3, 4));
        let c = &symbols[2];
        assert_eq!((c.start_line, c.end_line), (5, 6));
        let d = &symbols[3];
        assert_eq!((d.start_line, d.end_line), (7, 8));
    }

    #[test]
    fn ignora_cerquilha_sem_espaco_depois() {
        let source = "#semespaço\n# com espaço\n";
        let symbols = list_symbols(source);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "com espaço");
    }
}
