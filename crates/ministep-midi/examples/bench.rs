//! End-to-end transport timing bench.
//!
//! Plays a 16-step sequence through the real engine (real-time thread, midir,
//! CoreMIDI) into a virtual destination owned by this process, timestamps every
//! received Note On, and compares arrivals with the ideal grid.
//!
//!   cargo run --release -p ministep-midi --example bench -- [seconds] [bpm...] [--margin-us N] [--no-rt]

use std::sync::Arc;
use std::time::Duration;

use ministep_core::clock::{monotonic, step_duration_seconds};
use ministep_core::engine::{Engine, EngineConfig};
use ministep_core::state::{AppState, Step};
use ministep_core::transport::Outputs;
use parking_lot::Mutex;

fn pct(sorted: &[f64], f: f64) -> f64 {
    if sorted.is_empty() { return 0.0; }
    sorted[((f * (sorted.len() - 1) as f64).round() as usize).min(sorted.len() - 1)]
}

fn cpu_seconds() -> f64 {
    let mut u: libc::rusage = unsafe { std::mem::zeroed() };
    unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut u) };
    let tv = |t: libc::timeval| t.tv_sec as f64 + t.tv_usec as f64 / 1e6;
    tv(u.ru_utime) + tv(u.ru_stime)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut seconds = 30.0;
    let mut bpms = vec![120.0, 180.0, 240.0];
    let mut margin = 0.0008;
    let mut realtime = true;
    let mut positional = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--margin-us" => margin = it.next().unwrap().parse::<f64>().unwrap() / 1e6,
            "--no-rt" => realtime = false,
            _ => positional.push(a.parse::<f64>().expect("number")),
        }
    }
    if let Some((&s, rest)) = positional.split_first() {
        seconds = s;
        if !rest.is_empty() { bpms = rest.to_vec(); }
    }

    let arrivals: Arc<Mutex<Vec<f64>>> = Arc::default();
    let sink = Arc::clone(&arrivals);
    let _dest = ministep_midi::open_virtual_input("MiniStep Bench", move |bytes| {
        if bytes.len() == 3 && bytes[0] & 0xf0 == 0x90 && bytes[2] > 0 {
            sink.lock().push(monotonic());
        }
    })
    .expect("virtual destination");
    std::thread::sleep(Duration::from_millis(300));
    let out = ministep_midi::open_output("MiniStep Bench").expect("open bench port");

    println!("margin={:.0}us realtime={realtime} seconds={seconds}", margin * 1e6);
    for bpm in bpms {
        let state = AppState {
            bpm,
            step_division: 16,
            sequence: (0..16).map(|i| Step::new(Some(60 + i), 100, 0.5)).collect(),
            ..AppState::default()
        };
        let log = std::env::temp_dir().join("ministep-bench-timing.log");
        let _ = std::fs::remove_file(&log);
        let engine = Engine::new(
            state,
            Outputs { melodic: Some(Arc::clone(&out)), drums: None },
            EngineConfig { timing_log: Some(log.clone()), spin_margin: margin, realtime, ..EngineConfig::default() },
        );
        arrivals.lock().clear();
        let cpu0 = cpu_seconds();
        engine.play();
        std::thread::sleep(Duration::from_secs_f64(seconds));
        engine.stop();
        let cpu = cpu_seconds() - cpu0;
        engine.shutdown();
        let a = arrivals.lock().clone();
        let d = step_duration_seconds(bpm, 16);
        let intervals: Vec<f64> = a.windows(2).map(|w| w[1] - w[0] - d).collect();
        let mut jitter: Vec<f64> = intervals.iter().map(|e| e.abs()).collect();
        jitter.sort_by(f64::total_cmp);
        let drift: Vec<f64> = a.iter().enumerate().map(|(i, t)| t - (a[0] + i as f64 * d)).collect();
        let mean_err = intervals.iter().sum::<f64>() / intervals.len().max(1) as f64;
        let ms = 1000.0;
        println!(
            "[{bpm} BPM 1/16] arrivals={} expected~{} step={:.3}ms cpu={:.1}%\n  arrival jitter ms p50={:.4} p95={:.4} p99={:.4} max={:.4}  mean_err={:+.5}\n  drift ms final={:+.4} max_abs={:.4}",
            a.len(),
            (seconds / d) as usize,
            d * ms,
            100.0 * cpu / seconds,
            pct(&jitter, 0.5) * ms,
            pct(&jitter, 0.95) * ms,
            pct(&jitter, 0.99) * ms,
            jitter.last().copied().unwrap_or(0.0) * ms,
            mean_err * ms,
            drift.last().copied().unwrap_or(0.0) * ms,
            drift.iter().map(|x| x.abs()).fold(0.0, f64::max) * ms,
        );
        if let Ok(text) = std::fs::read_to_string(&log) {
            for line in text.lines() { println!("  engine {line}"); }
        }
    }
}
