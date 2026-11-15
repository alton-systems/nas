//! Renderização pura da tela: não depende do backend real (crossterm), só
//! de `ratatui::Frame` — por isso pode ser testada com `TestBackend` sem
//! abrir um terminal de verdade.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::{App, PromptKind};

pub fn draw(frame: &mut Frame, app: &mut App) {
    let root = frame.area();
    let rows = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).split(root);
    let text_area = rows[0];
    let status_area = rows[1];

    let gutter_width = number_width(app.document.len_lines().max(1)) + 1;
    let cols = Layout::horizontal([Constraint::Length(gutter_width as u16), Constraint::Min(1)])
        .split(text_area);
    let gutter_area = cols[0];
    let content_area = cols[1];

    app.ensure_cursor_visible(content_area.width as usize, content_area.height as usize);

    draw_gutter(frame, gutter_area, app, gutter_width);
    draw_content(frame, content_area, app);
    draw_status(frame, status_area, app);
    if app.prompt.is_some() {
        place_prompt_cursor(frame, status_area, app);
    } else {
        place_cursor(frame, content_area, app);
    }
}

fn visible_line_range(app: &App, height: usize) -> std::ops::Range<usize> {
    let total = app.document.len_lines();
    let start = app.scroll_top.min(total);
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
    let selection = app.document.selection_range();
    let lines: Vec<Line> = visible_line_range(app, area.height as usize)
        .map(|idx| {
            let line_start = app.document.line_start_offset(idx);
            let text = app.document.line(idx).into_owned();
            line_with_selection(text, line_start, selection)
        })
        .collect();
    frame.render_widget(
        Paragraph::new(lines).scroll((0, app.scroll_left as u16)),
        area,
    );
}

fn draw_status(frame: &mut Frame, area: Rect, app: &App) {
    let style = Style::default().add_modifier(Modifier::REVERSED);

    if let Some(prompt) = &app.prompt {
        let label = match prompt.kind {
            PromptKind::OpenFile => "Abrir arquivo",
            PromptKind::Search => "Buscar",
        };
        let text = format!("{label}: {}", prompt.input);
        frame.render_widget(Paragraph::new(text).style(style), area);
        return;
    }

    if let Some(message) = &app.status_message {
        frame.render_widget(Paragraph::new(message.clone()).style(style), area);
        return;
    }

    let name = app
        .document
        .path()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "[sem nome]".to_string());
    let modified = if app.document.is_modified() {
        " [+]"
    } else {
        ""
    };
    let name = format!("[{}] {name}{modified}", app.modal.status_label());

    let position = app.document.cursor_position();
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
    let position = app.document.cursor_position();
    let visual_column = app.document.cursor_visual_column();

    let within_rows = position.line >= app.scroll_top
        && position.line - app.scroll_top < content_area.height as usize;
    let within_cols = visual_column >= app.scroll_left
        && visual_column - app.scroll_left < content_area.width as usize;

    if within_rows && within_cols {
        let x = content_area.x + (visual_column - app.scroll_left) as u16;
        let y = content_area.y + (position.line - app.scroll_top) as u16;
        frame.set_cursor_position((x, y));
    }
}

fn number_width(n: usize) -> usize {
    n.to_string().len()
}

/// Posiciona o cursor do terminal no fim do texto digitado na linha de
/// entrada, não no documento (que fica em segundo plano enquanto o
/// prompt está ativo).
fn place_prompt_cursor(frame: &mut Frame, status_area: Rect, app: &App) {
    let Some(prompt) = &app.prompt else { return };
    let label = match prompt.kind {
        PromptKind::OpenFile => "Abrir arquivo",
        PromptKind::Search => "Buscar",
    };
    let text_len = label.chars().count() + 2 + prompt.input.chars().count(); // "label: input"
    let x = status_area.x + (text_len as u16).min(status_area.width.saturating_sub(1));
    frame.set_cursor_position((x, status_area.y));
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

        let rendered = render_to_string(&mut app, 30, 5);
        insta::assert_snapshot!(rendered);
    }

    #[test]
    fn tela_com_caminho_longo_trunca_pela_esquerda() {
        let long_path =
            "/um/caminho/bastante/longo/para/testar/o/truncamento/da/barra/de/status/arquivo.txt";
        let document = Document::open(long_path).expect("abrir caminho inexistente não falha");
        let mut app = App::with_layout(document, Layout::Qwerty);

        let rendered = render_to_string(&mut app, 40, 3);
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

        let rendered = render_to_string(&mut app, 30, 3);
        insta::assert_snapshot!(rendered);
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
        let rendered = render_to_string(&mut app, 30, 3);
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
        assert_eq!(app.document.selection_range(), Some((0, 3)));

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
}
