mod audio;
mod library;
mod state;
mod tempo;

use audio::Audio;
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers,
        MouseButton, MouseEvent, MouseEventKind,
    },
    execute,
};
use ratatui::{
    DefaultTerminal, Frame,
    layout::{Constraint, Layout, Position, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, List, ListItem, ListState, Paragraph, Wrap},
};
use state::State;
use std::{
    env, io,
    path::PathBuf,
    time::{Duration, Instant},
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
const ACCENT: Color = Color::Rgb(216, 180, 125);
struct MouseCapture;
impl MouseCapture {
    fn enable() -> io::Result<Self> {
        execute!(io::stdout(), EnableMouseCapture)?;
        Ok(Self)
    }
}
impl Drop for MouseCapture {
    fn drop(&mut self) {
        let _ = execute!(io::stdout(), DisableMouseCapture);
    }
}
#[derive(Default)]
struct MouseTargets {
    library: Rect,
    progress: Rect,
    controls: Vec<(Rect, KeyCode)>,
}
struct App {
    files: Vec<PathBuf>,
    selected: usize,
    active: Option<usize>,
    audio: Option<Audio>,
    state: State,
    state_path: PathBuf,
    speed: f32,
    volume: f32,
    sleep: Option<Instant>,
    message: String,
    query: String,
    searching: bool,
    last_save: Instant,
    list_offset: usize,
    mouse_targets: MouseTargets,
}
impl App {
    fn visible(&self) -> Vec<usize> {
        self.files
            .iter()
            .enumerate()
            .filter(|(_, p)| {
                p.to_string_lossy()
                    .to_lowercase()
                    .contains(&self.query.to_lowercase())
            })
            .map(|(i, _)| i)
            .collect()
    }
    fn remember(&mut self) {
        self.state.speed = self.speed;
        self.state.volume = self.volume;
        if let (Some(index), Some(audio)) = (self.active, &self.audio) {
            let position = if audio.player.empty() {
                0.0
            } else {
                audio.position()
            };
            self.state
                .positions
                .insert(self.files[index].clone(), position);
            self.state.last = Some(self.files[index].clone());
        }
    }
    fn save(&mut self) -> io::Result<()> {
        self.remember();
        self.state.save(&self.state_path)?;
        self.last_save = Instant::now();
        Ok(())
    }
    fn play(&mut self, index: usize, paused: bool) -> Result<()> {
        self.save()?;
        if self.audio.is_none() {
            self.audio = Some(Audio::open()?);
        }
        let position = self
            .state
            .positions
            .get(&self.files[index])
            .copied()
            .unwrap_or(0.0);
        // Clear the active marker before loading so a failed seek cannot overwrite another file's progress.
        self.active = None;
        let audio = self.audio.as_mut().unwrap();
        if let Err(error) = audio.load(
            &self.files[index],
            position,
            self.speed,
            self.volume,
            paused,
        ) {
            audio.player.clear();
            return Err(error);
        }
        self.active = Some(index);
        self.save()?;
        self.message = if paused {
            "Restored your place • press Space or click Play to continue"
        } else {
            "Listening • progress saves every second"
        }
        .into();
        Ok(())
    }
    fn key(&mut self, code: KeyCode) -> Result<bool> {
        if self.searching {
            match code {
                KeyCode::Esc => {
                    self.searching = false;
                    self.query.clear();
                }
                KeyCode::Enter => self.searching = false,
                KeyCode::Backspace => {
                    self.query.pop();
                    self.selected = 0;
                }
                KeyCode::Char(c) => {
                    self.query.push(c);
                    self.selected = 0;
                }
                _ => {}
            }
            return Ok(false);
        }
        let visible = self.visible();
        match code {
            KeyCode::Char('q') | KeyCode::Esc => return Ok(true),
            KeyCode::Down | KeyCode::Char('j') => {
                self.selected = (self.selected + 1).min(visible.len().saturating_sub(1))
            }
            KeyCode::Up | KeyCode::Char('k') => self.selected = self.selected.saturating_sub(1),
            KeyCode::Home => self.selected = 0,
            KeyCode::End => self.selected = visible.len().saturating_sub(1),
            KeyCode::Char('/') => {
                self.searching = true;
            }
            KeyCode::Enter => {
                if let Some(&index) = visible.get(self.selected) {
                    self.play(index, false)?;
                }
            }
            KeyCode::Char(' ') => {
                if let Some(a) = &self.audio {
                    if a.player.is_paused() {
                        a.player.play();
                    } else {
                        a.player.pause();
                    }
                }
            }
            KeyCode::Left => {
                if let Some(a) = &self.audio {
                    a.seek(-15.0)?;
                }
            }
            KeyCode::Right => {
                if let Some(a) = &self.audio {
                    a.seek(30.0)?;
                }
            }
            KeyCode::Char('[') | KeyCode::Char(']') => {
                self.speed = (self.speed
                    + if code == KeyCode::Char(']') {
                        0.1
                    } else {
                        -0.1
                    })
                .clamp(0.5, 3.0);
                if let Some(a) = &self.audio {
                    a.set_speed(self.speed);
                }
            }
            KeyCode::Char('-') | KeyCode::Char('+') | KeyCode::Char('=') => {
                self.volume = (self.volume
                    + if code == KeyCode::Char('-') {
                        -0.05
                    } else {
                        0.05
                    })
                .clamp(0.0, 1.0);
                if let Some(a) = &self.audio {
                    a.player.set_volume(self.volume);
                }
            }
            KeyCode::Char('s') => {
                self.sleep = if self.sleep.is_some() {
                    None
                } else {
                    Some(Instant::now() + Duration::from_secs(30 * 60))
                };
                self.message = if self.sleep.is_some() {
                    "Sleep timer: 30 minutes"
                } else {
                    "Sleep timer canceled"
                }
                .into();
            }
            KeyCode::Char('n') | KeyCode::Char('p') => {
                if let Some(index) = self.active {
                    let next = if code == KeyCode::Char('n') {
                        (index + 1).min(self.files.len() - 1)
                    } else {
                        index.saturating_sub(1)
                    };
                    self.play(next, false)?;
                }
            }
            _ => {}
        }
        if matches!(
            code,
            KeyCode::Char(' ')
                | KeyCode::Left
                | KeyCode::Right
                | KeyCode::Char('[')
                | KeyCode::Char(']')
                | KeyCode::Char('-')
                | KeyCode::Char('+')
                | KeyCode::Char('=')
                | KeyCode::Char('s')
        ) {
            self.save()?;
        }
        Ok(false)
    }
    fn library_hit(&self, position: Position) -> Option<(usize, usize)> {
        if !self.mouse_targets.library.contains(position) {
            return None;
        }
        let row = self.list_offset + usize::from(position.y - self.mouse_targets.library.y);
        self.visible().get(row).copied().map(|index| (row, index))
    }
    fn mouse(&mut self, event: MouseEvent) -> Result<bool> {
        let position = Position::new(event.column, event.row);
        let targets = &self.mouse_targets;
        match event.kind {
            MouseEventKind::ScrollDown if targets.library.contains(position) => {
                self.selected = (self.selected + 1).min(self.visible().len().saturating_sub(1));
            }
            MouseEventKind::ScrollUp if targets.library.contains(position) => {
                self.selected = self.selected.saturating_sub(1);
            }
            MouseEventKind::Down(MouseButton::Left) if targets.library.contains(position) => {
                if let Some((row, index)) = self.library_hit(position) {
                    self.selected = row;
                    self.play(index, false)?;
                }
            }
            MouseEventKind::Down(MouseButton::Left) | MouseEventKind::Drag(MouseButton::Left)
                if targets.progress.contains(position) =>
            {
                if let Some(audio) = &self.audio
                    && let Some(duration) = audio.duration
                {
                    let ratio = f64::from(event.column - targets.progress.x)
                        / f64::from(targets.progress.width.saturating_sub(1).max(1));
                    audio.seek_to(duration.as_secs_f64() * ratio)?;
                    self.save()?;
                }
            }
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some((_, key)) = targets
                    .controls
                    .iter()
                    .find(|(area, _)| area.contains(position))
                {
                    let key = *key;
                    // Clicked controls remain usable while a search is being edited.
                    let searching = self.searching;
                    self.searching = false;
                    let result = self.key(key);
                    self.searching = searching;
                    return result;
                }
            }
            _ => {}
        }
        Ok(false)
    }
    fn tick(&mut self) {
        if self
            .sleep
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            self.sleep = None;
            if let Some(a) = &self.audio {
                a.player.pause();
            }
            self.message = "Sleep timer finished • playback paused".into();
            if let Err(e) = self.save() {
                self.message = format!("Cannot save progress: {e}");
            }
        }
        if let (Some(index), Some(audio)) = (self.active, &self.audio)
            && audio.player.empty()
        {
            self.state.positions.insert(self.files[index].clone(), 0.0);
            self.active = None;
            if index + 1 < self.files.len() {
                if let Err(e) = self.play(index + 1, false) {
                    self.message = e.to_string();
                }
            } else {
                self.message = "Finished the library".into();
            }
        }
        if self.last_save.elapsed() >= Duration::from_secs(1)
            && let Err(e) = self.save()
        {
            self.message = format!("Cannot save progress: {e}");
            self.last_save = Instant::now();
        }
    }
    fn draw(&mut self, frame: &mut Frame) {
        let areas = Layout::vertical([
            Constraint::Length(3),
            Constraint::Min(3),
            Constraint::Length(9),
            Constraint::Length(4),
        ])
        .split(frame.area());
        let header = Paragraph::new(Line::from(vec![
            Span::styled(
                "  cozyread  ",
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!(
                "a little space to listen    {} files",
                self.files.len()
            )),
        ]))
        .block(Block::default().borders(Borders::BOTTOM));
        frame.render_widget(header, areas[0]);
        self.mouse_targets = MouseTargets {
            library: Block::bordered().inner(areas[1]),
            ..Default::default()
        };
        let visible = self.visible();
        let items: Vec<ListItem> = visible
            .iter()
            .map(|&i| {
                let path = &self.files[i];
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                let folder = path
                    .parent()
                    .and_then(|p| p.file_name())
                    .unwrap_or_default()
                    .to_string_lossy();
                let marker = if self.active == Some(i) { "▶ " } else { "  " };
                let saved = self.state.positions.get(path).copied().unwrap_or(0.0);
                ListItem::new(Line::from(vec![
                    Span::styled(
                        format!("{marker}{name}"),
                        Style::default().fg(if self.active == Some(i) {
                            ACCENT
                        } else {
                            Color::White
                        }),
                    ),
                    Span::styled(
                        format!(
                            "   {folder}{}",
                            if saved > 0.0 {
                                format!(" • {}", clock(saved))
                            } else {
                                String::new()
                            }
                        ),
                        Style::default().fg(Color::DarkGray),
                    ),
                ]))
            })
            .collect();
        let title = if self.searching {
            format!(" Search: {}▏ ", self.query)
        } else if !self.query.is_empty() {
            format!(" Library • filter: {} • / to edit ", self.query)
        } else {
            " Library • / to search ".into()
        };
        if visible.is_empty() {
            frame.render_widget(
                Paragraph::new("No matching audio files. Start cozyread with a file or folder.")
                    .wrap(Wrap { trim: true })
                    .block(Block::bordered().title(title)),
                areas[1],
            );
        } else {
            let list = List::new(items)
                .block(Block::bordered().title(title))
                .highlight_style(Style::default().bg(Color::Rgb(45, 43, 39)))
                .highlight_symbol("› ");
            let mut selection = ListState::default()
                .with_selected(Some(self.selected.min(visible.len() - 1)))
                .with_offset(self.list_offset);
            frame.render_stateful_widget(list, areas[1], &mut selection);
            self.list_offset = selection.offset();
        }
        let panel = Block::bordered().title(" Now playing ");
        let inner = panel.inner(areas[2]);
        frame.render_widget(panel, areas[2]);
        let rows = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(inner);
        let name = self
            .active
            .map(|i| {
                self.files[i]
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned()
            })
            .unwrap_or_else(|| "Choose a book and press Enter".into());
        frame.render_widget(
            Paragraph::new(name).style(Style::default().fg(ACCENT)),
            rows[0],
        );
        let (pos, duration, paused) = self
            .audio
            .as_ref()
            .map(|a| {
                (
                    a.position(),
                    a.duration.map(|d| d.as_secs_f64()),
                    a.player.is_paused(),
                )
            })
            .unwrap_or((0.0, None, true));
        let ratio = duration
            .filter(|d| *d > 0.0)
            .map_or(0.0, |d| (pos / d).clamp(0.0, 1.0));
        self.mouse_targets.progress = rows[1];
        frame.render_widget(
            Gauge::default()
                .gauge_style(Style::default().fg(ACCENT))
                .ratio(ratio)
                .label(format!(
                    "{} / {}",
                    clock(pos),
                    duration.map(clock).unwrap_or_else(|| "--:--".into())
                )),
            rows[1],
        );
        let timer = self
            .sleep
            .map(|t| {
                format!(
                    " • sleep {}",
                    clock(t.saturating_duration_since(Instant::now()).as_secs_f64())
                )
            })
            .unwrap_or_default();
        frame.render_widget(
            Paragraph::new(format!(
                "{}   {:.1}×   volume {:.0}%{timer}",
                if self.active.is_none() {
                    "Stopped"
                } else if paused {
                    "Paused"
                } else {
                    "Playing"
                },
                self.speed,
                self.volume * 100.0
            )),
            rows[2],
        );
        self.draw_buttons(
            frame,
            rows[3],
            &[
                ("[Prev]", KeyCode::Char('p')),
                ("[-15s]", KeyCode::Left),
                (
                    if paused { "[Play]" } else { "[Pause]" },
                    KeyCode::Char(' '),
                ),
                ("[+30s]", KeyCode::Right),
                ("[Next]", KeyCode::Char('n')),
            ],
        );
        self.draw_buttons(
            frame,
            rows[4],
            &[
                ("[Speed-]", KeyCode::Char('[')),
                ("[Speed+]", KeyCode::Char(']')),
                ("[Vol-]", KeyCode::Char('-')),
                ("[Vol+]", KeyCode::Char('+')),
                ("[Sleep]", KeyCode::Char('s')),
                ("[Quit]", KeyCode::Char('q')),
            ],
        );
        frame.render_widget(
            Paragraph::new(self.message.as_str()).style(Style::default().fg(Color::DarkGray)),
            rows[5],
        );
        frame.render_widget(Paragraph::new("↑/↓ j/k browse  Enter play  Space pause  ← −15s  → +30s\n[ / ] speed  −/+ volume  n/p next/previous  s sleep 30m  / search  q quit\nClick a file to play • click/drag progress to seek • scroll to browse").style(Style::default().fg(Color::Gray)).wrap(Wrap { trim: true }), areas[3]);
    }
    fn draw_buttons(&mut self, frame: &mut Frame, area: Rect, buttons: &[(&str, KeyCode)]) {
        let mut x = area.x;
        for &(label, key) in buttons {
            let width = label.len() as u16;
            if x.saturating_add(width) > area.right() || area.height == 0 {
                break;
            }
            let hit = Rect::new(x, area.y, width, 1);
            frame.render_widget(
                Paragraph::new(label).style(Style::default().fg(ACCENT)),
                hit,
            );
            self.mouse_targets.controls.push((hit, key));
            x += width + 1;
        }
    }
}
fn clock(seconds: f64) -> String {
    let seconds = seconds.max(0.0) as u64;
    format!(
        "{}:{:02}:{:02}",
        seconds / 3600,
        seconds / 60 % 60,
        seconds % 60
    )
}
fn run(terminal: &mut DefaultTerminal, app: &mut App) -> Result<()> {
    loop {
        app.tick();
        terminal.draw(|f| app.draw(f))?;
        if event::poll(Duration::from_millis(150))? {
            let result = match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    if key.modifiers.contains(KeyModifiers::CONTROL)
                        && key.code == KeyCode::Char('c')
                    {
                        break;
                    }
                    app.key(key.code)
                }
                Event::Mouse(mouse) => app.mouse(mouse),
                _ => Ok(false),
            };
            match result {
                Ok(true) => break,
                Ok(false) => {}
                Err(e) => app.message = e.to_string(),
            }
        }
    }
    Ok(())
}
fn main() -> Result<()> {
    let mut args = env::args_os().skip(1);
    let argument = args.next();
    let root = argument.clone().unwrap_or_else(|| ".".into());
    if root == "--help" || root == "-h" {
        println!(
            "cozyread [FILE_OR_FOLDER]\n\nListen to local audiobooks in your terminal. Reopens the last library, or defaults to the current folder.\nSupports MP3, M4B/M4A (AAC/ALAC), FLAC, Ogg Vorbis, and WAV.\nProgress: $XDG_STATE_HOME/cozyread or ~/.local/state/cozyread.\nOnly one instance should use a progress file at a time."
        );
        return Ok(());
    }
    if args.next().is_some() {
        return Err("Usage: cozyread [FILE_OR_FOLDER]".into());
    }
    let state_path = state::location()?;
    let mut state = State::load(&state_path).map_err(|e| {
        format!(
            "Cannot read {}: {e}. Move it aside to start fresh.",
            state_path.display()
        )
    })?;
    let root = argument
        .map(PathBuf::from)
        .or_else(|| state.library.clone())
        .unwrap_or_else(|| PathBuf::from("."));
    let root = root.canonicalize()?;
    let files = library::scan(&root)?;
    state.library = Some(root);
    let speed = if state.speed.is_finite() {
        state.speed.clamp(0.5, 3.0)
    } else {
        1.0
    };
    let volume = if state.volume.is_finite() {
        state.volume.clamp(0.0, 1.0)
    } else {
        0.8
    };
    let selected = state
        .last
        .as_ref()
        .and_then(|last| files.iter().position(|f| f == last))
        .unwrap_or(0);
    let mut app = App {
        files,
        selected,
        active: None,
        audio: None,
        state,
        state_path,
        speed,
        volume,
        sleep: None,
        message: "Welcome back. Select a file to resume.".into(),
        query: String::new(),
        searching: false,
        last_save: Instant::now(),
        list_offset: 0,
        mouse_targets: MouseTargets::default(),
    };
    if app
        .state
        .last
        .as_ref()
        .is_some_and(|last| app.files.get(selected) == Some(last))
        && let Err(e) = app.play(selected, true)
    {
        app.message = format!("Cannot restore playback: {e}");
    }
    let mut terminal = ratatui::init();
    let result = (|| -> Result<()> {
        let _mouse = MouseCapture::enable()?;
        run(&mut terminal, &mut app)
    })();
    ratatui::restore();
    let saved = app.save();
    result?;
    saved?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn empty_library_keys_and_small_terminal_are_safe() {
        let mut app = App {
            files: vec![],
            selected: 0,
            active: None,
            audio: None,
            state: State::default(),
            state_path: PathBuf::from("unused"),
            speed: 1.0,
            volume: 0.8,
            sleep: None,
            message: String::new(),
            query: String::new(),
            searching: false,
            last_save: Instant::now(),
            list_offset: 0,
            mouse_targets: MouseTargets::default(),
        };
        for key in [
            KeyCode::Down,
            KeyCode::End,
            KeyCode::Enter,
            KeyCode::Char('n'),
        ] {
            app.key(key).unwrap();
        }
        for (width, height) in [(80, 24), (20, 6), (1, 1)] {
            let backend = ratatui::backend::TestBackend::new(width, height);
            let mut terminal = ratatui::Terminal::new(backend).unwrap();
            terminal.draw(|f| app.draw(f)).unwrap();
        }
    }
    #[test]
    fn mouse_scroll_hit_testing_and_controls_follow_rendered_layout() {
        let dir = env::temp_dir().join(format!("cozyread-mouse-test-{}", std::process::id()));
        let mut app = App {
            files: (0..50)
                .map(|i| PathBuf::from(format!("/books/chapter-{i:02}.wav")))
                .collect(),
            selected: 30,
            active: None,
            audio: None,
            state: State::default(),
            state_path: dir.join("progress.json"),
            speed: 1.0,
            volume: 0.8,
            sleep: None,
            message: String::new(),
            query: String::new(),
            searching: false,
            last_save: Instant::now(),
            list_offset: 0,
            mouse_targets: MouseTargets::default(),
        };
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 24)).unwrap();
        terminal.draw(|f| app.draw(f)).unwrap();
        assert!(app.list_offset > 0);
        let area = app.mouse_targets.library;
        assert_eq!(
            app.library_hit(Position::new(area.x, area.y)),
            Some((app.list_offset, app.list_offset))
        );
        assert_eq!(app.library_hit(Position::new(area.x, area.y - 1)), None);
        let mouse = |kind, column, row| MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        };
        app.mouse(mouse(MouseEventKind::ScrollDown, area.x, area.y))
            .unwrap();
        assert_eq!(app.selected, 31);
        app.mouse(mouse(MouseEventKind::ScrollUp, area.x, area.y))
            .unwrap();
        assert_eq!(app.selected, 30);
        let &(button, _) = app
            .mouse_targets
            .controls
            .iter()
            .find(|(_, key)| *key == KeyCode::Char(']'))
            .unwrap();
        app.searching = true;
        app.mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            button.x,
            button.y,
        ))
        .unwrap();
        assert!(app.searching);
        assert!((app.speed - 1.1).abs() < 0.001);
        assert!((State::load(&app.state_path).unwrap().speed - 1.1).abs() < 0.001);
        app.query = "chapter-49".into();
        app.selected = 0;
        terminal.draw(|f| app.draw(f)).unwrap();
        let area = app.mouse_targets.library;
        assert_eq!(
            app.library_hit(Position::new(area.x, area.y)),
            Some((0, 49))
        );
        assert_eq!(app.library_hit(Position::new(area.x, area.y + 1)), None);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
