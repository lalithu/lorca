use std::{
    io::{self, Write},
    sync::mpsc::TryRecvError,
    time::{Duration, Instant},
};

use crossterm::{
    cursor::MoveToColumn,
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute, queue,
    style::{Color, Print, ResetColor, SetForegroundColor},
    terminal::{Clear, ClearType, size},
};

use crate::model::{ModelOperation, Ollama, same_model};

/// Names whose load was attempted, including uncertain/cancelled requests. A
/// failed unload stays here so session exit can make one further cleanup attempt.
#[derive(Default)]
pub struct Models {
    pending: Vec<String>,
    ready: bool,
}

enum Wait {
    Complete(Result<(), String>),
    Cancelled,
    Exit,
}

enum Unload {
    Confirmed,
    Unconfirmed,
    Cancelled,
}

impl Models {
    pub fn start(&mut self, stdout: &mut io::Stdout, model: &Ollama) -> io::Result<bool> {
        self.track(model.name());
        match wait(
            stdout,
            model.load(model.name()),
            &format!("loading {}", model.name()),
            true,
        )? {
            Wait::Complete(Ok(())) => {
                self.ready = true;
                Ok(true)
            }
            Wait::Complete(Err(error)) => {
                message(stdout, Color::Red, &format!("● model load failed: {error}"))?;
                Ok(true) // Allow /model, a retry message, or /exit.
            }
            Wait::Cancelled => {
                message(
                    stdout,
                    Color::Yellow,
                    "● loading cancelled; send a message or /model NAME to retry",
                )?;
                Ok(true)
            }
            Wait::Exit => Ok(false),
        }
    }

    pub fn switch(
        &mut self,
        stdout: &mut io::Stdout,
        model: &mut Ollama,
        name: &str,
    ) -> io::Result<bool> {
        let same = same_model(model.name(), name);
        if same && self.ready {
            message(
                stdout,
                Color::Green,
                &format!("● already using {}", model.name()),
            )?;
            return Ok(true);
        }
        self.track(name);
        match wait(stdout, model.load(name), &format!("loading {name}"), true)? {
            Wait::Complete(Ok(())) => {
                let previous = model.name().to_string();
                model.select(name.to_string());
                self.ready = true;
                message(stdout, Color::Green, &format!("● model switched to {name}"))?;
                if !same {
                    self.unload(stdout, model, &previous, true)?;
                }
            }
            Wait::Complete(Err(error)) => {
                message(
                    stdout,
                    Color::Red,
                    &format!("● model switch failed: {error}"),
                )?;
                message(
                    stdout,
                    Color::Yellow,
                    &format!("● continuing with {}", model.name()),
                )?;
                if !same {
                    self.unload(stdout, model, name, true)?;
                }
            }
            Wait::Cancelled => {
                message(
                    stdout,
                    Color::Yellow,
                    &format!("● switch cancelled; continuing with {}", model.name()),
                )?;
                if !same {
                    self.unload(stdout, model, name, true)?;
                }
            }
            Wait::Exit => return Ok(false),
        }
        Ok(true)
    }

    pub fn response_completed(&mut self) {
        self.ready = true;
    }

    pub fn close(&mut self, stdout: &mut io::Stdout, model: &Ollama) -> io::Result<()> {
        // Retired models go first, leaving the active model's confirmed message
        // immediately adjacent to "session closed" on the normal exit path.
        let mut names = self.pending.clone();
        names.sort_by_key(|name| same_model(name, model.name()));
        for name in names {
            if matches!(
                self.unload(stdout, model, &name, !same_model(&name, model.name()))?,
                Unload::Cancelled
            ) {
                message(
                    stdout,
                    Color::Yellow,
                    "● cleanup interrupted; models may still be loaded",
                )?;
                break;
            }
        }
        Ok(())
    }

    fn track(&mut self, name: &str) {
        if !self.pending.iter().any(|pending| same_model(pending, name)) {
            self.pending.push(name.to_string());
        }
    }

    fn unload(
        &mut self,
        stdout: &mut io::Stdout,
        model: &Ollama,
        name: &str,
        show_name: bool,
    ) -> io::Result<Unload> {
        // Ollama has no per-client lease/ownership API. Warn every time, even
        // for models lorca initially loaded: another client may have joined.
        message(
            stdout,
            Color::Yellow,
            &format!("● unloading {name} may affect other Ollama clients"),
        )?;
        match wait(
            stdout,
            model.unload(name),
            &format!("unloading {name}"),
            false,
        )? {
            Wait::Complete(Ok(())) => {
                self.pending.retain(|pending| !same_model(pending, name));
                let text = if show_name {
                    format!("● model unloaded · {name}")
                } else {
                    "● model unloaded".into()
                };
                message(stdout, Color::Green, &text)?;
                Ok(Unload::Confirmed)
            }
            Wait::Complete(Err(error)) => {
                message(
                    stdout,
                    Color::Red,
                    &format!("● unload not confirmed for {name}: {error}"),
                )?;
                Ok(Unload::Unconfirmed)
            }
            Wait::Cancelled | Wait::Exit => {
                message(
                    stdout,
                    Color::Yellow,
                    &format!("● unload cancelled; {name} may still be loaded"),
                )?;
                Ok(Unload::Cancelled)
            }
        }
    }
}

fn message(stdout: &mut io::Stdout, color: Color, text: &str) -> io::Result<()> {
    let text: String = text
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect();
    execute!(
        stdout,
        SetForegroundColor(color),
        Print(text),
        ResetColor,
        Print("\r\n")
    )
}

fn wait(
    stdout: &mut io::Stdout,
    operation: ModelOperation,
    label: &str,
    loading: bool,
) -> io::Result<Wait> {
    let result = wait_loop(stdout, &operation, label, loading);
    // Clear only this one transient row, on success, failure, or interruption.
    let clear = execute!(stdout, MoveToColumn(0), Clear(ClearType::CurrentLine));
    clear?;
    result
}

fn wait_loop(
    stdout: &mut io::Stdout,
    operation: &ModelOperation,
    label: &str,
    loading: bool,
) -> io::Result<Wait> {
    let started = Instant::now();
    let mut quit_armed = false;
    loop {
        match operation.result.try_recv() {
            Ok(result) => return Ok(Wait::Complete(result)),
            Err(TryRecvError::Disconnected) => {
                return Ok(Wait::Complete(Err(
                    "Ollama operation stopped unexpectedly".into()
                )));
            }
            Err(TryRecvError::Empty) => {}
        }
        let columns = size()?.0;
        let text = if quit_armed {
            "Press Ctrl+C again to exit, or type to continue"
                .chars()
                .take(usize::from(columns.saturating_sub(1)))
                .collect()
        } else {
            loading_line(label, started.elapsed(), columns)
        };
        // Replace the existing text before clearing its tail; one flush keeps
        // the indeterminate animation compact and avoids blank-frame flicker.
        queue!(
            stdout,
            MoveToColumn(0),
            SetForegroundColor(if quit_armed { Color::Red } else { Color::Cyan }),
            Print(text),
            ResetColor,
            Clear(ClearType::UntilNewLine)
        )?;
        stdout.flush()?;
        if event::poll(Duration::from_millis(80))?
            && let Event::Key(key) = event::read()?
        {
            if key.kind != KeyEventKind::Press {
                continue;
            }
            if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                if !loading {
                    return Ok(Wait::Cancelled);
                }
                if quit_armed {
                    return Ok(Wait::Exit);
                }
                quit_armed = true;
            } else if key.code == KeyCode::Esc {
                return Ok(Wait::Cancelled);
            } else if matches!(key.code, KeyCode::Char(_) | KeyCode::Backspace) {
                quit_armed = false;
            }
        }
    }
}

fn loading_line(label: &str, elapsed: Duration, columns: u16) -> String {
    use ratatui::{style::Style, text::Line};
    fn clip(text: &str, columns: usize) -> String {
        let line = Line::raw(text);
        let mut result = String::new();
        let mut width = 0;
        for grapheme in line.styled_graphemes(Style::default()) {
            let next = Line::raw(grapheme.symbol).width();
            if width + next > columns {
                break;
            }
            result.push_str(grapheme.symbol);
            width += next;
        }
        result
    }
    let phase = (elapsed.as_millis() / 80 % 10) as usize;
    let bar: String = (0..10)
        .map(|index| {
            if (index + 10 - phase) % 10 < 3 {
                '█'
            } else {
                '░'
            }
        })
        .collect();
    let tail = format!(" [{bar}] {:.1}s", elapsed.as_secs_f64());
    let available = usize::from(columns.saturating_sub(1));
    let label = clip(label, available.saturating_sub(tail.chars().count()));
    clip(&format!("{label}{tail}"), available)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::text::Line;

    #[test]
    fn animation_is_indeterminate_and_stays_on_one_row() {
        let initial = loading_line("loading orca", Duration::ZERO, 80);
        let later = loading_line("loading orca", Duration::from_millis(160), 80);
        assert_ne!(initial, later);
        assert!(later.ends_with("0.2s"));
        assert!(!later.contains('%'));
        assert!(
            Line::raw(loading_line(
                "loading a-very-long-model-name",
                Duration::from_secs(10),
                24
            ))
            .width()
                < 24
        );
    }
}
