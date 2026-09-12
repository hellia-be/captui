// captui - terminal UI to record the screen, a window, or a region with sound.
// Copyright (C) 2026  Kevin Andriessens
//
// This program is free software: you can redistribute it and/or modify it under
// the terms of the GNU General Public License as published by the Free Software
// Foundation, either version 3 of the License, or (at your option) any later
// version. See the LICENSE file for details.

use std::io;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, bail, Context, Result};
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use directories::{ProjectDirs, UserDirs};
use nix::sys::signal::{self, Signal};
use nix::unistd::Pid;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, HighlightSpacing, List, ListItem, ListState, Paragraph};

use captui::audio::{
    audio_target, parse_pw_dump, parse_sink_input_index, AudioSource, AudioTarget,
};
use captui::config::{parse_config, Config};
use captui::format::{format_duration, format_size};
use captui::meter::{meter_bar, samples_peak};
use captui::recorder::{
    concat_list_line, extension, ffmpeg_concat_argv, segment_path, timestamped_name,
    transcribe_argv, Backend, Mode,
};
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

fn load_config() -> Config {
    ProjectDirs::from("", "", "captui")
        .and_then(|d| std::fs::read_to_string(d.config_dir().join("config.toml")).ok())
        .map(|text| parse_config(&text))
        .unwrap_or_default()
}

fn expand_tilde(path: &str) -> PathBuf {
    match path.strip_prefix("~/") {
        Some(rest) => match std::env::var_os("HOME") {
            Some(home) => PathBuf::from(home).join(rest),
            None => PathBuf::from(path),
        },
        None => PathBuf::from(path),
    }
}

fn output_path(cfg: &Config, mode: Mode) -> Result<PathBuf> {
    let dir = match &cfg.output_dir {
        Some(d) => expand_tilde(d),
        None => UserDirs::new()
            .and_then(|u| u.video_dir().map(Path::to_path_buf))
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Videos")))
            .ok_or_else(|| anyhow!("could not determine a video directory"))?
            .join("captures"),
    };
    std::fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
    let ext = match mode {
        Mode::AudioVideo => cfg
            .container
            .clone()
            .unwrap_or_else(|| extension(Mode::AudioVideo).to_string()),
        Mode::AudioOnly => extension(Mode::AudioOnly).to_string(),
    };
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default();
    Ok(dir.join(timestamped_name(secs, &ext)))
}

fn capture_stderr(child: &mut Child) -> Arc<Mutex<String>> {
    let buf = Arc::new(Mutex::new(String::new()));
    if let Some(err) = child.stderr.take() {
        let shared = buf.clone();
        std::thread::spawn(move || {
            let mut text = String::new();
            let _ = std::io::BufReader::new(err).read_to_string(&mut text);
            if let Ok(mut b) = shared.lock() {
                *b = text;
            }
        });
    }
    buf
}

fn stderr_tail(buf: &Arc<Mutex<String>>) -> String {
    let line = buf
        .lock()
        .ok()
        .and_then(|b| {
            b.lines()
                .rev()
                .find(|l| !l.trim().is_empty())
                .map(str::to_string)
        })
        .unwrap_or_else(|| "no output produced".into());
    match line.char_indices().nth(200) {
        Some((cut, _)) => format!("{}…", &line[..cut]),
        None => line,
    }
}

fn spawn_audio_recorder(node: &str, out: &Path) -> Result<Child> {
    Command::new("pw-record")
        .arg(format!("--target={node}"))
        .arg(out)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .context("could not spawn pw-record")
}

fn spawn_recorder(
    backend: Backend,
    source: &Source,
    audio: Option<&str>,
    out: &Path,
    no_hw: bool,
) -> Result<Child> {
    let argv = backend.argv(source, audio, &out.to_string_lossy(), no_hw);
    Command::new(&argv[0])
        .args(&argv[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
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

const MIX_SINK: &str = "captui_mix";

struct AudioMix {
    modules: Vec<String>,
}

impl Drop for AudioMix {
    fn drop(&mut self) {
        for id in self.modules.iter().rev() {
            let _ = Command::new("pactl")
                .args(["unload-module", id])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
    }
}

fn pactl_load(args: &[&str]) -> Result<String> {
    let out = Command::new("pactl")
        .arg("load-module")
        .args(args)
        .output()
        .context("could not run pactl (is it installed?)")?;
    if !out.status.success() {
        bail!(
            "pactl load-module {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

struct MixSetup {
    monitor: String,
    mix: AudioMix,
    out_sink_input: Option<u32>,
    in_sink_input: Option<u32>,
}

fn load_loopback(source: &str, mix: &mut AudioMix) -> Result<String> {
    let id = pactl_load(&[
        "module-loopback",
        &format!("source={source}"),
        &format!("sink={MIX_SINK}"),
        "latency_msec=20",
    ])?;
    mix.modules.push(id.clone());
    Ok(id)
}

fn setup_mix(output: &str, input: &str) -> Result<MixSetup> {
    let sink = pactl_load(&[
        "module-null-sink",
        &format!("sink_name={MIX_SINK}"),
        "sink_properties=device.description=captui-mix",
    ])?;
    let mut mix = AudioMix {
        modules: vec![sink],
    };
    let out_module = load_loopback(output, &mut mix)?;
    let in_module = load_loopback(input, &mut mix)?;

    let sink_inputs = Command::new("pactl")
        .args(["list", "sink-inputs"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned());
    let idx = |module: &str| {
        sink_inputs
            .as_deref()
            .and_then(|t| parse_sink_input_index(t, module))
    };

    Ok(MixSetup {
        monitor: format!("{MIX_SINK}.monitor"),
        out_sink_input: idx(&out_module),
        in_sink_input: idx(&in_module),
        mix,
    })
}

fn set_sink_input_volume(idx: u32, pct: u16) {
    let _ = Command::new("pactl")
        .args([
            "set-sink-input-volume",
            &idx.to_string(),
            &format!("{pct}%"),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

struct Meter {
    child: Child,
    level: Arc<AtomicU32>,
    handle: Option<JoinHandle<()>>,
}

impl Meter {
    fn level(&self) -> f32 {
        f32::from_bits(self.level.load(Ordering::Relaxed))
    }
}

impl Drop for Meter {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn spawn_meter(node: &str) -> Option<Meter> {
    let mut child = Command::new("parec")
        .args([
            "--device",
            node,
            "--format=float32le",
            "--rate=48000",
            "--channels=1",
            "--latency-msec=30",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    let level = Arc::new(AtomicU32::new(0));
    let shared = level.clone();
    let handle = std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        let mut display = 0.0f32;
        while let Ok(n) = stdout.read(&mut buf) {
            if n == 0 {
                break;
            }
            display *= 0.8;
            display = display.max(samples_peak(&buf[..n]));
            shared.store(display.to_bits(), Ordering::Relaxed);
        }
    });
    Some(Meter {
        child,
        level,
        handle: Some(handle),
    })
}

struct SourceControl {
    label: &'static str,
    meter: Meter,
    sink_input: Option<u32>,
    volume: u16,
}

enum RecSpec {
    Av {
        backend: Backend,
        source: Source,
        audio: Option<String>,
        no_hw: bool,
    },
    Audio {
        node: String,
    },
}

impl RecSpec {
    fn spawn(&self, out: &Path) -> Result<Child> {
        match self {
            RecSpec::Av {
                backend,
                source,
                audio,
                no_hw,
            } => spawn_recorder(*backend, source, audio.as_deref(), out, *no_hw),
            RecSpec::Audio { node } => spawn_audio_recorder(node, out),
        }
    }
}

struct Rec {
    spec: RecSpec,
    path: PathBuf,
    segments: Vec<PathBuf>,
    child: Option<Child>,
    stderr: Arc<Mutex<String>>,
    stopped: bool,
    saved: bool,
    started: Instant,
    final_elapsed: Option<Duration>,
    paused: bool,
    pause_started: Option<Instant>,
    paused_total: Duration,
    sources: Vec<SourceControl>,
    focus: usize,
    mix: Option<AudioMix>,
}

impl Rec {
    fn elapsed(&self) -> Duration {
        if let Some(final_elapsed) = self.final_elapsed {
            return final_elapsed;
        }
        let mut paused = self.paused_total;
        if let Some(since) = self.pause_started {
            paused += since.elapsed();
        }
        self.started.elapsed().saturating_sub(paused)
    }

    fn size_bytes(&self) -> u64 {
        self.segments
            .iter()
            .filter_map(|p| std::fs::metadata(p).ok())
            .map(|m| m.len())
            .sum()
    }
}

enum Screen {
    Source,
    AudioOutput,
    AudioInput,
    Recording,
}

struct App {
    displays: Vec<Output>,
    source_list: ListState,
    pending_source: Option<Source>,
    output_options: Vec<Option<AudioSource>>,
    output_list: ListState,
    input_options: Vec<Option<AudioSource>>,
    input_list: ListState,
    chosen_output: Option<String>,
    audio_only: bool,
    screen: Screen,
    recording: Option<Rec>,
    status: Option<String>,
    config: Config,
    transcribe: bool,
}

impl App {
    fn new(displays: Vec<Output>, config: Config) -> Self {
        let mut source_list = ListState::default();
        if !displays.is_empty() {
            source_list.select(Some(0));
        }
        Self {
            displays,
            source_list,
            pending_source: None,
            output_options: Vec::new(),
            output_list: ListState::default(),
            input_options: Vec::new(),
            input_list: ListState::default(),
            chosen_output: None,
            audio_only: false,
            screen: Screen::Source,
            recording: None,
            status: None,
            config,
            transcribe: false,
        }
    }

    fn request_transcribe(&mut self) -> bool {
        let ready = self.config.transcribe_command.is_some()
            && matches!(&self.recording, Some(r) if r.stopped && r.saved);
        if ready {
            self.transcribe = true;
        }
        ready
    }

    fn active_list(&mut self) -> (&mut ListState, usize) {
        match self.screen {
            Screen::AudioOutput => (&mut self.output_list, self.output_options.len()),
            Screen::AudioInput => (&mut self.input_list, self.input_options.len()),
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
            self.audio_only = false;
            self.pending_source = Some(Source::Display(o.name.clone()));
            self.enter_output();
        }
    }

    fn choose_region(&mut self, src: Source) {
        self.audio_only = false;
        self.pending_source = Some(src);
        self.enter_output();
    }

    fn choose_audio_only(&mut self) {
        self.audio_only = true;
        self.pending_source = None;
        self.enter_output();
    }

    fn restart(&mut self) {
        self.recording = None;
        self.pending_source = None;
        self.chosen_output = None;
        self.audio_only = false;
        self.status = None;
        self.screen = Screen::Source;
    }

    fn enter_output(&mut self) {
        self.status = None;
        let sources = match enumerate_audio() {
            Ok(s) => s,
            Err(e) => {
                self.status = Some(format!("{e:#}"));
                Vec::new()
            }
        };
        let (monitors, mics): (Vec<_>, Vec<_>) = sources.into_iter().partition(|a| a.is_monitor);
        self.output_options = monitors
            .into_iter()
            .map(Some)
            .chain(std::iter::once(None))
            .collect();
        self.input_options = mics
            .into_iter()
            .map(Some)
            .chain(std::iter::once(None))
            .collect();
        // Preselect the configured sources; else system audio, and the default mic.
        let match_node = |opts: &[Option<AudioSource>], want: Option<&str>| {
            want.and_then(|w| {
                opts.iter()
                    .position(|o| o.as_ref().is_some_and(|a| a.node_name == w))
            })
        };
        let out_sel = match_node(&self.output_options, self.config.audio_output.as_deref());
        self.output_list.select(Some(out_sel.unwrap_or(0)));
        let default_mic = self.input_options.iter().position(|o| {
            o.as_ref()
                .is_some_and(|a| a.description.ends_with("(default)"))
        });
        let in_sel = match_node(&self.input_options, self.config.audio_input.as_deref())
            .or(default_mic)
            .unwrap_or(self.input_options.len().saturating_sub(1));
        self.input_list.select(Some(in_sel));
        self.screen = Screen::AudioOutput;
    }

    fn confirm_output(&mut self) {
        self.chosen_output = self
            .output_list
            .selected()
            .and_then(|i| self.output_options.get(i))
            .and_then(|c| c.as_ref().map(|a| a.node_name.clone()));
        self.screen = Screen::AudioInput;
    }

    fn back_to_source(&mut self) {
        self.pending_source = None;
        self.status = None;
        self.screen = Screen::Source;
    }

    fn back_to_output(&mut self) {
        self.status = None;
        self.screen = Screen::AudioOutput;
    }

    fn start_recording(&mut self) {
        let input = self
            .input_list
            .selected()
            .and_then(|i| self.input_options.get(i))
            .and_then(|c| c.as_ref().map(|a| a.node_name.clone()));
        let output = self.chosen_output.clone();
        let target = audio_target(output.as_deref(), input.as_deref());

        if self.audio_only && target == AudioTarget::Silent {
            self.status = Some("audio-only needs an output or an input".into());
            return;
        }

        let (audio_node, mix, out_input, in_input) = match target {
            AudioTarget::Silent => (None, None, None, None),
            AudioTarget::Single(node) => (Some(node), None, None, None),
            AudioTarget::Mix { output, input } => match setup_mix(&output, &input) {
                Ok(s) => (
                    Some(s.monitor),
                    Some(s.mix),
                    s.out_sink_input,
                    s.in_sink_input,
                ),
                Err(e) => {
                    self.status = Some(format!("{e:#}"));
                    return;
                }
            },
        };

        let mode = if self.audio_only {
            Mode::AudioOnly
        } else {
            Mode::AudioVideo
        };
        let path = match output_path(&self.config, mode) {
            Ok(p) => p,
            Err(e) => {
                self.status = Some(format!("{e:#}"));
                return;
            }
        };

        let spec = if self.audio_only {
            match audio_node {
                Some(node) => RecSpec::Audio { node },
                None => return,
            }
        } else {
            match self.pending_source.clone() {
                Some(source) => RecSpec::Av {
                    backend: Backend::from_config(self.config.backend.as_deref()),
                    source,
                    audio: audio_node,
                    no_hw: self.config.no_hw,
                },
                None => return,
            }
        };

        let seg = segment_path(&path, 0);
        match spec.spawn(&seg) {
            Ok(mut child) => {
                let stderr = capture_stderr(&mut child);
                let mut sources = Vec::new();
                if let Some(meter) = output.as_deref().and_then(spawn_meter) {
                    sources.push(SourceControl {
                        label: "output",
                        meter,
                        sink_input: out_input,
                        volume: 100,
                    });
                }
                if let Some(meter) = input.as_deref().and_then(spawn_meter) {
                    sources.push(SourceControl {
                        label: "input",
                        meter,
                        sink_input: in_input,
                        volume: 100,
                    });
                }
                self.pending_source = None;
                self.recording = Some(Rec {
                    spec,
                    path,
                    segments: vec![seg],
                    child: Some(child),
                    stderr,
                    stopped: false,
                    saved: false,
                    started: Instant::now(),
                    final_elapsed: None,
                    paused: false,
                    pause_started: None,
                    paused_total: Duration::ZERO,
                    sources,
                    focus: 0,
                    mix,
                });
                self.status = None;
                self.screen = Screen::Recording;
            }
            Err(e) => {
                self.status = Some(format!("{e:#}"));
            }
        }
    }

    fn toggle_pause(&mut self) {
        let Some(rec) = self.recording.as_mut() else {
            return;
        };
        if rec.stopped {
            return;
        }
        if rec.paused {
            let seg = segment_path(&rec.path, rec.segments.len());
            match rec.spec.spawn(&seg) {
                Ok(mut child) => {
                    rec.stderr = capture_stderr(&mut child);
                    rec.child = Some(child);
                    rec.segments.push(seg);
                    if let Some(since) = rec.pause_started.take() {
                        rec.paused_total += since.elapsed();
                    }
                    rec.paused = false;
                }
                Err(e) => self.status = Some(format!("resume failed: {e:#}")),
            }
        } else {
            if let Some(mut child) = rec.child.take() {
                let _ = stop_recorder(&mut child);
            }
            rec.paused = true;
            rec.pause_started = Some(Instant::now());
        }
    }

    fn focus_source(&mut self, delta: isize) {
        if let Some(rec) = self.recording.as_mut() {
            if !rec.sources.is_empty() {
                let len = rec.sources.len() as isize;
                rec.focus = (rec.focus as isize + delta).rem_euclid(len) as usize;
            }
        }
    }

    fn adjust_volume(&mut self, delta: i16) {
        if let Some(rec) = self.recording.as_mut() {
            if let Some(src) = rec.sources.get_mut(rec.focus) {
                if let Some(idx) = src.sink_input {
                    src.volume = (src.volume as i16 + delta).clamp(0, 150) as u16;
                    set_sink_input_volume(idx, src.volume);
                }
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
        if rec.paused {
            if let Some(since) = rec.pause_started.take() {
                rec.paused_total += since.elapsed();
            }
            rec.paused = false;
        }
        self.finalize();
    }

    fn poll_recorder(&mut self) {
        let Some(rec) = self.recording.as_mut() else {
            return;
        };
        if rec.stopped || rec.paused {
            return;
        }
        // The recorder exiting on its own means it crashed or refused to start.
        let died = matches!(rec.child.as_mut().map(Child::try_wait), Some(Ok(Some(_))));
        if died {
            rec.child = None;
            self.finalize();
        }
    }

    fn finalize(&mut self) {
        let Some(rec) = self.recording.as_mut() else {
            return;
        };
        rec.final_elapsed = Some(rec.elapsed());
        rec.sources.clear();
        if let Some(mut child) = rec.child.take() {
            let _ = stop_recorder(&mut child);
        }
        rec.mix = None;
        rec.saved = concat_segments(&rec.segments, &rec.path);
        rec.stopped = true;
        self.status = if rec.saved {
            None
        } else {
            Some(format!("recording failed: {}", stderr_tail(&rec.stderr)))
        };
    }
}

fn concat_segments(segments: &[PathBuf], out: &Path) -> bool {
    let present: Vec<&PathBuf> = segments
        .iter()
        .filter(|p| std::fs::metadata(p).map(|m| m.len() > 0).unwrap_or(false))
        .collect();
    match present.as_slice() {
        [] => false,
        [only] => std::fs::rename(only, out).is_ok(),
        many => {
            let list = out.with_extension("captui-concat.txt");
            let body: String = many
                .iter()
                .map(|p| concat_list_line(&p.to_string_lossy()))
                .collect();
            if std::fs::write(&list, body).is_err() {
                return false;
            }
            let argv = ffmpeg_concat_argv(&list.to_string_lossy(), &out.to_string_lossy());
            let ok = Command::new(&argv[0])
                .args(&argv[1..])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
            let _ = std::fs::remove_file(&list);
            if ok {
                for seg in many {
                    let _ = std::fs::remove_file(seg);
                }
            }
            ok && std::fs::metadata(out).map(|m| m.len() > 0).unwrap_or(false)
        }
    }
}

fn main() -> Result<()> {
    let displays = enumerate_displays();
    let config = load_config();

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout))?;

    let res = run(&mut terminal, displays, config);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    let app = res?;
    if let Some(rec) = &app.recording {
        if rec.saved {
            println!("recording saved: {}", rec.path.display());
            if app.transcribe {
                run_transcribe(&app.config, &rec.path);
            }
        } else {
            eprintln!("recording failed: {}", stderr_tail(&rec.stderr));
        }
    }
    Ok(())
}

fn run_transcribe(cfg: &Config, path: &Path) {
    let Some(template) = &cfg.transcribe_command else {
        return;
    };
    let Some(argv) = transcribe_argv(template, &path.to_string_lossy()) else {
        return;
    };
    println!("transcribing: {}", argv.join(" "));
    match Command::new(&argv[0]).args(&argv[1..]).status() {
        Ok(s) if s.success() => {}
        Ok(s) => eprintln!("transcribe command exited with {s}"),
        Err(e) => eprintln!("could not run transcribe command: {e}"),
    }
}

fn run(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    displays: Result<Vec<Output>>,
    config: Config,
) -> Result<App> {
    let (mut app, error) = match displays {
        Ok(d) => (App::new(d, config), None),
        Err(e) => (App::new(Vec::new(), config), Some(format!("{e:#}"))),
    };

    loop {
        app.poll_recorder();
        terminal.draw(|f| draw(f, &mut app, error.as_deref()))?;

        if !event::poll(Duration::from_millis(50))? {
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
                KeyCode::Char('a') => app.choose_audio_only(),
                KeyCode::Enter => app.choose_display(),
                _ => {}
            },
            Screen::AudioOutput => match k.code {
                KeyCode::Char('q') => return Ok(app),
                KeyCode::Esc => app.back_to_source(),
                KeyCode::Down | KeyCode::Char('j') => app.move_by(1),
                KeyCode::Up | KeyCode::Char('k') => app.move_by(-1),
                KeyCode::Enter => app.confirm_output(),
                _ => {}
            },
            Screen::AudioInput => match k.code {
                KeyCode::Char('q') => return Ok(app),
                KeyCode::Esc => app.back_to_output(),
                KeyCode::Down | KeyCode::Char('j') => app.move_by(1),
                KeyCode::Up | KeyCode::Char('k') => app.move_by(-1),
                KeyCode::Enter => app.start_recording(),
                _ => {}
            },
            Screen::Recording => match k.code {
                KeyCode::Char('s') => app.stop(),
                KeyCode::Char('p') => app.toggle_pause(),
                KeyCode::Up | KeyCode::Char('k') => app.focus_source(-1),
                KeyCode::Down | KeyCode::Char('j') => app.focus_source(1),
                KeyCode::Left | KeyCode::Char('h') => app.adjust_volume(-5),
                KeyCode::Right | KeyCode::Char('l') => app.adjust_volume(5),
                KeyCode::Char('t') => {
                    if app.request_transcribe() {
                        return Ok(app);
                    }
                }
                KeyCode::Char('n') => {
                    if matches!(&app.recording, Some(r) if r.stopped) {
                        app.restart();
                    }
                }
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
        None => "None".into(),
        Some(a) => a.description.clone(),
    }
}

fn draw(f: &mut Frame, app: &mut App, error: Option<&str>) {
    let chunks = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).split(f.area());
    match app.screen {
        Screen::Source => draw_source(f, app, error, chunks[0]),
        Screen::AudioOutput => draw_audio_list(
            f,
            " captui - output (system audio) ",
            &app.output_options,
            &mut app.output_list,
            chunks[0],
        ),
        Screen::AudioInput => draw_audio_list(
            f,
            " captui - input (microphone) ",
            &app.input_options,
            &mut app.input_list,
            chunks[0],
        ),
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

fn draw_audio_list(
    f: &mut Frame,
    title: &str,
    options: &[Option<AudioSource>],
    state: &mut ListState,
    area: Rect,
) {
    let block = Block::default().title(title).borders(Borders::ALL);
    let items: Vec<ListItem> = options
        .iter()
        .map(|c| ListItem::new(audio_label(c)))
        .collect();
    let list = List::new(items)
        .block(block)
        .highlight_symbol("> ")
        .highlight_spacing(HighlightSpacing::Always)
        .highlight_style(Style::new().reversed());
    f.render_stateful_widget(list, area, state);
}

fn draw_recording(f: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .title(" captui - recording ")
        .borders(Borders::ALL);
    let body = match &app.recording {
        Some(rec) => {
            let timer = format_duration(rec.elapsed().as_secs());
            let size = format_size(rec.size_bytes());
            let tag = if app.audio_only { " (audio)" } else { "" };
            let head = if rec.stopped && !rec.saved {
                Line::from("✗ recording failed".red().bold())
            } else if rec.stopped {
                Line::from(vec![format!("■ stopped{tag}  ").green(), timer.into()])
            } else if rec.paused {
                Line::from(vec![
                    format!("❚❚ PAUSED{tag}  ").yellow().bold(),
                    timer.into(),
                ])
            } else {
                Line::from(vec![format!("● REC{tag}  ").red().bold(), timer.into()])
            };
            let mut lines = vec![head, Line::from(format!("size: {size}"))];
            let adjustable = !rec.stopped && rec.sources.iter().any(|s| s.sink_input.is_some());
            for (i, src) in rec.sources.iter().enumerate() {
                let marker = if adjustable && i == rec.focus {
                    ">"
                } else {
                    " "
                };
                let vol = match src.sink_input {
                    Some(_) if !rec.stopped => format!("  {:>3}%", src.volume),
                    _ => String::new(),
                };
                lines.push(Line::from(format!(
                    "{marker} {:>6}: {}{vol}",
                    src.label,
                    meter_bar(src.meter.level(), 24)
                )));
            }
            let file_label = if rec.saved { "saved" } else { "file" };
            lines.push(Line::from(format!("{file_label}: {}", rec.path.display())));
            if rec.saved && app.config.transcribe_command.is_some() {
                lines.push(Line::from("press t to transcribe".cyan()));
            }
            Text::from(lines)
        }
        None => Text::from("not recording"),
    };
    f.render_widget(Paragraph::new(body).block(block), area);
}

fn draw_footer(f: &mut Frame, app: &App, area: Rect) {
    let stopped = matches!(&app.recording, Some(r) if r.stopped);
    let paused = matches!(&app.recording, Some(r) if r.paused);
    let adjustable = matches!(&app.recording, Some(r) if !r.stopped && !r.paused && r.sources.iter().any(|s| s.sink_input.is_some()));
    let hint = match app.screen {
        Screen::Source => {
            " up/down move  i identify  enter display  r region  a audio-only  q quit "
        }
        Screen::AudioOutput => " up/down move  enter next (input)  esc back  q quit ",
        Screen::AudioInput => " up/down move  enter record  esc back  q quit ",
        Screen::Recording if stopped => {
            let saved = matches!(&app.recording, Some(r) if r.saved);
            if saved && app.config.transcribe_command.is_some() {
                " n new recording  t transcribe  q quit "
            } else {
                " n new recording  q quit "
            }
        }
        Screen::Recording if paused => " p resume  s stop  q stop and quit ",
        Screen::Recording if adjustable => {
            " s stop  p pause  up/down focus  left/right volume  q stop and quit "
        }
        Screen::Recording => " s stop  p pause  q stop and quit ",
    };
    let footer = match &app.status {
        Some(s) => Paragraph::new(format!(" {s} ")).style(Style::new().yellow()),
        None => Paragraph::new(hint).style(Style::new().dim()),
    };
    f.render_widget(footer, area);
}
