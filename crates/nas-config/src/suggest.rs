//! Sugestão de "você quis dizer X?" pra uma chave desconhecida —
//! ADR 0007 §3.

/// O nome, entre `candidates`, mais parecido com `unknown` — `None`
/// se o mais parecido ainda estiver longe demais pra valer a pena
/// sugerir (mais de um terço do tamanho do próprio nome em edições).
pub(crate) fn closest<'a>(unknown: &str, candidates: &[&'a str]) -> Option<&'a str> {
    candidates
        .iter()
        .map(|candidate| (*candidate, levenshtein(unknown, candidate)))
        .min_by_key(|(_, distance)| *distance)
        .filter(|(candidate, distance)| *distance > 0 && *distance * 3 <= candidate.len().max(1))
        .map(|(candidate, _)| candidate)
}

/// Distância de edição clássica (inserir/remover/substituir um
/// caractere) — implementação direta, sem nova dependência só pra
/// isto.
fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();

    for i in 1..=a.len() {
        let mut previous_diagonal = row[0];
        row[0] = i;
        for j in 1..=b.len() {
            let above = row[j];
            row[j] = if a[i - 1] == b[j - 1] {
                previous_diagonal
            } else {
                1 + previous_diagonal.min(row[j - 1]).min(above)
            };
            previous_diagonal = above;
        }
    }
    row[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acha_o_mais_proximo_dentro_do_limite() {
        assert_eq!(
            closest("tabwidth", &["tab_width", "line_numbers"]),
            Some("tab_width")
        );
    }

    #[test]
    fn nenhuma_sugestao_quando_tudo_esta_longe() {
        assert_eq!(closest("xyz", &["tab_width", "line_numbers"]), None);
    }

    #[test]
    fn nenhuma_sugestao_pro_nome_exato() {
        assert_eq!(closest("tab_width", &["tab_width"]), None);
    }
}
