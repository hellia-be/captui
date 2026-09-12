// captui - terminal UI to record the screen, a window, or a region with sound.
// Copyright (C) 2026  Kevin Andriessens
//
// This program is free software: you can redistribute it and/or modify it under
// the terms of the GNU General Public License as published by the Free Software
// Foundation, either version 3 of the License, or (at your option) any later
// version. See the LICENSE file for details.

use std::io;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, bail, Context, Result};
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use directories::UserDirs;
use nix::sys::signal::{self, Signal};
use nix::unistd::Pid;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, HighlightSpacing, List, ListItem, ListState, Paragraph};

use captui::audio::{parse_pw_dump, AudioSource};
use captui::format::{format_duration, format_size};
use captui::recorder::{extension, timestamped_name, wf_recorder_argv, Mode};
use captui::sources::{
    layout_hints, parse_geometry, parse_wlr_randr, region, sort_reading_order, Output, Source,
};

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

fn enumerate_audio() -> Result<Vec<AudioSource>> {
    let out = Command::new("pw-dump")
        .output()
        .context("could not run pw-dump (is PipeWire installed?)")?;
    if !out.status.success() {
        bail!(
            "pw-dump failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(parse_pw_dump(&String::from_utf8_lossy(&out.stdout)))
}

fn output_path() -> Result<PathBuf> {
    let base = UserDirs::new()
        .and_then(|u| u.video_dir().map(Path::to_path_buf))
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Videos")))
        .ok_or_else(|| anyhow!("could not determine a video directory"))?;
    let dir = base.join("captures");
    std::fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default();
    Ok(dir.join(timestamped_name(secs, extension(Mode::AudioVideo))))
}

fn spawn_recorder(source: &Source, audio: Option<&str>, out: &Path) -> Result<Child> {
    let argv = wf_recorder_argv(source, audio, &out.to_string_lossy());
    Command::new(&argv[0])
        .args(&argv[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("could not spawn {} (is it installed?)", argv[0]))
}

fn stop_recorder(child: &mut Child) -> Result<()> {
    let _ = signal::kill(Pid::from_raw(child.id() as i32), Signal::SIGINT);
    child
        .wait()
        .context("waiting for wf-recorder to finalize")?;
    Ok(())
}

struct Rec {
    child: Child,
    path: PathBuf,
    stopped: bool,
    started: Instant,
    final_elapsed: Option<Duration>,
}

impl Rec {
    fn elapsed(&self) -> Duration {
        self.final_elapsed.unwrap_or_else(|| self.started.elapsed())
    }

    fn size_bytes(&self) -> u64 {
        std::fs::metadata(&self.path).map(|m| m.len()).unwrap_or(0)
    }
}

enum Screen {
    Source,
    Audio,
    Recording,
}

struct App {
    displays: Vec<Output>,
    source_list: ListState,
    pending_source: Option<Source>,
    audio_options: Vec<Option<AudioSource>>,
    audio_list: ListState,
    screen: Screen,
    recording: Option<Rec>,
    status: Option<String>,
}

impl App {
    fn new(displays: Vec<Output>) -> Self {
        let mut source_list = ListState::default();
        if !displays.is_empty() {
            source_list.select(Some(0));
        }
        Self {
            displays,
            source_list,
            pending_source: None,
            audio_options: Vec::new(),
            audio_list: ListState::default(),
            screen: Screen::Source,
            recording: None,
            status: None,
        }
    }

    fn active_list(&mut self) -> (&mut ListState, usize) {
        match self.screen {
            Screen::Audio => (&mut self.audio_list, self.audio_options.len()),
            _ => (&mut self.source_list, self.displays.len()),
        }
    }

    fn move_by(&mut self, delta: isize) {
        let (list, len) = self.active_list();
        if len == 0 {
            return;
        }
        let cur = list.selected().unwrap_or(0) as isize;
        list.select(Some((cur + delta).rem_euclid(len as isize) as usize));
    }

    fn identify(&mut self) {
        self.status = Some(run_identify(&self.displays));
    }

    fn choose_display(&mut self) {
        if let Some(o) = self
            .source_list
            .selected()
            .and_then(|i| self.displays.get(i))
        {
            self.pending_source = Some(Source::Display(o.name.clone()));
            self.enter_audio();
        }
    }

    fn choose_region(&mut self, src: Source) {
        self.pending_source = Some(src);
        self.enter_audio();
    }

    fn enter_audio(&mut self) {
        self.status = None;
        let sources = match enumerate_audio() {
            Ok(s) => s,
            Err(e) => {
                self.status = Some(format!("{e:#}"));
                Vec::new()
            }
        };
        self.audio_options = sources
            .into_iter()
            .map(Some)
            .chain(std::iter::once(None))
            .collect();
        self.audio_list.select(Some(0));
        self.screen = Screen::Audio;
    }

    fn back_to_source(&mut self) {
        self.pending_source = None;
        self.status = None;
        self.screen = Screen::Source;
    }

    fn start_recording(&mut self) {
        let audio = match self
            .audio_list
            .selected()
            .and_then(|i| self.audio_options.get(i))
        {
            Some(choice) => choice.as_ref().map(|a| a.node_name.clone()),
            None => return,
        };
        let Some(source) = self.pending_source.take() else {
            return;
        };
        let path = match output_path() {
            Ok(p) => p,
            Err(e) => {
                self.pending_source = Some(source);
                self.status = Some(format!("{e:#}"));
                return;
            }
        };
        match spawn_recorder(&source, audio.as_deref(), &path) {
            Ok(child) => {
                self.recording = Some(Rec {
                    child,
                    path,
                    stopped: false,
                    started: Instant::now(),
                    final_elapsed: None,
                });
                self.status = None;
                self.screen = Screen::Recording;
            }
            Err(e) => {
                self.pending_source = Some(source);
                self.status = Some(format!("{e:#}"));
            }
        }
    }

    fn stop(&mut self) {
        let Some(rec) = self.recording.as_mut() else {
            return;
        };
        if rec.stopped {
            return;
        }
        rec.final_elapsed = Some(rec.started.elapsed());
        let msg = match stop_recorder(&mut rec.child) {
            Ok(()) => {
                rec.stopped = true;
                format!("saved: {}", rec.path.display())
            }
            Err(e) => format!("stop failed: {e:#}"),
        };
        self.status = Some(msg);
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

    if let Some(rec) = res?.recording {
        println!("recording saved: {}", rec.path.display());
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

        if !event::poll(Duration::from_millis(200))? {
            continue;
        }
        let Event::Key(k) = event::read()? else {
            continue;
        };
        if k.kind != KeyEventKind::Press {
            continue;
        }
        match app.screen {
            Screen::Source => match k.code {
                KeyCode::Char('q') | KeyCode::Esc => return Ok(app),
                KeyCode::Down | KeyCode::Char('j') => app.move_by(1),
                KeyCode::Up | KeyCode::Char('k') => app.move_by(-1),
                KeyCode::Char('i') => app.identify(),
                KeyCode::Char('r') => match run_slurp() {
                    Ok(src) => app.choose_region(src),
                    Err(e) => app.status = Some(format!("{e:#}")),
                },
                KeyCode::Enter => app.choose_display(),
                _ => {}
            },
            Screen::Audio => match k.code {
                KeyCode::Char('q') => return Ok(app),
                KeyCode::Esc => app.back_to_source(),
                KeyCode::Down | KeyCode::Char('j') => app.move_by(1),
                KeyCode::Up | KeyCode::Char('k') => app.move_by(-1),
                KeyCode::Enter => app.start_recording(),
                _ => {}
            },
            Screen::Recording => match k.code {
                KeyCode::Char('s') => app.stop(),
                KeyCode::Char('q') | KeyCode::Esc => {
                    app.stop();
                    return Ok(app);
                }
                _ => {}
            },
        }
    }
}

fn run_slurp() -> Result<Source> {
    let out = Command::new("slurp")
        .output()
        .context("could not run slurp (is it installed?)")?;
    if !out.status.success() {
        bail!("region selection cancelled");
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let (x, y, w, h) = parse_geometry(&text)
        .ok_or_else(|| anyhow!("unexpected slurp output: {:?}", text.trim()))?;
    Ok(Source::Region(region(x, y, w, h)))
}

#[cfg(feature = "identify")]
fn run_identify(displays: &[Output]) -> String {
    use std::collections::HashMap;

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

fn audio_label(choice: &Option<AudioSource>) -> String {
    match choice {
        None => "No audio (silent)".into(),
        Some(a) => a.description.clone(),
    }
}

fn draw(f: &mut Frame, app: &mut App, error: Option<&str>) {
    let chunks = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).split(f.area());
    match app.screen {
        Screen::Source => draw_source(f, app, error, chunks[0]),
        Screen::Audio => draw_audio(f, app, chunks[0]),
        Screen::Recording => draw_recording(f, app, chunks[0]),
    }
    draw_footer(f, app, chunks[1]);
}

fn draw_source(f: &mut Frame, app: &mut App, error: Option<&str>, area: Rect) {
    let block = Block::default()
        .title(" captui - select a source ")
        .borders(Borders::ALL);
    if let Some(err) = error {
        f.render_widget(
            Paragraph::new(err).block(block).style(Style::new().red()),
            area,
        );
    } else if app.displays.is_empty() {
        f.render_widget(
            Paragraph::new("no enabled displays found.").block(block),
            area,
        );
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
        f.render_stateful_widget(list, area, &mut app.source_list);
    }
}

fn draw_audio(f: &mut Frame, app: &mut App, area: Rect) {
    let block = Block::default()
        .title(" captui - select audio ")
        .borders(Borders::ALL);
    let items: Vec<ListItem> = app
        .audio_options
        .iter()
        .map(|c| ListItem::new(audio_label(c)))
        .collect();
    let list = List::new(items)
        .block(block)
        .highlight_symbol("> ")
        .highlight_spacing(HighlightSpacing::Always)
        .highlight_style(Style::new().reversed());
    f.render_stateful_widget(list, area, &mut app.audio_list);
}

fn draw_recording(f: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .title(" captui - recording ")
        .borders(Borders::ALL);
    let body = match &app.recording {
        Some(rec) => {
            let timer = format_duration(rec.elapsed().as_secs());
            let size = format_size(rec.size_bytes());
            let head = if rec.stopped {
                Line::from(vec!["■ stopped  ".green(), timer.into()])
            } else {
                Line::from(vec!["● REC  ".red().bold(), timer.into()])
            };
            Text::from(vec![
                head,
                Line::from(format!("size: {size}")),
                Line::from(format!("file: {}", rec.path.display())),
            ])
        }
        None => Text::from("not recording"),
    };
    f.render_widget(Paragraph::new(body).block(block), area);
}

fn draw_footer(f: &mut Frame, app: &App, area: Rect) {
    let stopped = matches!(&app.recording, Some(r) if r.stopped);
    let hint = match app.screen {
        Screen::Source => " up/down move  i identify  enter display  r region  q quit ",
        Screen::Audio => " up/down move  enter select  esc back  q quit ",
        Screen::Recording if stopped => " q quit ",
        Screen::Recording => " s stop  q stop and quit ",
    };
    let footer = match &app.status {
        Some(s) => Paragraph::new(format!(" {s} ")).style(Style::new().yellow()),
        None => Paragraph::new(hint).style(Style::new().dim()),
    };
    f.render_widget(footer, area);
}
