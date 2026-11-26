//! MiniStep: hardware-style MIDI step sequencer.

mod app;
mod ports;
mod theme;
mod widgets;

use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use eframe::egui;
use ministep_core::controller::{Mapping, default_mapping_path};
use ministep_core::engine::{Engine, EngineConfig};
use ministep_core::state::AppState;
use ministep_core::transport::Outputs;

#[derive(Parser, Debug)]
#[command(name = "ministep", version, about = "MiniStep MIDI step recorder/sequencer")]
struct Args {
    /// MIDI input (exact or unambiguous partial match).
    #[arg(long)]
    input: Option<String>,
    /// MIDI output (exact or unambiguous partial match). Default: virtual "MiniStep Out".
    #[arg(long)]
    output: Option<String>,
    /// MIDI output for the drum track; default: same as --output.
    #[arg(long)]
    drum_output: Option<String>,
    /// List MIDI ports and exit.
    #[arg(long)]
    list_midi: bool,
    /// Load a JSON sequence on startup.
    #[arg(long)]
    load: Option<PathBuf>,
    /// Enable persisted MIDI Learn mappings.
    #[arg(long, value_parser = ["none", "minilab3"], default_value = "none")]
    mapping: String,
    /// YAML synth-control profile (the synth panel).
    #[arg(long)]
    profile: Option<PathBuf>,
    /// MIDI output for profile CCs; overrides the profile's `output`.
    #[arg(long)]
    control_output: Option<String>,
    /// Show MiniStep state on a connected MiniLab 3 (OLED + pads) and enable its encoder menu.
    #[arg(long)]
    minilab_display: bool,
    /// Do not open the local control socket.
    #[arg(long)]
    no_remote: bool,
    /// Unix socket for remote control (NDJSON); default ~/.config/ministep/control.sock.
    #[arg(long)]
    remote_socket: Option<PathBuf>,
    /// Where drum hits go: built-in synth, MIDI, or both. Default: synth, or MIDI with --drum-output.
    #[arg(long, value_parser = ["synth", "midi", "both"])]
    drum_engine: Option<String>,
    /// Audio output for the drum synth (default: system output).
    #[arg(long)]
    audio_device: Option<String>,
    /// List audio outputs and exit.
    #[arg(long)]
    list_audio: bool,
    /// Append a transport timing summary to this file each time playback stops.
    #[arg(long)]
    timing_log: Option<PathBuf>,
}

fn pick(kind: &str, query: Option<&str>, ports: &[String]) -> Result<Option<String>, String> {
    match query {
        None => Ok(None),
        Some(q) => ministep_midi::find_port(q, ports).map(Some).ok_or(format!("No unambiguous {kind} port matches {q:?}")),
    }
}

fn run(args: Args) -> Result<(), String> {
    if args.list_audio {
        println!("Audio outputs:");
        ministep_drumsynth::output_devices().iter().for_each(|d| println!("  {d}"));
        return Ok(());
    }
    let (inputs, outputs) = (ministep_midi::input_ports(), ministep_midi::output_ports());
    if args.list_midi {
        println!("MIDI inputs:");
        inputs.iter().for_each(|p| println!("  {p}"));
        println!("MIDI outputs:");
        outputs.iter().for_each(|p| println!("  {p}"));
        return Ok(());
    }
    let input = pick("input", args.input.as_deref(), &inputs)?;
    let output = pick("output", args.output.as_deref(), &outputs)?.unwrap_or_else(|| ports::VIRTUAL_LABEL.into());
    let drum_output = pick("drum output", args.drum_output.as_deref(), &outputs)?;

    let mut state = AppState::default();
    let route = args.drum_engine.as_deref().and_then(ministep_core::voice::DrumRoute::parse).unwrap_or(if drum_output.is_some() {
        ministep_core::voice::DrumRoute::Midi
    } else {
        ministep_core::voice::DrumRoute::Synth
    });
    if let Some(path) = &args.load {
        state.load(path).map_err(|e| format!("cannot load {}: {e}", path.display()))?;
    }
    let mapping = (args.mapping == "minilab3")
        .then(|| Mapping::load_or_default(&default_mapping_path()))
        .transpose()
        .map_err(|e| format!("cannot load MIDI mapping: {e}"))?;
    let engine = Engine::new(
        state,
        Outputs::default(),
        EngineConfig {
            timing_log: args.timing_log.clone(),
            mapping,
            mapping_path: Some(default_mapping_path()),
            encoder_menu: args.minilab_display,
            ..EngineConfig::default()
        },
    );
    let audio = match ministep_drumsynth::start(args.audio_device.as_deref()) {
        Ok((output, sink)) => {
            engine.set_drum_synth(Some(sink));
            Some(output)
        }
        Err(e) => {
            eprintln!("ministep: drum synth unavailable ({e}); drums go to MIDI");
            None
        }
    };
    let route = if audio.is_none() && route.synth() { ministep_core::voice::DrumRoute::Midi } else { route };
    engine.edit(|s| s.drum_route = route);
    let audio_label = audio.as_ref().map(|a| format!("{} · {} Hz · {:.1} ms", a.device, a.sample_rate, a.latency_ms));
    let mut ports = ports::Ports::default();
    ports.refresh();
    ports.set_outputs(&engine, Some(&output), drum_output.as_deref())?;
    ports.set_input(&engine, input.as_deref())?;
    if let Some(path) = &args.profile {
        let profile = ministep_core::profiles::load_profile(path).map_err(|e| format!("cannot load control profile: {e}"))?;
        let router = ports.router_for(profile, args.control_output.as_deref())?;
        engine.set_profile(Some(router));
    }
    let _remote = if args.no_remote {
        None
    } else {
        let path = args.remote_socket.clone().unwrap_or_else(ministep_remote::default_socket_path);
        match ministep_remote::RemoteServer::start(Arc::clone(&engine), path) {
            Ok(server) => Some(server),
            Err(e) => {
                eprintln!("ministep: cannot open control socket: {e}");
                None
            }
        }
    };
    let _display = if args.minilab_display {
        let display = ministep_minilab::MiniLab3Display::open().map_err(|e| format!("cannot open MiniLab display: {e}"))?;
        Some(ministep_minilab::run_display(Arc::clone(&engine), display))
    } else {
        None
    };
    engine.edit(|s| s.status_message = "Ready: audition a note, then commit it.".into());
    let engine_for_app = Arc::clone(&engine);
    let profile_path = args.profile.clone();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("MiniStep")
            .with_inner_size([1680.0, 960.0])
            .with_min_inner_size([1280.0, 820.0])
            .with_fullsize_content_view(true)
            .with_title_shown(false)
            .with_titlebar_shown(false),
        ..Default::default()
    };
    let result = eframe::run_native(
        "MiniStep",
        options,
        Box::new(move |cc| {
            let mut app = app::App::new(cc, engine_for_app, ports, profile_path);
            app.audio_device = audio_label;
            Ok(Box::new(app))
        }),
    );
    engine.shutdown();
    drop(audio);
    result.map_err(|e| e.to_string())
}

fn main() {
    if let Err(e) = run(Args::parse()) {
        eprintln!("ministep: {e}");
        std::process::exit(2);
    }
}
