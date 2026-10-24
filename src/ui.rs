use std::{
    io::{self, Write},
    sync::mpsc::TryRecvError,
    time::Duration,
};

use crate::{
    config::Config,
    lifecycle::Models,
    model::{Generation, Ollama, Update, Usage},
    timer::TurnTimer,
};

use crossterm::{
    cursor::{MoveToColumn, MoveUp, Show},
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    style::{Attribute, Color, Print, ResetColor, SetAttribute, SetForegroundColor},
    terminal::{Clear, ClearType, SetTitle, disable_raw_mode, enable_raw_mode, size},
};
use ratatui::text::Line;

const ASCII: &str = include_str!("../assets/lorca.txt");
const LOGO: &str = include_str!("../assets/orca.txt");

pub fn run(config: &Config, model: &mut Ollama) -> io::Result<()> {
    let mut stdout = io::stdout();

    // Restore input before the default panic hook writes its diagnostic.
    let panic_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = restore_terminal();
        let _ = execute!(io::stdout(), Print("\r\n"));
        panic_hook(info);
    }));

    // Window title: emoji + lorca
    execute!(
        stdout,
        SetTitle(format!("{} lorca", LOGO.trim())),
        Print("\r\n")
    )?;

    print_header(&mut stdout, config)?;

    // Read keyboard events without entering fullscreen.
    let mut raw_mode = RawMode::new()?;

    let result = session(&mut stdout, model);

    // Always restore normal terminal input.
    let cleanup = raw_mode.restore();

    result?;
    cleanup?;

    // Printed at the current cursor, not the screen bottom.
    execute!(
        stdout,
        SetForegroundColor(Color::Red),
        Print("● session closed\r\n"),
        ResetColor
    )?;

    Ok(())
}

fn restore_terminal() -> io::Result<()> {
    // Attempt both operations even if writing to stdout fails.
    let cursor = execute!(io::stdout(), ResetColor, Show);
    let input = disable_raw_mode();
    cursor.and(input)
}

struct RawMode(bool);

impl RawMode {
    fn new() -> io::Result<Self> {
        enable_raw_mode()?;
        Ok(Self(true))
    }

    fn restore(&mut self) -> io::Result<()> {
        let result = restore_terminal();
        if result.is_ok() {
            self.0 = false;
        }
        result
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        if self.0 {
            let _ = restore_terminal();
        }
    }
}

fn print_header(stdout: &mut io::Stdout, config: &Config) -> io::Result<()> {
    let artwork: Vec<&str> = ASCII.trim_end_matches('\n').lines().collect();

    let width = artwork
        .iter()
        .map(|line| line.chars().count())
        .max()
        .unwrap_or(12);

    let cwd = std::env::current_dir()?;
    let home = std::env::var("HOME").unwrap_or_default();

    let location = cwd
        .strip_prefix(&home)
        .ok()
        .map(|path| {
            if path.as_os_str().is_empty() {
                "~".to_string()
            } else {
                format!("~/{}", path.display())
            }
        })
        .unwrap_or_else(|| cwd.display().to_string());

    // Mascot and session information side by side.
    for i in 0..artwork.len().max(3) {
        let line = artwork.get(i).copied().unwrap_or("");

        write!(stdout, "{line:<width$}   ")?;

        match i {
            0 => {
                execute!(
                    stdout,
                    SetForegroundColor(Color::Green),
                    Print("● "),
                    ResetColor,
                    Print("new session · ollama "),
                    SetForegroundColor(Color::Cyan),
                    Print(&config.model),
                    ResetColor
                )?;
            }

            1 => {
                execute!(
                    stdout,
                    SetForegroundColor(Color::DarkGrey),
                    Print(&location),
                    ResetColor
                )?;
            }

            2 => {
                writeln!(stdout, "lorca is ready")?;
                continue;
            }

            _ => {}
        }

        writeln!(stdout)?;
    }

    // Small Goose-style divider.
    execute!(
        stdout,
        SetForegroundColor(Color::DarkGrey),
        Print("╌".repeat(24)),
        ResetColor,
        Print("\n")
    )?;

    stdout.flush()
}

fn session(stdout: &mut io::Stdout, model: &mut Ollama) -> io::Result<()> {
    let mut prompt = Prompt::default();
    let mut models = Models::default();
    let result = (|| {
        if models.start(stdout, model)? {
            read_input(stdout, &mut prompt, model, &mut models)?;
        }
        Ok(())
    })();
    // Clear the entire transient prompt, including wrapped warning/input rows,
    // on normal exit and on event/output errors. History above it stays intact.
    let cleanup = prompt.clear(stdout);
    let unload = models.close(stdout, model);
    result.and(cleanup).and(unload)
}

fn read_input(
    stdout: &mut io::Stdout,
    prompt: &mut Prompt,
    model: &mut Ollama,
    models: &mut Models,
) -> io::Result<()> {
    let mut input = String::new();
    let mut quit_armed = false;
    let mut generation: Option<Generation> = None;
    let mut timer: Option<TurnTimer> = None;

    prompt.draw(stdout, &input, quit_armed)?;

    loop {
        if let Some(active) = generation.as_ref() {
            let update = match active.updates.try_recv() {
                Ok(update) => Some(update),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => {
                    Some(Update::Error("Ollama response stopped unexpectedly".into()))
                }
            };
            if let Some(update) = update {
                match update {
                    Update::Text(text) => {
                        print_response(stdout, &text)?;
                        if let Some(timer) = timer.as_mut() {
                            timer.advance(&text);
                        }
                    }
                    Update::Done(usage) => {
                        models.response_completed();
                        execute!(stdout, Print("\r\n"))?;
                        finish_timer(stdout, &mut timer)?;
                        generation = None;
                        prompt.usage = usage;
                        prompt.draw(stdout, &input, false)?;
                    }
                    Update::Error(error) => {
                        execute!(stdout, Print("\r\n"))?;
                        finish_timer(stdout, &mut timer)?;
                        execute!(stdout, SetForegroundColor(Color::Red))?;
                        print_response(stdout, &error)?;
                        execute!(stdout, ResetColor, Print("\r\n"))?;
                        generation = None;
                        prompt.draw(stdout, &input, false)?;
                    }
                }
            }
            if let Some(timer) = timer.as_mut() {
                timer.tick(stdout)?;
            }
            // Continue polling the keyboard even when output arrives rapidly.
            if !event::poll(Duration::from_millis(10))? {
                continue;
            }
        }
        if let Event::Key(key) = event::read()? {
            if key.kind != KeyEventKind::Press {
                continue;
            }

            if generation.is_some() {
                if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                    generation = None; // Dropping the task closes its HTTP stream.
                    execute!(stdout, Print("\r\n"))?;
                    finish_timer(stdout, &mut timer)?;
                    quit_armed = false;
                    prompt.draw(stdout, &input, false)?;
                }
                continue;
            }

            match key.code {
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    if quit_armed {
                        break;
                    }

                    quit_armed = true;
                }

                KeyCode::Enter => {
                    if input.trim() == "/exit" {
                        break;
                    }

                    quit_armed = false;

                    let command = input.trim();
                    if command == "/model" || command.starts_with("/model ") {
                        prompt.clear(stdout)?;
                        let name = command.strip_prefix("/model").unwrap().trim().to_string();
                        input.clear();
                        if name.is_empty() || name.split_whitespace().count() != 1 {
                            execute!(
                                stdout,
                                SetForegroundColor(Color::Red),
                                Print("Usage: /model NAME\r\n"),
                                ResetColor
                            )?;
                        } else {
                            let previous = model.name().to_string();
                            if !models.switch(stdout, model, &name)? {
                                break;
                            }
                            if previous != model.name() {
                                prompt.usage = Usage::default();
                            }
                        }
                        prompt.draw(stdout, &input, false)?;
                        continue;
                    }

                    if !input.trim().is_empty() {
                        prompt.clear(stdout)?;
                        execute!(
                            stdout,
                            SetForegroundColor(Color::Rgb {
                                r: 75,
                                g: 156,
                                b: 211
                            }),
                            SetAttribute(Attribute::Bold),
                            Print("me🦦  "),
                            ResetColor,
                            Print(input.as_str())
                        )?;
                        timer = Some(TurnTimer::start(stdout)?);

                        prompt.usage.context_used = None;
                        prompt.usage.tokens_per_second = None;
                        generation = Some(model.start(std::mem::take(&mut input)));
                        execute!(
                            stdout,
                            SetForegroundColor(Color::Rgb {
                                r: 255,
                                g: 255,
                                b: 255
                            }),
                            SetAttribute(Attribute::Bold),
                            Print("lorca🫍  "),
                            SetAttribute(Attribute::NormalIntensity),
                            ResetColor
                        )?;
                        if let Some(timer) = timer.as_mut() {
                            timer.advance("lorca  ");
                        }
                        continue;
                    }
                }

                KeyCode::Backspace => {
                    input.pop();
                    quit_armed = false;
                }

                KeyCode::Char(c)
                    if !c.is_control() && !key.modifiers.contains(KeyModifiers::CONTROL) =>
                {
                    input.push(c);
                    quit_armed = false;
                }

                KeyCode::Esc => {
                    quit_armed = false;
                }

                _ => {}
            }

            prompt.draw(stdout, &input, quit_armed)?;
        }
    }

    Ok(())
}

fn finish_timer(stdout: &mut io::Stdout, timer: &mut Option<TurnTimer>) -> io::Result<()> {
    if let Some(mut timer) = timer.take() {
        timer.advance("\n");
        timer.finish(stdout)?;
    }
    Ok(())
}

fn print_response(stdout: &mut io::Stdout, text: &str) -> io::Result<()> {
    // Raw mode needs CRLF. Model output must remain text, never terminal commands.
    let text: String = text
        .chars()
        .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
        .collect();
    write!(stdout, "{}", text.replace('\n', "\r\n"))?;
    stdout.flush()
}

#[derive(Default)]
struct Prompt {
    cursor_row: u16,
    usage: Usage,
}

impl Prompt {
    fn clear(&mut self, stdout: &mut io::Stdout) -> io::Result<()> {
        if self.cursor_row > 0 {
            execute!(stdout, MoveUp(self.cursor_row))?;
        }
        execute!(stdout, MoveToColumn(0), Clear(ClearType::FromCursorDown))?;
        self.cursor_row = 0;
        stdout.flush()
    }

    fn draw(&mut self, stdout: &mut io::Stdout, input: &str, quit_armed: bool) -> io::Result<()> {
        self.clear(stdout)?;
        let (columns, _) = size()?;
        execute!(
            stdout,
            SetForegroundColor(Color::Cyan),
            Print(status_line(&self.usage, columns)),
            ResetColor,
            Print("\r\n")
        )?;
        self.cursor_row = 1;
        let (text, color) = if quit_armed {
            (
                "Press Ctrl+C again to exit, or type to continue",
                Some(Color::Red),
            )
        } else if input.is_empty() {
            ("Enter to send · Ctrl+C to quit", Some(Color::DarkGrey))
        } else {
            (input, None)
        };

        // Count the rows occupied by this transient line, including a wrapped
        // Ctrl+C warning. An exact-width line is still on its last row until
        // the terminal receives another printable character (delayed autowrap).
        let width = Line::raw(format!("> {text}")).width();
        let wrapped_rows = (width.saturating_sub(1) / usize::from(columns.max(1))) as u16;
        self.cursor_row += wrapped_rows;
        execute!(stdout, Print("> "))?;
        if let Some(color) = color {
            execute!(stdout, SetForegroundColor(color))?;
        }
        execute!(stdout, Print(text), ResetColor)?;
        if input.is_empty() && !quit_armed {
            if wrapped_rows > 0 {
                execute!(stdout, MoveUp(wrapped_rows))?;
            }
            execute!(stdout, MoveToColumn(2))?;
            self.cursor_row = 1;
        }
        stdout.flush()
    }
}

fn status_line(usage: &Usage, columns: u16) -> String {
    let speed = usage
        .tokens_per_second
        .map(|speed| format!("{speed:.1}"))
        .unwrap_or_else(|| "--".into());
    let ratio = usage
        .context_used
        .zip(usage.context_limit)
        .map(|(used, limit)| used as f64 / limit as f64);
    let percent = ratio
        .map(|ratio| format!("{:.0}%", ratio * 100.0))
        .unwrap_or_else(|| "--".into());
    let line = if columns >= 60 {
        let filled = (ratio.unwrap_or(0.0).clamp(0.0, 1.0) * 10.0).round() as usize;
        let bar = format!("{}{}", "█".repeat(filled), "░".repeat(10 - filled));
        let used = usage
            .context_used
            .map(|count| count.to_string())
            .unwrap_or_else(|| "--".into());
        let limit = usage
            .context_limit
            .map(|count| count.to_string())
            .unwrap_or_else(|| "--".into());
        format!("ctx [{bar}] {percent} {used}/{limit} · {speed} t/s")
    } else {
        format!("ctx {percent} · {speed} t/s")
    };
    // Keep status on one row even in narrow terminals; input cleanup can then
    // move back exactly one extra row without reserving a viewport.
    line.chars()
        .take(usize::from(columns.saturating_sub(1)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_shows_usage_and_speed_without_wrapping() {
        let usage = Usage {
            context_used: Some(2048),
            context_limit: Some(4096),
            tokens_per_second: Some(42.25),
        };
        let line = status_line(&usage, 80);
        assert!(line.contains("█████░░░░░"));
        assert!(line.contains("50% 2048/4096"));
        assert!(line.contains("42.2 t/s"));
        assert!(Line::raw(status_line(&usage, 24)).width() < 24);
        assert!(status_line(&Usage::default(), 80).contains("--/--"));
    }
}
