use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::style::{Color, Style};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::{DefaultTerminal, Frame};

// Nord palette (see PLAN.md §3 / about.md): cyan = active/focused.
const NORD_CYAN: Color = Color::Rgb(0x88, 0xc0, 0xd0);

fn main() -> anyhow::Result<()> {
    let mut terminal = ratatui::init();
    let result = run(&mut terminal);
    ratatui::restore();
    result
}

fn run(terminal: &mut DefaultTerminal) -> anyhow::Result<()> {
    loop {
        terminal.draw(render)?;
        if let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
            && key.code == KeyCode::Char('q')
        {
            break;
        }
    }
    Ok(())
}

fn render(frame: &mut Frame) {
    let block = Block::default()
        .title(" lazytickets ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(NORD_CYAN));
    let body = Paragraph::new("lazytickets — M0 skeleton\n\nPress q to quit.").block(block);
    frame.render_widget(body, frame.area());
}
