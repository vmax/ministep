//! The engine: shared state, command dispatch, MIDI input handling, and the
//! real-time transport thread.
//!
//! Every entry point (`execute`, `handle_midi`, `edit`, ...) is `&self` and safe
//! to call from any thread: the GUI, the MIDI input callback, or the remote
//! socket. Lock order is always control → transport → state → output.

use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender, unbounded};
use parking_lot::{Mutex, MutexGuard, RwLock};

use crate::clock::monotonic;
use crate::controller::{Command, CommandName, Mapping, absolute_range};
use crate::message::MidiMessage;
use crate::output::{Owner, SharedOutput};
use crate::patterns::{PatternError, PatternKind, PatternStore, default_config_dir};
use crate::profiles::{ControlProfile, minilab_knob};
use crate::state::{AppState, LOOP_LENGTHS, Track};
use crate::timing::TimingStats;
use crate::transport::{Outputs, Transport};

pub const BASE_MENU_ITEMS: [&str; 5] = ["BPM", "GATE", "DIVISION", "TRANSPOSE", "LOOP LENGTH"];
const MAIN_ENCODER_TURNS: [u8; 2] = [28, 114];
const MAIN_ENCODER_PRESSES: [u8; 2] = [115, 118];
const MAIN_ENCODER_SHIFT_PRESSES: [u8; 2] = [113, 119];

pub fn default_sequence_path() -> PathBuf {
    default_config_dir().join("sequence.json")
}

/// MiniLab's 64-centred and the common 1/127 relative encodings.
pub fn relative_delta(value: u8) -> i64 {
    match value {
        64 => 0,
        1..=3 => i64::from(value),
        125..=127 => i64::from(value) - 128,
        _ => i64::from(value) - 64,
    }
}

/// Things observers (MiniLab display, GUI) may want to show.
#[derive(Debug, Clone, PartialEq)]
pub enum EngineEvent {
    /// A profile knob sent a CC.
    Knob { page: String, label: String, value: u8 },
    /// A learned control produced a command (label, formatted value).
    Command { label: String, value: Option<String> },
    /// The encoder menu changed; `None` when closed.
    Menu(Option<(String, String)>),
    /// Any input message, after routing (for value displays).
    Input(MidiMessage),
}

/// Profile knobs routed to CC output, with remembered positions for the UI.
pub struct ProfileRouter {
    pub profile: ControlProfile,
    pub output: SharedOutput,
    pub page_index: usize,
    /// (page index, knob) -> last value sent.
    pub values: HashMap<(usize, u8), u8>,
}

impl ProfileRouter {
    pub fn new(profile: ControlProfile, output: SharedOutput) -> Self {
        let values = profile
            .pages
            .iter()
            .enumerate()
            .flat_map(|(p, page)| page.knobs.iter().map(move |(&k, b)| ((p, k), b.default)))
            .collect();
        Self { profile, output, page_index: 0, values }
    }

    /// Send one knob value on a page; returns (page name, label) when bound.
    pub fn set(&mut self, page: usize, knob: u8, value: u8) -> Option<(String, String)> {
        let p = self.profile.pages.get(page)?;
        let binding = p.knobs.get(&knob)?;
        let value = value.min(127);
        self.output.lock().control_change(binding.cc, value, binding.channel - 1);
        self.values.insert((page, knob), value);
        Some((p.name.clone(), binding.label.clone()))
    }

    pub fn value(&self, page: usize, knob: u8) -> u8 {
        self.values.get(&(page, knob)).copied().unwrap_or(0)
    }
}

#[derive(Default)]
pub struct Control {
    pub mapping: Option<Mapping>,
    pub mapping_path: Option<PathBuf>,
    pub learn_target: Option<CommandName>,
    pub menu_enabled: bool,
    pub menu_open: bool,
    pub menu_editing: bool,
    pub menu_index: usize,
    pub profile: Option<ProfileRouter>,
}

impl Control {
    pub fn menu_items(&self) -> Vec<&'static str> {
        let mut items = BASE_MENU_ITEMS.to_vec();
        if self.profile.is_some() {
            items.push("CONTROL PAGE");
        }
        items
    }
}

pub struct EngineConfig {
    pub patterns: PatternStore,
    pub sequence_path: PathBuf,
    pub timing_log: Option<PathBuf>,
    pub mapping: Option<Mapping>,
    pub mapping_path: Option<PathBuf>,
    /// Enable the MiniLab main-encoder menu.
    pub encoder_menu: bool,
    pub profile: Option<ProfileRouter>,
    /// Final stretch before a deadline that is spun instead of slept (seconds).
    pub spin_margin: f64,
    /// Request macOS real-time scheduling for the transport thread.
    pub realtime: bool,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            patterns: PatternStore::default(),
            sequence_path: default_sequence_path(),
            timing_log: None,
            mapping: None,
            mapping_path: None,
            encoder_menu: false,
            profile: None,
            spin_margin: 0.000_8,
            realtime: true,
        }
    }
}

struct Shared {
    control: Mutex<Control>,
    transport: Mutex<Transport>,
    state: Mutex<AppState>,
    outs: RwLock<Outputs>,
    patterns: PatternStore,
    sequence_path: PathBuf,
    timing_log: Option<PathBuf>,
    version: AtomicU64,
    running: AtomicBool,
    listeners: Mutex<Vec<Box<dyn Fn() + Send + Sync>>>,
    events: Mutex<Vec<Sender<EngineEvent>>>,
}

impl Shared {
    fn notify(&self) {
        self.version.fetch_add(1, Ordering::Release);
        for listener in self.listeners.lock().iter() {
            listener();
        }
    }

    fn emit(&self, event: EngineEvent) {
        self.events.lock().retain(|tx| tx.send(event.clone()).is_ok());
    }
}

pub struct Engine {
    shared: Arc<Shared>,
    wake: Sender<()>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl Engine {
    pub fn new(state: AppState, outputs: Outputs, config: EngineConfig) -> Arc<Self> {
        let stats = config.timing_log.as_ref().map(|_| TimingStats::default());
        let control = Control {
            mapping: config.mapping,
            mapping_path: config.mapping_path,
            menu_enabled: config.encoder_menu,
            profile: config.profile,
            ..Control::default()
        };
        let shared = Arc::new(Shared {
            control: Mutex::new(control),
            transport: Mutex::new(Transport::new(stats)),
            state: Mutex::new(state),
            outs: RwLock::new(outputs),
            patterns: config.patterns,
            sequence_path: config.sequence_path,
            timing_log: config.timing_log,
            version: AtomicU64::new(0),
            running: AtomicBool::new(true),
            listeners: Mutex::new(Vec::new()),
            events: Mutex::new(Vec::new()),
        });
        let (wake, rx) = unbounded();
        let thread_shared = Arc::clone(&shared);
        let (margin, realtime) = (config.spin_margin, config.realtime);
        let handle = std::thread::Builder::new()
            .name("ministep-transport".into())
            .spawn(move || transport_thread(&thread_shared, &rx, margin, realtime))
            .expect("spawn transport thread");
        Arc::new(Self { shared, wake, thread: Mutex::new(Some(handle)) })
    }

    // --- observation ----------------------------------------------------------

    /// Monotonic change counter: bumps on every mutation and every played step.
    pub fn version(&self) -> u64 {
        self.shared.version.load(Ordering::Acquire)
    }

    /// Called (from any thread) after every change. Keep it cheap.
    pub fn subscribe(&self, listener: impl Fn() + Send + Sync + 'static) {
        self.shared.listeners.lock().push(Box::new(listener));
    }

    pub fn events(&self) -> Receiver<EngineEvent> {
        let (tx, rx) = unbounded();
        self.shared.events.lock().push(tx);
        rx
    }

    /// Read-only access. Do not hold across frames: the transport thread needs it.
    pub fn state(&self) -> MutexGuard<'_, AppState> {
        self.shared.state.lock()
    }

    pub fn control(&self) -> MutexGuard<'_, Control> {
        self.shared.control.lock()
    }

    pub fn patterns(&self) -> &PatternStore {
        &self.shared.patterns
    }

    pub fn sequence_path(&self) -> &PathBuf {
        &self.shared.sequence_path
    }

    pub fn outputs(&self) -> Outputs {
        self.shared.outs.read().clone()
    }

    /// Mutate state and notify observers.
    pub fn edit<R>(&self, f: impl FnOnce(&mut AppState) -> R) -> R {
        let r = f(&mut self.shared.state.lock());
        self.changed();
        r
    }

    fn changed(&self) {
        self.shared.notify();
        let _ = self.wake.send(());
    }

    fn set_status(&self, msg: impl Into<String>) {
        self.shared.state.lock().status_message = msg.into();
    }

    // --- ports ------------------------------------------------------------------

    /// Swap output ports (stops playback first so nothing hangs on the old port).
    pub fn set_outputs(&self, outputs: Outputs, melodic_name: Option<String>, drum_name: Option<String>) {
        self.stop();
        self.shared.outs.read().all_notes_off();
        *self.shared.outs.write() = outputs;
        self.edit(|s| {
            s.selected_output = melodic_name;
            s.selected_drum_output = drum_name;
        });
    }

    pub fn set_profile(&self, profile: Option<ProfileRouter>) {
        self.shared.control.lock().profile = profile;
        self.changed();
    }

    // --- transport ----------------------------------------------------------------

    pub fn play(&self) {
        {
            let mut transport = self.shared.transport.lock();
            let mut state = self.shared.state.lock();
            if self.shared.outs.read().melodic.is_none() {
                state.status_message = "Choose a MIDI output before playing.".into();
            } else if !Transport::has_content(&state) {
                state.status_message = "Sequence is empty.".into();
            } else {
                transport.start(&mut state, true);
                state.status_message = "Playing from step 1.".into();
            }
        }
        self.changed();
    }

    pub fn stop(&self) {
        let summary = {
            let mut transport = self.shared.transport.lock();
            let mut state = self.shared.state.lock();
            let outs = self.shared.outs.read().clone();
            transport.stop(&mut state, &outs);
            outs.all_notes_off();
            state.held_notes.clear();
            state.status_message = "Stopped; sent All Notes Off.".into();
            let label = format!("{} BPM 1/{}", fmt_g(state.bpm), state.step_division);
            transport.stats.as_mut().and_then(|s| {
                let summary = s.summary();
                s.reset();
                summary.map(|x| x.format(&label))
            })
        };
        if let (Some(text), Some(path)) = (summary, &self.shared.timing_log) {
            let written = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .and_then(|mut f| writeln!(f, "{text}"));
            if let Err(e) = written {
                self.set_status(format!("Timing log failed: {e}"));
            }
        }
        self.changed();
    }

    pub fn toggle_play(&self) {
        if self.state().playing { self.stop() } else { self.play() }
    }

    pub fn restart(&self) {
        let has_content = Transport::has_content(&self.state());
        if !self.state().playing || !has_content {
            self.play();
            return;
        }
        {
            let mut transport = self.shared.transport.lock();
            let mut state = self.shared.state.lock();
            let outs = self.shared.outs.read().clone();
            transport.stop(&mut state, &outs);
            transport.start(&mut state, true);
            state.status_message = "Restarted from step 1.".into();
        }
        self.changed();
    }

    // --- commands -------------------------------------------------------------------

    pub fn execute(&self, command: Command) {
        use CommandName as C;
        let value = command.value;
        match command.name {
            C::COMMIT => {
                self.commit();
            }
            C::HOLD => self.edit(|s| {
                s.append_hold();
            }),
            C::RECORD => self.set_recording(value.is_none_or(|v| v != 0.0)),
            C::RECORD_TOGGLE => {
                let on = !self.state().recording;
                self.set_recording(on);
            }
            C::REST => self.edit(|s| {
                if s.active_track == Track::Drums {
                    s.drums.skip_step();
                    s.status_message = format!("Drum step {}.", s.drums.cursor_step + 1);
                } else {
                    s.append_rest();
                }
            }),
            C::UNDO => self.edit(|s| {
                s.undo();
            }),
            C::CLEAR => self.edit(AppState::clear),
            C::PLAY => self.play(),
            C::STOP => self.stop(),
            C::PLAY_STOP => self.toggle_play(),
            C::RESTART => self.restart(),
            C::BPM_UP => self.edit(|s| s.set_bpm(s.bpm + value.unwrap_or(1.0))),
            C::BPM_DOWN => self.edit(|s| s.set_bpm(s.bpm - value.unwrap_or(1.0))),
            C::STEP_DIVISION_UP => self.edit(|s| s.cycle_step_division(1)),
            C::STEP_DIVISION_DOWN => self.edit(|s| s.cycle_step_division(-1)),
            C::SET_BPM => {
                if let Some(v) = value {
                    self.edit(|s| s.set_bpm(v));
                }
            }
            C::SET_GATE => {
                if let Some(v) = value {
                    self.edit(|s| s.set_default_gate(v, true));
                }
            }
            C::OCTAVE_UP => self.edit(|s| s.octave = (s.octave + 1).min(8)),
            C::OCTAVE_DOWN => self.edit(|s| s.octave = (s.octave - 1).max(-8)),
            C::SET_OCTAVE => {
                if let Some(v) = value {
                    self.edit(|s| s.octave = v as i32);
                }
            }
            C::TRANSPOSE_UP => {
                let by = value.map_or(1, |v| v as i32);
                self.edit(|s| s.transpose = (s.transpose + by).min(127));
            }
            C::TRANSPOSE_DOWN => {
                let by = value.map_or(1, |v| v as i32);
                self.edit(|s| s.transpose = (s.transpose - by).max(-127));
            }
            C::ARM_ROOT_CAPTURE => self.edit(|s| {
                s.root_capture_armed = true;
                s.status_message = "Root selection armed: play the desired root MIDI note.".into();
            }),
            C::SET_TRANSPOSE => {
                if let Some(v) = value {
                    self.edit(|s| s.transpose = v as i32);
                }
            }
        }
    }

    /// COMMIT for the active track: the melodic last note, or the auditioned drum hits.
    pub fn commit(&self) -> bool {
        self.edit(|s| {
            if s.active_track != Track::Drums {
                return s.commit_last_note();
            }
            let step = s.drums.cursor_step + 1;
            let names = s.drums.commit_audition();
            if names.is_empty() {
                s.status_message = "Nothing to commit: play a drum note first.".into();
                return false;
            }
            s.status_message = format!("Committed {} on step {step}.", names.join(" + "));
            true
        })
    }

    /// Non-temporal step recording; never changes transport state.
    pub fn set_recording(&self, enabled: bool) {
        self.edit(|s| {
            s.recording = enabled;
            s.status_message = if enabled {
                "RECORD ON: each physical Note On adds a step.".into()
            } else {
                "RECORD OFF: audition notes, then COMMIT the ones you want.".into()
            };
        });
    }

    pub fn arm_midi_learn(&self, command: CommandName) -> bool {
        let mut control = self.shared.control.lock();
        if control.mapping.is_none() {
            drop(control);
            self.edit(|s| s.status_message = "MIDI Learn needs --mapping minilab3.".into());
            return false;
        }
        control.learn_target = Some(command);
        drop(control);
        self.edit(|s| s.status_message = format!("MIDI Learn {}: send a CC or pad now.", command.as_str()));
        true
    }

    // --- persistence ----------------------------------------------------------------

    pub fn save(&self) -> Result<(), PatternError> {
        let path = self.shared.sequence_path.clone();
        self.save_to(&path)
    }

    pub fn save_to(&self, path: &std::path::Path) -> Result<(), PatternError> {
        let r = self.shared.state.lock().save(path);
        if let Err(e) = &r {
            self.set_status(format!("Failed to save: {e}"));
        }
        self.changed();
        r
    }

    pub fn load(&self) -> Result<(), PatternError> {
        let path = self.shared.sequence_path.clone();
        self.load_from(&path)
    }

    pub fn load_from(&self, path: &std::path::Path) -> Result<(), PatternError> {
        if self.state().playing {
            self.stop();
        }
        let r = self.shared.state.lock().load(path);
        if let Err(e) = &r {
            self.set_status(format!("Failed to load: {e}"));
        }
        self.changed();
        r
    }

    /// Which pattern store the active track addresses.
    pub fn pattern_kind(&self) -> PatternKind {
        match self.state().active_track {
            Track::Melodic => PatternKind::Melodic,
            Track::Drums => PatternKind::Drums,
        }
    }

    pub fn save_pattern(&self, name: &str, kind: Option<PatternKind>) -> bool {
        let kind = kind.unwrap_or_else(|| self.pattern_kind());
        let result = self.shared.patterns.save(kind, &self.state(), name);
        self.edit(|s| match result {
            Ok(stored) => {
                s.status_message = format!("Saved {}: {stored}", kind.label());
                true
            }
            Err(e) => {
                s.status_message = format!("Failed to save: {e}");
                false
            }
        })
    }

    /// Replace one track with the named pattern; the other track and tempo stay.
    pub fn load_pattern(&self, name: &str, kind: Option<PatternKind>) -> bool {
        let kind = kind.unwrap_or_else(|| self.pattern_kind());
        let section = match self.shared.patterns.read_section(kind, name) {
            Ok(s) => s,
            Err(e) => {
                self.edit(|s| s.status_message = format!("Failed to load: {e}"));
                return false;
            }
        };
        if self.state().playing {
            self.stop();
        }
        let stem = self.shared.patterns.path_for(kind, name).ok().and_then(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()));
        self.edit(|s| {
            let applied = match kind {
                PatternKind::Melodic => s.apply_melodic(&section),
                PatternKind::Drums => s.apply_drums(&section),
            };
            match applied {
                Ok(()) => {
                    s.status_message = format!("Loaded {}: {}", kind.label(), stem.unwrap_or_default());
                    true
                }
                Err(e) => {
                    s.status_message = format!("Failed to load: {name}: {e}");
                    false
                }
            }
        })
    }

    // --- profile knobs --------------------------------------------------------------

    /// Turn a synth-panel knob (GUI or hardware).
    pub fn set_knob(&self, page: usize, knob: u8, value: u8) {
        let routed = self.shared.control.lock().profile.as_mut().and_then(|r| r.set(page, knob, value));
        if let Some((page, label)) = routed {
            self.shared.emit(EngineEvent::Knob { page, label, value });
            self.changed();
        }
    }

    // --- encoder menu ---------------------------------------------------------------

    pub fn menu_screen(&self) -> Option<(String, String)> {
        let control = self.shared.control.lock();
        let state = self.shared.state.lock();
        menu_screen(&control, &state)
    }

    /// MiniLab main-encoder menu; returns whether the message was consumed.
    pub fn handle_main_encoder(&self, message: &MidiMessage) -> bool {
        let MidiMessage::ControlChange { control: cc, value, .. } = *message else { return false };
        let mut control = self.shared.control.lock();
        let mut state = self.shared.state.lock();
        let consumed = if MAIN_ENCODER_SHIFT_PRESSES.contains(&cc) && value > 0 {
            control.menu_open = false;
            control.menu_editing = false;
            state.status_message = "Menu closed.".into();
            true
        } else if MAIN_ENCODER_PRESSES.contains(&cc) && value > 0 {
            if !control.menu_open {
                control.menu_open = true;
            } else {
                control.menu_editing = !control.menu_editing;
            }
            set_menu_status(&control, &mut state);
            true
        } else if MAIN_ENCODER_TURNS.contains(&cc) {
            let delta = relative_delta(value);
            if delta == 0 || !control.menu_open {
                control.menu_open
            } else {
                if control.menu_editing {
                    adjust_menu_value(&mut control, &mut state, delta);
                } else {
                    let n = control.menu_items().len() as i64;
                    control.menu_index = (control.menu_index as i64 + delta).rem_euclid(n) as usize;
                }
                set_menu_status(&control, &mut state);
                true
            }
        } else {
            false
        };
        let screen = consumed.then(|| menu_screen(&control, &state));
        drop(state);
        drop(control);
        if let Some(screen) = screen {
            self.shared.emit(EngineEvent::Menu(screen));
            self.changed();
        }
        consumed
    }

    // --- MIDI input -------------------------------------------------------------------

    /// Route one input message: encoder menu, profile knobs, learn, mapping, drums, notes.
    pub fn handle_midi(&self, message: MidiMessage) {
        let menu_enabled = self.shared.control.lock().menu_enabled;
        if menu_enabled && self.handle_main_encoder(&message) {
            return;
        }
        if let MidiMessage::ControlChange { control: cc, value, .. } = message {
            if let Some(knob) = minilab_knob(cc) {
                let mut control = self.shared.control.lock();
                if let Some(router) = control.profile.as_mut() {
                    let page = router.page_index;
                    let routed = router.set(page, knob, value);
                    drop(control);
                    if let Some((page, label)) = routed {
                        self.shared.emit(EngineEvent::Knob { page, label, value });
                        self.changed();
                    }
                    return;
                }
            }
        }
        if self.handle_learn_and_mapping(&message) {
            return;
        }
        self.shared.emit(EngineEvent::Input(message.clone()));
        self.handle_note_input(message);
        self.changed();
    }

    fn handle_learn_and_mapping(&self, message: &MidiMessage) -> bool {
        let mut control = self.shared.control.lock();
        if let Some(target) = control.learn_target {
            let source = match *message {
                MidiMessage::NoteOn { velocity: 0, .. } => return true,
                MidiMessage::NoteOn { channel, note, .. } => {
                    if absolute_range(target).is_some() {
                        drop(control);
                        self.edit(|s| s.status_message = format!("{} needs a continuous CC control.", target.as_str()));
                        return true;
                    }
                    if let Some(m) = control.mapping.as_mut() {
                        let _ = m.learn_note(channel, note, target);
                    }
                    format!("note {note} on channel {}", channel + 1)
                }
                MidiMessage::ControlChange { control: cc, .. } => {
                    if let Some(m) = control.mapping.as_mut() {
                        m.learn_cc(cc, target);
                    }
                    format!("CC {cc}")
                }
                _ => return false,
            };
            control.learn_target = None;
            let saved = match (&control.mapping, &control.mapping_path) {
                (Some(m), Some(p)) => m.save(p).err(),
                _ => None,
            };
            drop(control);
            self.edit(|s| {
                s.status_message = match saved {
                    None => format!("Learned {source} → {}; mapping saved.", target.as_str()),
                    Some(e) => format!("Learned {source} → {}; saving failed: {e}", target.as_str()),
                }
            });
            return true;
        }
        let command = control.mapping.as_ref().and_then(|m| m.command_for(message));
        drop(control);
        let Some(command) = command else { return false };
        let (label, value) = display_command(&command);
        self.shared.emit(EngineEvent::Command { label, value });
        self.execute(command);
        true
    }

    fn handle_note_input(&self, message: MidiMessage) {
        let outs = self.shared.outs.read().clone();
        let mut state = self.shared.state.lock();
        if state.active_track == Track::Drums {
            if let MidiMessage::NoteOn { note, .. } | MidiMessage::NoteOff { note, .. } = message {
                if state.drums.lane_for_note(note).is_some() {
                    let channel = state.drums.channel - 1;
                    let sink = outs.drum_sink();
                    if let MidiMessage::NoteOn { velocity, .. } = message.clone().filter_press() {
                        let lane = state.drums.audition_note(note, velocity);
                        if let Some(sink) = sink {
                            sink.lock().note_on(note, velocity, channel, Owner::Input);
                        }
                        if let Some(lane) = lane {
                            state.status_message = format!(
                                "Auditioning {} (velocity {velocity}); Enter commits.",
                                state.drums.lanes[lane].name
                            );
                        }
                    } else if let Some(sink) = sink {
                        sink.lock().note_off(note, channel, Owner::Input);
                    }
                    return;
                }
            }
        }
        let channel = state.output_channel - 1;
        let out = outs.melodic.as_ref();
        match message {
            MidiMessage::NoteOn { note, velocity, .. } if velocity > 0 => {
                state.note_held(note, velocity);
                if let Some(o) = out {
                    o.lock().note_on(note, velocity, channel, Owner::Input);
                }
                if state.root_capture_armed {
                    state.root_capture_armed = false;
                    state.set_root_from_note(note);
                } else if state.recording {
                    // Only physical input reaches here, so playback cannot record itself.
                    state.commit_last_note();
                }
            }
            MidiMessage::NoteOn { note, .. } | MidiMessage::NoteOff { note, .. } => {
                state.note_released(note);
                if let Some(o) = out {
                    o.lock().note_off(note, channel, Owner::Input);
                }
            }
            other => {
                if let Some(o) = out {
                    o.lock().send_raw(&other.to_bytes(), Some(channel));
                }
            }
        }
    }

    /// Stop playback, silence everything, and join the transport thread.
    pub fn shutdown(&self) {
        if self.shared.running.swap(false, Ordering::AcqRel) {
            self.stop();
            let _ = self.wake.send(());
            if let Some(handle) = self.thread.lock().take() {
                let _ = handle.join();
            }
        }
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.shutdown();
    }
}

trait FilterPress {
    fn filter_press(self) -> MidiMessage;
}

impl FilterPress for MidiMessage {
    /// Note On with velocity 0 is a Note Off.
    fn filter_press(self) -> MidiMessage {
        match self {
            MidiMessage::NoteOn { channel, note, velocity: 0 } => MidiMessage::NoteOff { channel, note, velocity: 0 },
            other => other,
        }
    }
}

/// `%g`-style number formatting for BPM labels.
pub fn fmt_g(v: f64) -> String {
    if v.fract() == 0.0 { format!("{}", v as i64) } else { format!("{v}") }
}

pub fn display_command(command: &Command) -> (String, Option<String>) {
    use CommandName as C;
    let label = match command.name {
        C::SET_BPM => "BPM".to_string(),
        C::SET_GATE => "GATE".to_string(),
        C::SET_TRANSPOSE => "TRANSPOSE".to_string(),
        C::SET_OCTAVE => "OCTAVE".to_string(),
        other => other.as_str().replace('_', " "),
    };
    let value = command.value.map(|v| if command.name == C::SET_GATE { format!("{v:.2}") } else { fmt_g(v) });
    (label, value)
}

fn menu_value(control: &Control, state: &AppState) -> String {
    let items = control.menu_items();
    match items.get(control.menu_index).copied().unwrap_or("BPM") {
        "BPM" => format!("{} BPM", fmt_g(state.bpm)),
        "GATE" => format!("{:.2}", state.default_gate),
        "DIVISION" => format!("1/{}", state.step_division),
        "TRANSPOSE" => format!("{:+} st", state.transpose),
        "LOOP LENGTH" => state.loop_length.map_or("FULL".into(), |l| format!("{l} STEPS")),
        _ => control
            .profile
            .as_ref()
            .and_then(|r| r.profile.pages.get(r.page_index))
            .map(|p| p.name.clone())
            .unwrap_or_default(),
    }
}

fn menu_screen(control: &Control, state: &AppState) -> Option<(String, String)> {
    if !control.menu_open {
        return None;
    }
    let mode = if control.menu_editing { "EDIT" } else { "MENU" };
    let item = control.menu_items()[control.menu_index];
    Some((format!("{mode} > {item}"), menu_value(control, state)))
}

fn set_menu_status(control: &Control, state: &mut AppState) {
    let mode = if control.menu_editing { "editing" } else { "selected" };
    let item = control.menu_items()[control.menu_index];
    state.status_message = format!("Menu {mode}: {item} = {}", menu_value(control, state));
}

fn adjust_menu_value(control: &mut Control, state: &mut AppState, delta: i64) {
    let item = control.menu_items()[control.menu_index];
    match item {
        "BPM" => state.set_bpm(state.bpm + delta as f64),
        "GATE" => state.set_default_gate(state.default_gate + 0.05 * delta as f64, true),
        "DIVISION" => state.cycle_step_division(delta),
        "TRANSPOSE" => state.transpose = (state.transpose + delta as i32).clamp(-127, 127),
        "LOOP LENGTH" => {
            let i = LOOP_LENGTHS.iter().position(|&l| l == state.loop_length).unwrap_or(0) as i64;
            state.loop_length = LOOP_LENGTHS[(i + delta).rem_euclid(LOOP_LENGTHS.len() as i64) as usize];
        }
        _ => {
            if let Some(r) = control.profile.as_mut() {
                let n = r.profile.pages.len() as i64;
                r.page_index = (r.page_index as i64 + delta).rem_euclid(n) as usize;
            }
        }
    }
}

// --- the real-time thread ------------------------------------------------------------

fn instant_at(t: f64) -> Instant {
    static ORIGIN: std::sync::OnceLock<(Instant, f64)> = std::sync::OnceLock::new();
    let (origin, base) = *ORIGIN.get_or_init(|| (Instant::now(), monotonic()));
    origin + Duration::from_secs_f64((t - base).max(0.0))
}

fn transport_thread(shared: &Shared, wake: &Receiver<()>, spin_margin: f64, realtime: bool) {
    if realtime {
        crate::rt::promote_current_thread();
    }
    while shared.running.load(Ordering::Acquire) {
        let (deadline, stepped) = {
            let mut transport = shared.transport.lock();
            let mut state = shared.state.lock();
            let outs = shared.outs.read().clone();
            let before = transport.last_step_at;
            let deadline = transport.poll(monotonic(), &mut state, &outs, &monotonic);
            (deadline, transport.last_step_at != before)
        };
        if stepped {
            shared.notify();
        }
        // Sleep until shortly before the deadline (or a wake-up), then spin.
        let coarse = instant_at(deadline - spin_margin);
        match wake.recv_deadline(coarse) {
            Ok(()) => {
                while wake.try_recv().is_ok() {}
                continue;
            }
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => return,
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
        }
        while monotonic() < deadline {
            if !wake.is_empty() {
                break;
            }
            std::hint::spin_loop();
        }
    }
}
