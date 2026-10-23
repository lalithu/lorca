use std::{
    io,
    time::{Duration, Instant},
};

use crossterm::{
    cursor::{MoveTo, RestorePosition, SavePosition, position},
    execute,
    style::{Color, Print, ResetColor, SetForegroundColor},
    terminal::size,
};
use ratatui::{style::Style, text::Line};

const WIDTH: u16 = 11;

/// An elapsed timer attached to the submitted user line. Tracking streamed rows
/// lets its anchor move with ordinary terminal scrolling, without a viewport.
pub struct TurnTimer {
    started: Instant,
    last_draw: Instant,
    anchor: Option<(u16, u16)>,
    column: u16,
    row: u16,
    columns: u16,
    height: u16,
}

impl TurnTimer {
    pub fn start(stdout: &mut io::Stdout) -> io::Result<Self> {
        let started = Instant::now();
        let (columns, height) = size()?;
        let (mut column, mut row) = position()?;
        if columns.saturating_sub(column) < WIDTH {
            execute!(stdout, Print("\r\n"))?;
            column = 0;
            row = (row + 1).min(height.saturating_sub(1));
        }
        let mut timer = Self {
            started,
            last_draw: started,
            anchor: Some((column, row)),
            column,
            row,
            columns,
            height,
        };
        let text = timer.text();
        execute!(
            stdout,
            SetForegroundColor(Color::DarkGrey),
            Print(&text),
            ResetColor,
            Print("\r\n")
        )?;
        timer.advance(&text);
        timer.advance("\n");
        Ok(timer)
    }

    pub fn tick(&mut self, stdout: &mut io::Stdout) -> io::Result<()> {
        if self.last_draw.elapsed() >= Duration::from_millis(100) {
            self.finish(stdout)?;
        }
        Ok(())
    }

    pub fn finish(&mut self, stdout: &mut io::Stdout) -> io::Result<()> {
        self.last_draw = Instant::now();
        // A resize can reflow existing terminal rows. Stop editing the old
        // anchor rather than risk writing over conversation text.
        if size()? != (self.columns, self.height) {
            self.anchor = None;
        }
        if let Some((column, row)) = self.anchor {
            // Saving/restoring a cursor can cancel delayed autowrap. Wait for
            // the next character/newline when output has filled the final cell.
            if self.column < self.columns {
                execute!(
                    stdout,
                    SavePosition,
                    MoveTo(column, row),
                    SetForegroundColor(Color::DarkGrey),
                    Print(self.text()),
                    ResetColor,
                    RestorePosition
                )?;
            }
        }
        Ok(())
    }

    fn text(&self) -> String {
        format!(
            "  [{:>6.1}s]",
            self.started.elapsed().as_secs_f64().min(9999.9)
        )
    }

    pub fn advance(&mut self, text: &str) {
        for part in text.split_inclusive('\n') {
            let line = Line::raw(part.trim_end_matches('\n'));
            for grapheme in line.styled_graphemes(Style::default()) {
                if grapheme.symbol == "\t" {
                    self.column = ((self.column / 8 + 1) * 8).min(self.columns.saturating_sub(1));
                    continue;
                }
                if grapheme.symbol.chars().any(char::is_control) {
                    continue;
                }
                let width = Line::raw(grapheme.symbol).width() as u16;
                if width > 0 && self.column.saturating_add(width) > self.columns {
                    self.new_line();
                }
                self.column = self.column.saturating_add(width);
            }
            if part.ends_with('\n') {
                self.new_line();
            }
        }
    }

    fn new_line(&mut self) {
        self.column = 0;
        if self.row + 1 < self.height {
            self.row += 1;
        } else if let Some((column, row)) = self.anchor {
            self.anchor = row.checked_sub(1).map(|row| (column, row));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anchor_tracks_wraps_and_scrolling_without_reaching_scrollback() {
        let now = Instant::now();
        let mut timer = TurnTimer {
            started: now,
            last_draw: now,
            anchor: Some((10, 1)),
            column: 7,
            row: 2,
            columns: 20,
            height: 3,
        };
        timer.advance("hello world 12");
        assert_eq!(timer.anchor, Some((10, 0)));
        timer.advance("\n");
        assert_eq!(timer.anchor, None);
        assert_eq!(timer.column, 0);
        assert_eq!(timer.row, 2);
    }
}
