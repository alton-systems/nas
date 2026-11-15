//! Renderização pura da tela: não depende do backend real (crossterm), só
//! de `ratatui::Frame` — por isso pode ser testada com `TestBackend` sem
//! abrir um terminal de verdade.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::Paragraph;

use crate::app::App;

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
    place_cursor(frame, content_area, app);
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
    let lines: Vec<Line> = visible_line_range(app, area.height as usize)
        .map(|idx| Line::from(app.document.line(idx).into_owned()))
        .collect();
    frame.render_widget(
        Paragraph::new(lines).scroll((0, app.scroll_left as u16)),
        area,
    );
}

fn draw_status(frame: &mut Frame, area: Rect, app: &App) {
    let style = Style::default().add_modifier(Modifier::REVERSED);

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;
    use nas_editor::Document;
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
        let mut app = App::new(document);

        let rendered = render_to_string(&mut app, 30, 5);
        insta::assert_snapshot!(rendered);
    }

    #[test]
    fn tela_com_caminho_longo_trunca_pela_esquerda() {
        let long_path =
            "/um/caminho/bastante/longo/para/testar/o/truncamento/da/barra/de/status/arquivo.txt";
        let document = Document::open(long_path).expect("abrir caminho inexistente não falha");
        let mut app = App::new(document);

        let rendered = render_to_string(&mut app, 40, 3);
        insta::assert_snapshot!(rendered);
    }
}
