// captui - terminal UI to record the screen, a window, or a region with sound.
// Copyright (C) 2026  Kevin Andriessens
//
// This program is free software: you can redistribute it and/or modify it under
// the terms of the GNU General Public License as published by the Free Software
// Foundation, either version 3 of the License, or (at your option) any later
// version. See the LICENSE file for details.

use std::io;
use std::process::Command;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, HighlightSpacing, List, ListItem, ListState, Paragraph};

use captui::sources::{layout_hints, parse_wlr_randr, sort_reading_order, Output, Source};

#[cfg(feature = "identify")]
mod identify;

fn enumerate_displays() -> Result<Vec<Output>> {
    let out = Command::new("wlr-randr").output().context(
        "could not run wlr-randr (is it installed and are you on a wlroots Wayland session?)",
    )?;
    if !out.status.success() {
        bail!(
            "wlr-randr failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    let outputs = parse_wlr_randr(&String::from_utf8_lossy(&out.stdout));
    let mut displays: Vec<Output> = outputs.into_iter().filter(|o| o.enabled).collect();
    sort_reading_order(&mut displays);
    Ok(displays)
}

struct App {
    displays: Vec<Output>,
    list: ListState,
    selected: Option<Source>,
    status: Option<String>,
}

impl App {
    fn new(displays: Vec<Output>) -> Self {
        let mut list = ListState::default();
        if !displays.is_empty() {
            list.select(Some(0));
        }
        Self {
            displays,
            list,
            selected: None,
            status: None,
        }
    }

    fn identify(&mut self) {
        self.status = Some(run_identify(&self.displays));
    }

    fn move_by(&mut self, delta: isize) {
        if self.displays.is_empty() {
            return;
        }
        let len = self.displays.len();
        let cur = self.list.selected().unwrap_or(0) as isize;
        let next = (cur + delta).rem_euclid(len as isize) as usize;
        self.list.select(Some(next));
    }

    fn confirm(&mut self) {
        if let Some(i) = self.list.selected() {
            if let Some(o) = self.displays.get(i) {
                self.selected = Some(Source::Display(o.name.clone()));
            }
        }
    }
}

fn main() -> Result<()> {
    let displays = enumerate_displays();

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout))?;

    let res = run(&mut terminal, displays);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    let app = res?;
    if let Some(Source::Display(name)) = app.selected {
        println!("selected display: {name}");
    }
    Ok(())
}

fn run(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    displays: Result<Vec<Output>>,
) -> Result<App> {
    let (mut app, error) = match displays {
        Ok(d) => (App::new(d), None),
        Err(e) => (App::new(Vec::new()), Some(format!("{e:#}"))),
    };

    loop {
        terminal.draw(|f| draw(f, &mut app, error.as_deref()))?;

        if event::poll(Duration::from_millis(200))? {
            if let Event::Key(k) = event::read()? {
                if k.kind != KeyEventKind::Press {
                    continue;
                }
                match k.code {
                    KeyCode::Char('q') | KeyCode::Esc => return Ok(app),
                    KeyCode::Down | KeyCode::Char('j') => app.move_by(1),
                    KeyCode::Up | KeyCode::Char('k') => app.move_by(-1),
                    KeyCode::Char('i') => app.identify(),
                    KeyCode::Enter => {
                        app.confirm();
                        return Ok(app);
                    }
                    _ => {}
                }
            }
        }
    }
}

#[cfg(feature = "identify")]
fn run_identify(displays: &[Output]) -> String {
    use std::collections::HashMap;
    use std::time::Duration;

    let numbers: HashMap<String, u32> = displays
        .iter()
        .enumerate()
        .map(|(i, o)| (o.name.clone(), i as u32 + 1))
        .collect();
    match identify::flash(&numbers, Duration::from_millis(1600)) {
        Ok(0) => "identify: no matching outputs".into(),
        Ok(n) => format!("flashed a number on {n} screen(s)"),
        Err(e) => format!("identify failed: {e:#}"),
    }
}

#[cfg(not(feature = "identify"))]
fn run_identify(_displays: &[Output]) -> String {
    "identify overlay not built in this binary".into()
}

fn row_label(n: usize, o: &Output, hint: &str) -> String {
    let mode = o.mode.map(|m| m.label()).unwrap_or_else(|| "?".into());
    let pos = match o.position {
        Some((x, y)) => format!("@{x},{y}"),
        None => String::new(),
    };
    format!("{n:>2}. {:<9} {:<13} {:<12} {hint}", o.name, mode, pos)
        .trim_end()
        .to_string()
}

fn draw(f: &mut Frame, app: &mut App, error: Option<&str>) {
    let chunks = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).split(f.area());

    let block = Block::default()
        .title(" captui - select a display ")
        .borders(Borders::ALL);

    if let Some(err) = error {
        let body = Paragraph::new(err).block(block).style(Style::new().red());
        f.render_widget(body, chunks[0]);
    } else if app.displays.is_empty() {
        let body = Paragraph::new("no enabled displays found.").block(block);
        f.render_widget(body, chunks[0]);
    } else {
        let hints = layout_hints(&app.displays);
        let items: Vec<ListItem> = app
            .displays
            .iter()
            .zip(hints)
            .enumerate()
            .map(|(i, (o, hint))| ListItem::new(row_label(i + 1, o, &hint)))
            .collect();
        let list = List::new(items)
            .block(block)
            .highlight_symbol("> ")
            .highlight_spacing(HighlightSpacing::Always)
            .highlight_style(Style::new().reversed());
        f.render_stateful_widget(list, chunks[0], &mut app.list);
    }

    let footer = match &app.status {
        Some(s) => Paragraph::new(format!(" {s} ")).style(Style::new().yellow()),
        None => Paragraph::new(" up/down move  i identify  enter select  q quit ")
            .style(Style::new().dim()),
    };
    f.render_widget(footer, chunks[1]);
}
