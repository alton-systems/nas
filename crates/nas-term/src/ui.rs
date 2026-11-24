//! Renderização pura da tela: não depende do backend real (crossterm), só
//! de `ratatui::Frame` — por isso pode ser testada com `TestBackend` sem
//! abrir um terminal de verdade.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use nas_keys::{CatalogEntry, Key, ModalState, PendingHint};

use crate::app::{App, Overlay, PromptKind, ReviewPicker, buffer_name, format_author};
use crate::palette::Palette;

/// Cor das letras das opções numa dica which-key — provisória, até
/// existir um sistema de temas de verdade (issue #10, "Temas por
/// arquivo de dados").
const HINT_KEY_COLOR: Color = Color::Yellow;

/// Cor de uma opção destrutiva (ex.: `Ctrl+t z`) numa dica which-key —
/// provisória, mesma ressalva de `HINT_KEY_COLOR`.
const HINT_DESTRUCTIVE_COLOR: Color = Color::Red;

pub fn draw(frame: &mut Frame, app: &mut App) {
    let root = frame.area();

    if let Some(overlay) = &app.overlay {
        draw_overlay(frame, root, overlay);
        return;
    }

    if let Some(palette) = &app.palette {
        draw_palette(frame, root, palette);
        return;
    }

    if let Some(picker) = &app.review_picker {
        draw_review_picker(frame, root, picker);
        return;
    }

    app.expire_status_message_if_needed();

    let echo_lines = echo_area_lines(app, root.width as usize);
    let echo_height = (echo_lines.len() as u16).max(1);

    let rows = Layout::vertical([
        Constraint::Min(1),
        Constraint::Length(1),
        Constraint::Length(echo_height),
    ])
    .split(root);
    let text_area = rows[0];
    let status_area = rows[1];
    let echo_area = rows[2];

    // ADR 0007-configuracao-em-camadas.md, Parte 2: `line_numbers`
    // desliga a régua — largura zero, todo o resto do layout segue
    // igual, só sem a coluna.
    let gutter_width = if app.config().line_numbers {
        number_width(app.document().len_lines().max(1)) + 1
    } else {
        0
    };
    let cols = Layout::horizontal([Constraint::Length(gutter_width as u16), Constraint::Min(1)])
        .split(text_area);
    let gutter_area = cols[0];
    let content_area = cols[1];

    app.ensure_cursor_visible(content_area.width as usize, content_area.height as usize);

    if app.config().line_numbers {
        draw_gutter(frame, gutter_area, app, gutter_width);
    }
    draw_content(frame, content_area, app);
    draw_status(frame, status_area, app);
    draw_echo_area(frame, echo_area, &echo_lines);

    if app.prompt.is_some() {
        place_prompt_cursor(frame, echo_area, app);
    } else {
        place_cursor(frame, content_area, app);
    }
}

/// Uma visualização somente-leitura (diff, lista de checkpoints,
/// autoria) ocupa a tela inteira, com o título em destaque — ver
/// `App::overlay`, `Overlay`. `scroll` rola o texto verticalmente.
fn draw_overlay(frame: &mut Frame, area: Rect, overlay: &Overlay) {
    let mut lines = vec![Line::from(Span::styled(
        overlay.title.clone(),
        Style::default().add_modifier(Modifier::BOLD),
    ))];
    lines.extend(overlay.lines.iter().map(|line| Line::from(line.clone())));
    frame.render_widget(Paragraph::new(lines).scroll((overlay.scroll, 0)), area);
}

/// O rótulo que abre a sobreposição da paleta, antes da consulta
/// digitada — reaproveitado pra posicionar o cursor do terminal no
/// fim do texto (`draw_palette`), do mesmo jeito que `prompt_label`
/// mais `": "` faz para `place_prompt_cursor`.
const PALETTE_PROMPT: &str = "Paleta de comandos  »  ";

/// A paleta de comandos (`Espaço p`/`Shift+Espaço`, docs/keys.md
/// "paleta de comandos") ocupa a tela inteira, sem mexer no buffer —
/// mesmo princípio de `draw_overlay`, mas com uma linha de busca no
/// título (em vez de só um rótulo) e navegação por seleção em vez de
/// rolagem. Comandos sem atalho aparecem sem nada na coluna de tecla;
/// um comando destrutivo (`entry.destructive`) usa a mesma cor de
/// alerta do which-key (`HINT_DESTRUCTIVE_COLOR`) na linha inteira,
/// mais um prefixo `!` na descrição — nunca um emoji, que pode ter
/// largura ambígua num terminal.
fn draw_palette(frame: &mut Frame, area: Rect, palette: &Palette) {
    let mut lines = vec![Line::from(Span::styled(
        format!("{PALETTE_PROMPT}{}", palette.query),
        Style::default().add_modifier(Modifier::BOLD),
    ))];

    let entries: Vec<&CatalogEntry> = palette.visible_entries().collect();
    if entries.is_empty() {
        lines.push(Line::from(Span::styled(
            "nenhum comando encontrado",
            Style::default().add_modifier(Modifier::DIM),
        )));
    } else {
        // A janela visível rola o mínimo pra manter `palette.selected`
        // sempre à vista — sem isto, `Down` além da última linha que
        // cabe na tela selecionaria um item invisível, o que quebraria
        // "setas movem a seleção" (docs/keys.md) na prática assim que
        // a lista for maior que a tela. Recalculado a cada desenho a
        // partir da seleção atual — não precisa de estado próprio.
        let available_rows = (area.height as usize).saturating_sub(1).max(1);
        let start = if entries.len() <= available_rows {
            0
        } else {
            palette
                .selected
                .saturating_sub(available_rows - 1)
                .min(entries.len() - available_rows)
        };
        let end = (start + available_rows).min(entries.len());
        let visible = &entries[start..end];

        let shortcut_width = visible
            .iter()
            .map(|entry| shortcut_label(entry).chars().count())
            .max()
            .unwrap_or(0);
        for (offset, entry) in visible.iter().enumerate() {
            let selected = start + offset == palette.selected;
            let mut base = Style::default();
            if selected {
                base = base.add_modifier(Modifier::REVERSED);
            }
            let shortcut = shortcut_label(entry);
            let pad = shortcut_width.saturating_sub(shortcut.chars().count()) + 2;

            // Mesma convenção do which-key pra ação destrutiva
            // (`ui::which_key_lines`, `HINT_DESTRUCTIVE_COLOR`): a cor
            // de alerta na linha inteira, nunca um caractere à parte —
            // sem largura ambígua num terminal (regra 8). O prefixo
            // `!` reforça o aviso mesmo onde a cor não se destaca
            // (ex.: o estilo reverso da seleção).
            let (shortcut_style, text_style, description) = if entry.destructive {
                let alert = base.fg(HINT_DESTRUCTIVE_COLOR);
                (alert, alert, format!("! {}", entry.description))
            } else {
                (base.fg(HINT_KEY_COLOR), base, entry.description.clone())
            };

            let spans = vec![
                Span::styled(shortcut, shortcut_style),
                Span::styled(" ".repeat(pad), base),
                Span::styled(description, text_style),
            ];
            lines.push(Line::from(spans));
        }
    }

    frame.render_widget(Paragraph::new(lines), area);

    let cursor_x = area.x + (PALETTE_PROMPT.chars().count() + palette.query.chars().count()) as u16;
    frame.set_cursor_position((cursor_x.min(area.x + area.width.saturating_sub(1)), area.y));
}

/// `Ctrl+t V` (docs/decisions/0008-modo-de-revisao.md §4): a lista
/// selecionável de checkpoints pra escolher o "antes" da revisão —
/// mesmos dados de `Ctrl+t l` (`checkpoint_list`), mas navegável
/// (`↑`/`↓`, `Enter` confirma, `Esc` cancela) em vez de só mostrada.
fn draw_review_picker(frame: &mut Frame, area: Rect, picker: &ReviewPicker) {
    let mut lines = vec![Line::from(Span::styled(
        "Revisar — escolha um checkpoint  »  ↑/↓ escolhe, Enter confirma, Esc cancela",
        Style::default().add_modifier(Modifier::BOLD),
    ))];

    for (idx, checkpoint) in picker.checkpoints().iter().enumerate() {
        let mut style = Style::default();
        if idx == picker.selected {
            style = style.add_modifier(Modifier::REVERSED);
        }
        let kind = if checkpoint.automatic {
            "automático"
        } else {
            "nomeado"
        };
        let text = format!(
            "{}  {} ({}, {}, {} arquivo(s))",
            nas_history::format_timestamp(checkpoint.created_at),
            checkpoint.name,
            kind,
            format_author(&checkpoint.author),
            checkpoint.files.len()
        );
        lines.push(Line::from(Span::styled(text, style)));
    }

    frame.render_widget(Paragraph::new(lines), area);
}

/// O atalho de um comando do catálogo, pra exibição — vazio se o
/// comando não tiver um atalho que o alcance sozinho (ver
/// `CatalogEntry::keys`); `Ctrl+` só na primeira tecla, nunca na
/// segunda (`CatalogEntry::ctrl`, ex.: `Ctrl+t n`, nunca `Ctrl+t
/// Ctrl+n`).
fn shortcut_label(entry: &CatalogEntry) -> String {
    entry
        .keys
        .iter()
        .enumerate()
        .map(|(i, key)| {
            if i == 0 && entry.ctrl {
                format!("Ctrl+{key}")
            } else {
                key.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn visible_line_range(app: &App, height: usize) -> std::ops::Range<usize> {
    let total = app.document().len_lines();
    let start = app.scroll_top().min(total);
    let end = (start + height).min(total);
    start..end
}

fn draw_gutter(frame: &mut Frame, area: Rect, app: &App, gutter_width: usize) {
    let number_width = gutter_width - 1;
    let lines: Vec<Line> = visible_line_range(app, area.height as usize)
        .map(|idx| Line::from(format!("{:>number_width$} ", idx + 1)))
        .collect();
    frame.render_widget(
        Paragraph::new(lines).style(Style::default().add_modifier(Modifier::DIM)),
        area,
    );
}

fn draw_content(frame: &mut Frame, area: Rect, app: &App) {
    let selection = app.document().selection_range();
    let is_review = app.modal.state() == ModalState::Review;
    let lines: Vec<Line> = visible_line_range(app, area.height as usize)
        .map(|idx| {
            let text = app.document().line(idx).into_owned();
            if is_review {
                review_line(text)
            } else {
                let line_start = app.document().line_start_offset(idx);
                line_with_selection(text, line_start, selection)
            }
        })
        .collect();
    frame.render_widget(
        Paragraph::new(lines).scroll((0, app.scroll_left() as u16)),
        area,
    );
}

/// Uma linha do buffer sintético da revisão (docs/decisions/0008-modo-de-revisao.md
/// §2): `-`/`+` coloridas (vermelho/verde — o mesmo par que remover/
/// adicionar já usa no resto do nas), o cabeçalho de um trecho já
/// revisado (`[revisado]`/`[rejeitado]`) e o separador de arquivo
/// (`=== ... ===`) em destaque.
fn review_line(text: String) -> Line<'static> {
    let trimmed = text.trim_end_matches(['\n', '\r']);
    let style = if trimmed.starts_with('-') {
        Style::default().fg(Color::Red)
    } else if trimmed.starts_with('+') {
        Style::default().fg(Color::Green)
    } else if trimmed.starts_with("===")
        || trimmed.ends_with("[revisado]")
        || trimmed.ends_with("[rejeitado]")
    {
        Style::default().add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };
    Line::from(Span::styled(text, style))
}

/// Só o estado **persistente** do buffer ativo — nome, `[+]` se
/// modificado, posição do cursor. Nunca substituída por mensagem,
/// prompt ou dica — essas são passageiras, e vivem na echo area,
/// abaixo (docs/keys.md regra 8).
fn draw_status(frame: &mut Frame, area: Rect, app: &App) {
    let style = Style::default().add_modifier(Modifier::REVERSED);

    let name = buffer_name(app.document());
    let modified = if app.document().is_modified() {
        " [+]"
    } else {
        ""
    };
    let name = format!("[{}] {name}{modified}", app.modal.status_label());

    let position = app.document().cursor_position();
    let position_text = format!("{}:{}", position.line + 1, position.column + 1);
    let position_width = (position_text.chars().count() as u16).min(area.width);

    let cols =
        Layout::horizontal([Constraint::Min(0), Constraint::Length(position_width)]).split(area);
    let name_area = cols[0];
    let position_area = cols[1];

    let gap: usize = 2;
    let name_budget = (name_area.width as usize).saturating_sub(gap);
    let truncated_name = truncate_left(&name, name_budget);

    frame.render_widget(Paragraph::new(truncated_name).style(style), name_area);
    frame.render_widget(Paragraph::new(position_text).style(style), position_area);
}

/// O que a echo area mostra agora — prompt, mensagem ou dica de
/// verbo/prefixo/unidade pendente, nessa ordem de prioridade; vazia
/// (nenhuma linha) se não há nada a dizer, o que `draw` interpreta
/// como uma linha em branco (docs/keys.md regra 8). `width` é o que
/// `which_key_lines` usa pra decidir se cabe numa linha só ou precisa
/// crescer em colunas.
fn echo_area_lines(app: &App, width: usize) -> Vec<Line<'static>> {
    if let Some(prompt) = &app.prompt {
        let label = prompt_label(app, prompt.kind);
        return vec![Line::from(format!("{label}: {}", prompt.input))];
    }

    if let Some(message) = app.status_message() {
        return vec![Line::from(truncate_right(message, width))];
    }

    if let Some(prefix) = app.pending_system_prefix() {
        return which_key_lines(app.modal.system_prefix_hint(prefix), width);
    }

    if let Some(hint) = app.modal.pending_hint() {
        return which_key_lines(hint, width);
    }

    // A dica de [review] é persistente (docs/decisions/0008-modo-de-revisao.md
    // §4: "fica persistente enquanto o estado for Review") — ao
    // contrário das de cima, aparece mesmo sem nada pendente. `g h` e
    // `Esc` não vivem em `[review]` (o primeiro é global, em
    // `[prefixes.go]`; o segundo é universal), por isso entram à mão.
    if let Some(hint) = app.modal.review_hint() {
        let mut lines = which_key_lines(hint, width);
        lines.push(Line::from("g h ir até lá  ·  Esc sair da revisão"));
        return lines;
    }

    Vec::new()
}

/// Desenha a echo area já computada por `echo_area_lines` — nunca
/// decide o conteúdo, só a apresenta; `area.height` já reflete
/// `lines.len()` (calculado por `draw` antes de montar o layout). Usa
/// o fundo do editor (não o invertido da status line, acima): a echo
/// area é conteúdo passageiro, não estado fixo — ver `draw_status`.
fn draw_echo_area(frame: &mut Frame, area: Rect, lines: &[Line<'static>]) {
    frame.render_widget(Paragraph::new(lines.to_vec()), area);
}

/// Uma opção da dica, com a letra em destaque (`HINT_KEY_COLOR`) — o
/// texto plano equivalente (pra medir largura) é `{key} {description}`.
fn entry_spans(key: Key, description: &str) -> Vec<Span<'static>> {
    vec![
        Span::styled(key.to_string(), Style::default().fg(HINT_KEY_COLOR)),
        Span::raw(format!(" {description}")),
    ]
}

fn entry_text(key: Key, description: &str) -> String {
    format!("{key} {description}")
}

/// Uma dica de verbo/prefixo/unidade pendente (docs/keys.md regra 8),
/// no estilo which-key: numa linha só (`tecla  rótulo  »  tecla
/// descrição  · ...`) se couber em `width`; senão, o título isolado na
/// primeira linha e as opções em colunas nas seguintes, sem cortar
/// nenhuma — a echo area cresce temporariamente pra caber. Opções
/// destrutivas (`entry.2`) nunca entram na linha única nem nas
/// colunas: ficam isoladas, cada uma na própria linha, no fim, na cor
/// de alerta — o que também força o layout crescido mesmo quando o
/// resto caberia numa linha só.
///
/// As entradas de um prefixo de sistema já vêm na ordem declarada no
/// TOML (`hint.ordered`); as da máquina modal vêm de um `HashMap` e
/// são alfabetizadas aqui, pra não depender da ordem de iteração do
/// hash.
fn which_key_lines(mut hint: PendingHint, width: usize) -> Vec<Line<'static>> {
    if !hint.ordered {
        hint.entries.sort_by_key(|(key, _, _)| key.to_string());
    }

    let (destructive, normal): (Vec<_>, Vec<_>) =
        hint.entries.into_iter().partition(|(_, _, d)| *d);

    let title = format!("{}  {}", hint.keys, hint.label);

    if destructive.is_empty() {
        let mut spans = vec![Span::raw(format!("{title}  »  "))];
        let mut plain_width = title.chars().count() + 5;
        for (i, (key, description, _)) in normal.iter().enumerate() {
            if i > 0 {
                spans.push(Span::raw("  ·  "));
                plain_width += 5;
            }
            spans.extend(entry_spans(*key, description));
            plain_width += entry_text(*key, description).chars().count();
        }
        if plain_width <= width {
            return vec![Line::from(spans)];
        }
    }

    let mut lines = vec![Line::from(Span::styled(
        title,
        Style::default().add_modifier(Modifier::BOLD),
    ))];

    if !normal.is_empty() {
        let col_width = normal
            .iter()
            .map(|(key, description, _)| entry_text(*key, description).chars().count())
            .max()
            .unwrap_or(0)
            + 2;
        let columns = (width / col_width.max(1)).max(1);

        for row in normal.chunks(columns) {
            let mut spans = Vec::new();
            for (i, (key, description, _)) in row.iter().enumerate() {
                spans.extend(entry_spans(*key, description));
                if i + 1 < row.len() {
                    let pad =
                        col_width.saturating_sub(entry_text(*key, description).chars().count());
                    spans.push(Span::raw(" ".repeat(pad)));
                }
            }
            lines.push(Line::from(spans));
        }
    }

    for (key, description, _) in &destructive {
        lines.push(Line::from(Span::styled(
            entry_text(*key, description),
            Style::default().fg(HINT_DESTRUCTIVE_COLOR),
        )));
    }

    lines
}

/// Corta `s` pela direita para caber em `max_width` caracteres,
/// marcando o corte com "…" — ao contrário de `truncate_left` (usada
/// no nome do buffer, onde o fim do caminho é o que importa manter
/// visível), numa mensagem é o começo que importa. A versão inteira
/// continua em `App::message_history`.
fn truncate_right(s: &str, max_width: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max_width {
        return s.to_string();
    }
    if max_width == 0 {
        return String::new();
    }
    if max_width == 1 {
        return "…".to_string();
    }
    let keep = max_width - 1;
    chars[..keep].iter().collect::<String>() + "…"
}

/// Monta a linha destacando a parte que cai dentro da seleção ativa
/// (`line_start` é a posição absoluta, em caracteres, de onde essa
/// linha começa no documento).
fn line_with_selection(
    text: String,
    line_start: usize,
    selection: Option<(usize, usize)>,
) -> Line<'static> {
    let Some((sel_start, sel_end)) = selection else {
        return Line::from(text);
    };
    let line_len = text.chars().count();
    let line_end = line_start + line_len;
    if sel_end <= line_start || sel_start >= line_end {
        return Line::from(text);
    }

    let local_start = sel_start.saturating_sub(line_start).min(line_len);
    let local_end = sel_end.saturating_sub(line_start).min(line_len);

    let chars: Vec<char> = text.chars().collect();
    let before: String = chars[..local_start].iter().collect();
    let selected: String = chars[local_start..local_end].iter().collect();
    let after: String = chars[local_end..].iter().collect();

    let highlight = Style::default().add_modifier(Modifier::REVERSED);
    let mut spans = Vec::new();
    if !before.is_empty() {
        spans.push(Span::raw(before));
    }
    if !selected.is_empty() {
        spans.push(Span::styled(selected, highlight));
    }
    if !after.is_empty() {
        spans.push(Span::raw(after));
    }
    Line::from(spans)
}

/// Corta `s` pela esquerda para caber em `max_width` caracteres, marcando
/// o corte com "…". Usa contagem de caracteres, não largura visual — o
/// caminho de um arquivo é, na prática, quase sempre ASCII.
fn truncate_left(s: &str, max_width: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max_width {
        return s.to_string();
    }
    if max_width == 0 {
        return String::new();
    }
    if max_width == 1 {
        return "…".to_string();
    }
    let start = chars.len() - (max_width - 1);
    std::iter::once('…')
        .chain(chars[start..].iter().copied())
        .collect()
}

fn place_cursor(frame: &mut Frame, content_area: Rect, app: &App) {
    let position = app.document().cursor_position();
    let visual_column = app.document().cursor_visual_column();
    let scroll_top = app.scroll_top();
    let scroll_left = app.scroll_left();

    let within_rows =
        position.line >= scroll_top && position.line - scroll_top < content_area.height as usize;
    let within_cols =
        visual_column >= scroll_left && visual_column - scroll_left < content_area.width as usize;

    if within_rows && within_cols {
        let x = content_area.x + (visual_column - scroll_left) as u16;
        let y = content_area.y + (position.line - scroll_top) as u16;
        frame.set_cursor_position((x, y));
    }
}

fn number_width(n: usize) -> usize {
    n.to_string().len()
}

/// Posiciona o cursor do terminal no fim do texto digitado na linha de
/// entrada, na echo area — não no documento, que fica em segundo
/// plano enquanto o prompt está ativo. Um prompt sempre ocupa só a
/// primeira linha da echo area (nunca cresce como a dica which-key),
/// então `echo_area.y` é sempre a linha certa.
fn place_prompt_cursor(frame: &mut Frame, echo_area: Rect, app: &App) {
    let Some(prompt) = &app.prompt else { return };
    let label = prompt_label(app, prompt.kind);
    let text_len = label.chars().count() + 2 + prompt.input.chars().count(); // "label: input"
    let x = echo_area.x + (text_len as u16).min(echo_area.width.saturating_sub(1));
    frame.set_cursor_position((x, echo_area.y));
}

/// Rótulo exibido na echo area enquanto um prompt está ativo. O
/// de trocar de buffer lista os buffers abertos, como pedido na etapa
/// 5c — é a única informação que o usuário precisa pra saber o que
/// digitar.
fn prompt_label(app: &App, kind: PromptKind) -> String {
    match kind {
        PromptKind::OpenFile => "Abrir arquivo".to_string(),
        PromptKind::Search => "Buscar".to_string(),
        PromptKind::GotoLine => "Ir para linha".to_string(),
        PromptKind::SwitchBuffer => {
            let names: Vec<String> = app
                .buffers
                .iter()
                .map(|buffer| buffer_name(&buffer.document))
                .collect();
            format!("Trocar buffer ({})", names.join(", "))
        }
        PromptKind::CheckpointName => "Nome do checkpoint".to_string(),
        PromptKind::RestoreCheckpoint => "Restaurar checkpoint (nome ou id)".to_string(),
        PromptKind::DeleteHistoryConfirm => {
            "Apagar todo o histórico do projeto — digite \"apagar\" para confirmar".to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;
    use nas_editor::Document;
    use nas_keys::Layout;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn render_to_string(app: &mut App, width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).expect("terminal de teste");
        terminal.draw(|frame| draw(frame, app)).expect("desenhar");

        let buffer = terminal.backend().buffer();
        let mut rendered = String::new();
        for y in 0..height {
            for x in 0..width {
                rendered.push_str(buffer[(x, y)].symbol());
            }
            rendered.push('\n');
        }
        rendered
    }

    #[test]
    fn tela_com_acentos_emoji_e_cjk() {
        let mut document = Document::empty();
        for ch in "café 🙂日本語".chars() {
            document.insert_char(ch);
        }
        let mut app = App::with_layout(document, Layout::Qwerty);

        let rendered = render_to_string(&mut app, 30, 6);
        insta::assert_snapshot!(rendered);
    }

    #[test]
    fn tela_com_caminho_longo_trunca_pela_esquerda() {
        let long_path =
            "/um/caminho/bastante/longo/para/testar/o/truncamento/da/barra/de/status/arquivo.txt";
        let document = Document::open(long_path).expect("abrir caminho inexistente não falha");
        let mut app = App::with_layout(document, Layout::Qwerty);

        let rendered = render_to_string(&mut app, 40, 4);
        insta::assert_snapshot!(rendered);
    }

    #[test]
    fn tela_com_prompt_de_busca_ativo() {
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let mut document = Document::empty();
        for ch in "ola mundo".chars() {
            document.insert_char(ch);
        }
        let mut app = App::with_layout(document, Layout::Qwerty);

        app.handle_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE));
        for c in "mun".chars() {
            app.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        }

        let rendered = render_to_string(&mut app, 30, 4);
        insta::assert_snapshot!(rendered);
    }

    #[test]
    fn tela_com_dica_de_verbo_pendente() {
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let mut document = Document::empty();
        for ch in "ola mundo".chars() {
            document.insert_char(ch);
        }
        let mut app = App::with_layout(document, Layout::Qwerty);

        app.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE));

        let rendered = render_to_string(&mut app, 60, 14);
        insta::assert_snapshot!(rendered);
    }

    #[test]
    fn tela_com_dica_which_key_do_ctrl_t_todas_as_nove_opcoes_visiveis() {
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let mut document = Document::empty();
        for ch in "ola mundo".chars() {
            document.insert_char(ch);
        }
        let mut app = App::with_layout(document, Layout::Qwerty);

        app.handle_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::CONTROL));

        let rendered = render_to_string(&mut app, 80, 12);
        insta::assert_snapshot!(rendered);
    }

    #[test]
    fn tela_com_mensagem_longa_cortada() {
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let mut document = Document::empty();
        for ch in "ola mundo".chars() {
            document.insert_char(ch);
        }
        let mut app = App::with_layout(document, Layout::Qwerty);

        // Ctrl+b com uma consulta bem longa: "nenhum buffer encontrado
        // para <consulta>" fica maior que qualquer terminal razoável.
        app.handle_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL));
        let long_query = "x".repeat(80);
        for c in long_query.chars() {
            app.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        }
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

        let full_message = app
            .status_message()
            .expect("deveria ter uma mensagem")
            .to_string();
        assert!(
            full_message.len() > 40,
            "a mensagem precisa ser mais longa que a largura do terminal do teste"
        );

        let rendered = render_to_string(&mut app, 40, 4);
        insta::assert_snapshot!(rendered);

        assert_eq!(
            app.message_history().last(),
            Some(&full_message),
            "a mensagem inteira (não cortada) deveria estar no histórico"
        );
    }

    #[test]
    fn tela_com_selecao_ativa() {
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let mut document = Document::empty();
        for ch in "ola mundo".chars() {
            document.insert_char(ch);
        }
        let mut app = App::with_layout(document, Layout::Qwerty);

        app.handle_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE));
        for _ in 0..3 {
            app.handle_key(KeyEvent::new(KeyCode::Char(';'), KeyModifiers::NONE));
        }

        // o texto é o mesmo com ou sem seleção — o destaque (estilo
        // reverso) não aparece no dump de caracteres; ver o teste
        // seguinte, que confere o estilo de cada célula.
        let rendered = render_to_string(&mut app, 30, 4);
        insta::assert_snapshot!(rendered);
    }

    #[test]
    fn selecao_aplica_estilo_reverso_so_no_trecho_selecionado() {
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        use ratatui::style::Modifier as StyleModifier;

        let mut document = Document::empty();
        for ch in "ola mundo".chars() {
            document.insert_char(ch);
        }
        let mut app = App::with_layout(document, Layout::Qwerty);

        app.handle_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE));
        for _ in 0..3 {
            app.handle_key(KeyEvent::new(KeyCode::Char(';'), KeyModifiers::NONE));
        }
        assert_eq!(app.document().selection_range(), Some((0, 3)));

        let backend = TestBackend::new(30, 3);
        let mut terminal = Terminal::new(backend).expect("terminal de teste");
        terminal
            .draw(|frame| draw(frame, &mut app))
            .expect("desenhar");
        let buffer = terminal.backend().buffer();

        let gutter_width = 2; // "1 " para um documento de 1 linha
        for col in 0..9usize {
            let is_selected = col < 3; // "ola" são as 3 primeiras colunas
            let cell = &buffer[((gutter_width + col) as u16, 0)];
            assert_eq!(
                cell.modifier.contains(StyleModifier::REVERSED),
                is_selected,
                "coluna {col} ({:?}) deveria{} estar destacada",
                cell.symbol(),
                if is_selected { "" } else { " não" }
            );
        }
    }

    #[test]
    fn line_numbers_desligado_remove_a_coluna_da_regua() {
        let mut document = Document::empty();
        for ch in "ola".chars() {
            document.insert_char(ch);
        }
        let mut app = App::with_layout(document, Layout::Qwerty);
        app.attach_config(
            nas_config::Config {
                line_numbers: false,
                ..nas_config::Config::default()
            },
            None,
            None,
        );

        let backend = TestBackend::new(30, 3);
        let mut terminal = Terminal::new(backend).expect("terminal de teste");
        terminal
            .draw(|frame| draw(frame, &mut app))
            .expect("desenhar");
        let buffer = terminal.backend().buffer();

        // sem régua, "ola" começa direto na coluna 0.
        let line: String = (0..3u16)
            .map(|col| buffer[(col, 0)].symbol().chars().next().unwrap_or(' '))
            .collect();
        assert_eq!(line, "ola");
    }

    #[test]
    fn paleta_rola_a_lista_pra_manter_a_selecao_visivel() {
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        app.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::SHIFT));

        // terminal baixo o bastante pra caber bem menos que o
        // catálogo inteiro (dezenas de comandos) — move a seleção além
        // do que a primeira janela mostraria.
        for _ in 0..15 {
            app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        }
        let selected_name = app
            .palette
            .as_ref()
            .unwrap()
            .selected_entry()
            .unwrap()
            .description
            .clone();

        let rendered = render_to_string(&mut app, 50, 6);
        assert!(
            rendered.contains(&selected_name),
            "a linha selecionada ({selected_name:?}) deveria estar na janela visível:\n{rendered}"
        );
    }

    #[test]
    fn tela_com_paleta_de_comandos_vazia() {
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        app.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::SHIFT));

        let rendered = render_to_string(&mut app, 50, 10);
        insta::assert_snapshot!(rendered);
    }

    #[test]
    fn tela_com_paleta_de_comandos_com_busca() {
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        app.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::SHIFT));
        for c in "linha".chars() {
            app.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        }

        let rendered = render_to_string(&mut app, 50, 6);
        insta::assert_snapshot!(rendered);
    }

    #[test]
    fn tela_com_paleta_de_comandos_com_marcador_destrutivo() {
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let mut app = App::with_layout(Document::empty(), Layout::Qwerty);
        app.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::SHIFT));
        // "apagar o histórico do projeto" (Ctrl+t z) é o único comando
        // destrutivo do keymap padrão — busca restrita a ele.
        for c in "apagar historico".chars() {
            app.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        }

        let rendered = render_to_string(&mut app, 60, 6);
        insta::assert_snapshot!(rendered);
    }

    #[test]
    fn tela_com_paleta_de_comandos_no_layout_abnt2() {
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        // move-right é "j k l ;" -> ";" no qwerty, "ç" no ABNT2
        // (docs/keys.md, "paleta de comandos": "no ABNT2, ç e não ;").
        let mut app = App::with_layout(Document::empty(), Layout::QwertyAbnt);
        app.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::SHIFT));
        for c in "proximo caractere".chars() {
            app.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        }

        let rendered = render_to_string(&mut app, 50, 6);
        insta::assert_snapshot!(rendered);
    }
}
