//! Run the sandbox in a terminal: the same physics, scene and colours as the
//! Android app, drawn with 24-bit ANSI colour. Click a solid to tap it;
//! `r` restarts with a new random set; `q` / Esc quits.
//!
//!   cargo run --release --example terminal

use std::io::Write as _;
use std::time::{Duration, Instant};

use crossterm::event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, MouseButton, MouseEventKind};
use crossterm::{cursor, execute, terminal};
use platomon::color::{ColorDepth, push_sgr};
use solid_sandbox::sandbox::{Sandbox, render_settings};

const FPS: f32 = 30.0;
/// Terminal cell width / height.
const CHAR_ASPECT: f32 = 0.5;

fn seed() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(7, |t| t.as_nanos() as u64)
}

fn main() -> std::io::Result<()> {
    let mut out = std::io::stdout();
    terminal::enable_raw_mode()?;
    execute!(out, terminal::EnterAlternateScreen, cursor::Hide, EnableMouseCapture)?;
    let result = run(&mut out);
    execute!(out, DisableMouseCapture, cursor::Show, terminal::LeaveAlternateScreen)?;
    terminal::disable_raw_mode()?;
    result
}

fn run(out: &mut std::io::Stdout) -> std::io::Result<()> {
    let rs = render_settings(CHAR_ASPECT);
    let mut sb = Sandbox::new(seed());
    let period = Duration::from_secs_f32(1.0 / FPS);
    let mut last = Instant::now();
    let mut frame = String::new();
    let mut taps = 0u32;
    loop {
        // input until the next frame is due
        let due = last + period;
        while let Some(wait) = due.checked_duration_since(Instant::now()) {
            if !event::poll(wait)? {
                break;
            }
            match event::read()? {
                Event::Key(k) => match k.code {
                    KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                    KeyCode::Char('c') if k.modifiers.contains(event::KeyModifiers::CONTROL) => return Ok(()),
                    KeyCode::Char('r') => sb = Sandbox::new(seed()),
                    _ => {}
                },
                Event::Mouse(m) if m.kind == MouseEventKind::Down(MouseButton::Left) => {
                    if sb.tap(m.column as f32 + 0.5, m.row as f32 + 0.5).is_some() {
                        taps += 1;
                    }
                }
                _ => {}
            }
        }
        let now = Instant::now();
        let dt = (now - last).as_secs_f32();
        last = now;

        let (cols, rows) = terminal::size()?;
        let (cols, rows) = (cols.max(10) as usize, rows.max(3) as usize);
        let area = rows - 1;
        sb.step(dt);
        sb.render(cols, area, &rs);

        frame.clear();
        frame.push_str("\x1b[H");
        let mut cur = u32::MAX - 1;
        for (row, (chars, colors)) in sb.chars.chunks(cols).zip(sb.colors.chunks(cols)).enumerate() {
            for (&c, &k) in chars.iter().zip(colors) {
                if c != ' ' && k != cur {
                    push_sgr(&mut frame, k, ColorDepth::True);
                    cur = k;
                }
                frame.push(c);
            }
            if row + 1 < area {
                frame.push_str("\r\n");
            }
        }
        let status = format!(
            " {} solids | click a solid to tap it | r new set | q quit | taps {taps} ",
            sb.world.bodies.len()
        );
        frame.push_str("\r\n\x1b[0m\x1b[7m");
        frame.extend(status.chars().chain(std::iter::repeat(' ')).take(cols));
        frame.push_str("\x1b[0m");
        out.write_all(frame.as_bytes())?;
        out.flush()?;
    }
}
