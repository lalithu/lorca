use std::io::{self, Write};

use crossterm::{
    cursor::{MoveToColumn, MoveUp, Show},
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    style::{Color, Print, ResetColor, SetForegroundColor},
    terminal::{Clear, ClearType, SetTitle, disable_raw_mode, enable_raw_mode, size},
};
use ratatui::text::Line;

const ASCII: &str = include_str!("../assets/lorca.txt");
const LOGO: &str = include_str!("../assets/orca.txt");

fn main() -> io::Result<()> {
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

    print_header(&mut stdout)?;

    // Read keyboard events without entering fullscreen.
    let mut raw_mode = RawMode::new()?;

    let result = run(&mut stdout);

    // Always restore normal terminal input.
    let cleanup = raw_mode.restore();

    result?;
    cleanup?;

    // Printed at the current cursor, not the screen bottom.
    execute!(stdout, Print("● session closed\r\n"))?;

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

fn print_header(stdout: &mut io::Stdout) -> io::Result<()> {
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
                    Print("qwen3.5:9b"),
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

fn run(stdout: &mut io::Stdout) -> io::Result<()> {
    let mut prompt = Prompt::default();
    let result = read_input(stdout, &mut prompt);
    // Clear the entire transient prompt, including wrapped warning/input rows,
    // on normal exit and on event/output errors. History above it stays intact.
    let cleanup = prompt.clear(stdout);
    result.and(cleanup)
}

fn read_input(stdout: &mut io::Stdout, prompt: &mut Prompt) -> io::Result<()> {
    let mut input = String::new();
    let mut quit_armed = false;

    prompt.draw(stdout, &input, quit_armed)?;

    loop {
        if let Event::Key(key) = event::read()? {
            if key.kind != KeyEventKind::Press {
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

                    if !input.trim().is_empty() {
                        prompt.clear(stdout)?;
                        execute!(
                            stdout,
                            SetForegroundColor(Color::Cyan),
                            Print("you  "),
                            ResetColor,
                            Print(input.as_str()),
                            Print("\r\n")
                        )?;

                        input.clear();
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

#[derive(Default)]
struct Prompt {
    cursor_row: u16,
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
        self.cursor_row = (width.saturating_sub(1) / usize::from(columns.max(1))) as u16;
        execute!(stdout, Print("> "))?;
        if let Some(color) = color {
            execute!(stdout, SetForegroundColor(color))?;
        }
        execute!(stdout, Print(text), ResetColor)?;
        if input.is_empty() && !quit_armed {
            if self.cursor_row > 0 {
                execute!(stdout, MoveUp(self.cursor_row))?;
            }
            execute!(stdout, MoveToColumn(2))?;
            self.cursor_row = 0;
        }
        stdout.flush()
    }
}
