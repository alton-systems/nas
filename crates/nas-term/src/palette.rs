//! A paleta de comandos (`Espaço p` / `Shift+Espaço`, docs/keys.md
//! "paleta de comandos") — uma sobreposição de busca sobre o catálogo
//! de comandos do keymap (`nas_keys::Machine::command_catalog`), sem
//! mexer no buffer (`App::palette`, `ui::draw_palette`). Separado de
//! `app.rs` só por tamanho — a paleta não conhece `Document` nem
//! nenhum outro estado do editor, só o catálogo e a busca.
//!
//! Busca aproximada com a crate `nucleo`: `CaseMatching::Ignore` +
//! `Normalization::Smart` já cobrem "sem diferenciar acentos nem
//! maiúsculas" (`Config::DEFAULT` também liga os dois, por padrão).

use nas_keys::CatalogEntry;
use nucleo::pattern::{AtomKind, CaseMatching, Normalization, Pattern};
use nucleo::{Config, Matcher, Utf32Str};

pub struct Palette {
    pub query: String,
    pub selected: usize,
    all: Vec<CatalogEntry>,
    matcher: Matcher,
    /// Índices em `all`, na ordem de exibição atual — recalculados a
    /// cada tecla da busca (`rerank`).
    visible: Vec<usize>,
}

impl Palette {
    /// `recent` é `App::recent_commands` (mais usado recentemente
    /// primeiro) — usada já na abertura, pra que a lista inicial (busca
    /// vazia) comece ordenada por recência, não alfabética.
    pub fn new(all: Vec<CatalogEntry>, recent: &[String]) -> Self {
        let mut palette = Palette {
            all,
            matcher: Matcher::new(Config::DEFAULT),
            query: String::new(),
            selected: 0,
            visible: Vec::new(),
        };
        palette.rerank(recent);
        palette
    }

    pub fn push_char(&mut self, c: char, recent: &[String]) {
        self.query.push(c);
        self.rerank(recent);
    }

    pub fn backspace(&mut self, recent: &[String]) {
        self.query.pop();
        self.rerank(recent);
    }

    /// Move a seleção por `delta` posições (negativo = pra cima),
    /// dando a volta nas duas pontas — sem isso, `Down` na última linha
    /// ou `Up` na primeira não faria nada, o que não é o padrão usual
    /// de uma lista navegável por setas.
    pub fn move_selection(&mut self, delta: isize) {
        let len = self.visible.len() as isize;
        if len == 0 {
            return;
        }
        let next = (self.selected as isize + delta).rem_euclid(len);
        self.selected = next as usize;
    }

    pub fn selected_entry(&self) -> Option<&CatalogEntry> {
        let &idx = self.visible.get(self.selected)?;
        self.all.get(idx)
    }

    /// As entradas visíveis agora, já na ordem de exibição — pra
    /// `ui::draw_palette` desenhar a lista sem conhecer `all`/`visible`.
    pub fn visible_entries(&self) -> impl Iterator<Item = &CatalogEntry> {
        self.visible.iter().map(move |&idx| &self.all[idx])
    }

    fn rerank(&mut self, recent: &[String]) {
        self.visible = rank(&self.all, &self.query, recent, &mut self.matcher);
        self.selected = 0;
    }
}

/// A ordem de exibição: busca vazia mostra os mais usados
/// recentemente primeiro (`rank_by_recency`); busca não vazia filtra
/// pelo que bate (aproximado, no nome e na descrição) e ordena pelo
/// score, maior primeiro — empate quebrado pelo nome, pra ficar
/// determinístico mesmo quando dois comandos empatam no score.
fn rank(all: &[CatalogEntry], query: &str, recent: &[String], matcher: &mut Matcher) -> Vec<usize> {
    if query.is_empty() {
        return rank_by_recency(all, recent);
    }

    let pattern = Pattern::new(
        query,
        CaseMatching::Ignore,
        Normalization::Smart,
        AtomKind::Fuzzy,
    );
    let mut buf = Vec::new();
    let mut scored: Vec<(usize, u32)> = Vec::new();
    for (idx, entry) in all.iter().enumerate() {
        // nome e descrição no mesmo texto buscado — "busca aproximada
        // ... no nome e na descrição" (docs/keys.md).
        let haystack = format!("{} {}", entry.description, entry.name);
        if let Some(score) = pattern.score(Utf32Str::new(&haystack, &mut buf), matcher) {
            scored.push((idx, score));
        }
    }
    scored.sort_by(|&(a, score_a), &(b, score_b)| {
        score_b
            .cmp(&score_a)
            .then_with(|| all[a].name.cmp(&all[b].name))
    });
    scored.into_iter().map(|(idx, _)| idx).collect()
}

/// `recent` mais recente primeiro: cada nome aparece no máximo uma vez
/// (a primeira posição em que aparece em `recent`, que já é a mais
/// recente), seguido do resto do catálogo na ordem em que já chegou
/// (determinística — ver `Keymap::catalog`), sem repetir nenhum nome
/// já colocado na primeira parte.
fn rank_by_recency(all: &[CatalogEntry], recent: &[String]) -> Vec<usize> {
    let mut used = vec![false; all.len()];
    let mut ordered = Vec::with_capacity(all.len());
    for name in recent {
        if let Some(idx) = all.iter().position(|entry| &entry.name == name)
            && !used[idx]
        {
            used[idx] = true;
            ordered.push(idx);
        }
    }
    for (idx, was_used) in used.iter().enumerate() {
        if !was_used {
            ordered.push(idx);
        }
    }
    ordered
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, description: &str) -> CatalogEntry {
        CatalogEntry {
            name: name.to_string(),
            description: description.to_string(),
            keys: vec![nas_keys::Key::char('x')],
            ctrl: false,
            destructive: false,
        }
    }

    #[test]
    fn busca_vazia_mostra_tudo_na_ordem_do_catalogo() {
        let catalog = vec![entry("save", "salvar"), entry("quit", "sair do editor")];
        let palette = Palette::new(catalog, &[]);
        let names: Vec<&str> = palette
            .visible_entries()
            .map(|entry| entry.name.as_str())
            .collect();
        assert_eq!(names, vec!["save", "quit"]);
    }

    #[test]
    fn busca_vazia_com_recentes_mostra_eles_primeiro() {
        let catalog = vec![
            entry("save", "salvar"),
            entry("quit", "sair do editor"),
            entry("undo", "desfazer"),
        ];
        let recent = vec!["undo".to_string(), "quit".to_string()];
        let palette = Palette::new(catalog, &recent);
        let names: Vec<&str> = palette
            .visible_entries()
            .map(|entry| entry.name.as_str())
            .collect();
        assert_eq!(names, vec!["undo", "quit", "save"]);
    }

    #[test]
    fn busca_filtra_por_nome_ou_descricao_sem_acento_nem_maiuscula() {
        let catalog = vec![
            entry("checkpoint-blame", "autoria (histórico do arquivo)"),
            entry("quit", "sair do editor"),
        ];
        let mut palette = Palette::new(catalog, &[]);
        palette.push_char('H', &[]);
        palette.push_char('I', &[]);
        palette.push_char('S', &[]);
        palette.push_char('T', &[]);
        palette.push_char('O', &[]);
        palette.push_char('R', &[]);
        palette.push_char('I', &[]);
        palette.push_char('C', &[]);
        palette.push_char('O', &[]);
        let names: Vec<&str> = palette
            .visible_entries()
            .map(|entry| entry.name.as_str())
            .collect();
        assert_eq!(names, vec!["checkpoint-blame"]);
    }

    #[test]
    fn backspace_refaz_a_busca_a_partir_da_consulta_menor() {
        let catalog = vec![entry("save", "salvar"), entry("quit", "sair do editor")];
        let mut palette = Palette::new(catalog, &[]);
        for c in "savz".chars() {
            palette.push_char(c, &[]); // "savz" não bate em nada
        }
        assert!(palette.visible_entries().next().is_none());
        palette.backspace(&[]); // volta pra "sav" — só "salvar" tem um 'v'
        let names: Vec<&str> = palette
            .visible_entries()
            .map(|entry| entry.name.as_str())
            .collect();
        assert_eq!(names, vec!["save"]);
    }

    #[test]
    fn mover_selecao_da_a_volta_nas_duas_pontas() {
        let catalog = vec![entry("a", "a"), entry("b", "b"), entry("c", "c")];
        let mut palette = Palette::new(catalog, &[]);
        assert_eq!(palette.selected, 0);
        palette.move_selection(-1);
        assert_eq!(palette.selected, 2);
        palette.move_selection(1);
        assert_eq!(palette.selected, 0);
    }

    #[test]
    fn selected_entry_acompanha_a_selecao() {
        let catalog = vec![entry("a", "a"), entry("b", "b")];
        let mut palette = Palette::new(catalog, &[]);
        assert_eq!(palette.selected_entry().unwrap().name, "a");
        palette.move_selection(1);
        assert_eq!(palette.selected_entry().unwrap().name, "b");
    }
}
