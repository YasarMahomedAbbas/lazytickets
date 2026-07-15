mod app;
mod gh;
mod model;
mod ui;

use app::App;
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // M1: resolver is hardcoded to travel-smart board #6. Git-remote resolution
    // and the first-run wizard land in M4.
    let items = gh::project::item_list("WhiteWolfStudio", 6).await?;
    let app = App::new(items, "travel-smart #6".to_string());

    let mut terminal = ratatui::init();
    let result = run(&mut terminal, app);
    ratatui::restore();
    result
}

fn run(terminal: &mut DefaultTerminal, mut app: App) -> anyhow::Result<()> {
    loop {
        terminal.draw(|frame| ui::render(frame, &mut app))?;

        if let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            match key.code {
                KeyCode::Char('q') => break,
                KeyCode::Char('j') | KeyCode::Down => app.next(),
                KeyCode::Char('k') | KeyCode::Up => app.prev(),
                _ => {}
            }
        }
    }
    Ok(())
}
