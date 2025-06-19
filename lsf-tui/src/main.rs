use std::result;

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use ratatui::{
    DefaultTerminal, Frame,
    buffer::Buffer,
    layout::Rect,
    style::Stylize,
    symbols::border,
    text::{Line, Text},
    widgets::{Block, Paragraph, Widget},
};

#[derive(Debug)]
pub struct App {
    // search query history
    history: Vec<String>,
    query_cursor: usize,
    queries: Vec<String>,

    results: Vec<String>,
    updates: Vec<String>,
    exit: bool,
}

impl App {
    pub fn new() -> Self {
        Self {
            history: vec![],
            query_cursor: 0,
            queries: vec![String::new()],
            results: vec![],
            updates: vec![],
            exit: false,
        }
    }

    /// runs the application's main loop until the user quits
    pub fn run(&mut self, terminal: &mut DefaultTerminal) -> Result<()> {
        while !self.exit {
            terminal.draw(|frame| self.draw(frame))?;
            self.handle_events()?;
        }
        Ok(())
    }

    fn draw(&self, frame: &mut Frame) {
        frame.render_widget(self, frame.area());
    }

    fn handle_events(&mut self) -> Result<()> {
        match event::read()? {
            // it's important to check that the event is a key press event as
            // crossterm also emits key release and repeat events on Windows.
            Event::Key(key_event) if key_event.kind == KeyEventKind::Press => {
                self.handle_key_event(key_event)
            }
            _ => {}
        };
        Ok(())
    }

    fn query_cursor_move_back(&mut self) {
        if self.queries.get(self.query_cursor + 1).is_some() {
            self.query_cursor += 1;
        } else {
            if self.history.len() - 1 >= self.query_cursor {
                if let Some(history) = self.history.get(self.history.len() - 1 - self.query_cursor)
                {
                    self.queries.push(history.clone());
                    self.query_cursor += 1;
                }
            }
        }
    }

    fn query_cursor_move_forward(&mut self) {
        self.query_cursor = self.query_cursor.saturating_sub(1);
    }

    fn fire_query_and_reset_query_cursor(&mut self) {
        self.history
            .push(std::mem::take(&mut self.queries[self.query_cursor]));
        self.queries = vec![String::new()];
        self.query_cursor = 0;
    }

    fn query(&self) -> &String {
        &self.queries[self.query_cursor]
    }

    fn query_mut(&mut self) -> &mut String {
        &mut self.queries[self.query_cursor]
    }

    fn handle_key_event(&mut self, key_event: KeyEvent) {
        match key_event.code {
            KeyCode::Enter => {
                if self.query() == "/bye" {
                    self.exit = true;
                } else {
                    self.results.push(self.query().clone());
                    self.fire_query_and_reset_query_cursor();
                }
            }

            KeyCode::Up => {
                self.query_cursor_move_back();
            }
            KeyCode::Down => {
                self.query_cursor_move_forward();
            }
            KeyCode::Char(c) => self.query_mut().push(c),
            KeyCode::Backspace => {
                self.query_mut().pop();
            }
            _ => {}
        }
    }
}

impl Widget for &App {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let title = Line::from(" ListSystemFile ".bold());
        let instructions = Line::from(vec![" Quit ".into(), "</bye>".blue().bold()]);
        let block = Block::bordered()
            .title(title.centered())
            .title_bottom(instructions.centered())
            .border_set(border::THICK);

            let query_line = Line::from(vec![
            " > ".to_string().green(),
            self.query().clone().yellow(),
        ]);
        let result_lines = self.results.iter().map(|s| Line::from(s.clone()));
        let mut lines = vec![query_line];
        lines.extend(result_lines.into_iter());
        let inner_text = Text::from(lines);

        Paragraph::new(inner_text).block(block).render(area, buf);
    }
}

fn main() -> Result<()> {
    let mut terminal = ratatui::init();
    let app_result = App::new().run(&mut terminal);
    ratatui::restore();
    app_result
}
