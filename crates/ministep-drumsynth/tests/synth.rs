use ministep_core::voice::{VoiceModel, VoiceParams, decay_seconds};
use ministep_drumsynth::{Hit, Mixer, render};

const SR: f32 = 48_000.0;

fn peak(x: &[f32]) -> f32 {
    x.iter().fold(0.0, |m, v| m.max(v.abs()))
}

#[test]
fn every_model_is_audible_bounded_and_ends() {
    for model in VoiceModel::ALL {
        let p = VoiceParams::new(model);
        let out = render(p, 110, SR, 5.0);
        assert!(out.iter().all(|v| v.is_finite()), "{model:?} produced NaN/inf");
        let pk = peak(&out);
        assert!(pk > 0.05, "{model:?} too quiet: {pk}");
        assert!(pk < 2.0, "{model:?} too loud: {pk}");
        let limit = decay_seconds(model, p.decay) * 1.6 + 0.05;
        assert!((out.len() as f32) < limit * SR, "{model:?} rings {}s > {limit}s", out.len() as f32 / SR);
    }
}

#[test]
fn decay_and_level_knobs_do_what_they_say() {
    for model in VoiceModel::ALL {
        let mut short = VoiceParams::new(model);
        short.decay = 0.1;
        let mut long = short;
        long.decay = 0.9;
        assert!(render(long, 100, SR, 6.0).len() > render(short, 100, SR, 6.0).len(), "{model:?} decay");
        let mut quiet = VoiceParams::new(model);
        quiet.level = 0.3;
        assert!(peak(&render(quiet, 100, SR, 3.0)) < peak(&render(VoiceParams::new(model), 100, SR, 3.0)), "{model:?} level");
    }
}

#[test]
fn kick_tune_raises_pitch() {
    let crossings = |tune: f32| {
        let mut p = VoiceParams::new(VoiceModel::Kick);
        p.tune = tune;
        p.snap = 0.0;
        let x = render(p, 100, SR, 0.4);
        // Skip the pitch sweep.
        x[(0.1 * SR) as usize..].windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count()
    };
    assert!(crossings(0.8) > crossings(0.5) && crossings(0.5) > crossings(0.2));
}

/// First frame whose magnitude exceeds `thr` after `from`.
fn onset(x: &[f32], from: usize, thr: f32) -> Option<usize> {
    x[from..].iter().position(|v| v.abs() > thr).map(|i| i + from)
}

#[test]
fn hits_land_on_exact_frames_despite_callback_jitter() {
    let latency = 0.01;
    let mut m = Mixer::new(SR, latency);
    let block = 256usize;
    // The device runs for a while before Play (warm-up of the clock estimate).
    let warm = 2.0;
    let t0 = 100.0 + warm;
    let step = 0.125; // 1/16 at 120 BPM
    let mut out = Vec::new();
    let mut buf = vec![0.0f32; block];
    let mut rng = 12345u32;
    let mut next_hit = 0;
    let blocks = ((2.2 + warm) * SR as f64) as usize / block;
    for b in 0..blocks {
        // Callback "now" wobbles by up to ±1.5 ms around the true block time.
        rng = rng.wrapping_mul(1_103_515_245).wrapping_add(12345);
        let jitter = ((rng >> 8) as f64 / f64::from(1u32 << 24) - 0.5) * 0.003;
        let now = 100.0 + (b * block) as f64 / f64::from(SR) + jitter;
        // The transport sends each hit when its time comes (possibly a bit late).
        while next_hit < 16 && t0 + next_hit as f64 * step <= now + 0.001 {
            let mut p = VoiceParams::new(VoiceModel::Rim);
            p.decay = 0.0;
            m.schedule(Hit { at: t0 + next_hit as f64 * step, params: p, velocity: 127 });
            next_hit += 1;
        }
        m.process(&mut buf, 1, now, None);
        out.extend_from_slice(&buf);
    }
    let spacing = f64::from(SR) * step;
    let first = onset(&out, 0, 1e-9).unwrap();
    let mut prev = first;
    for k in 1..16 {
        let o = onset(&out, prev + (spacing * 0.5) as usize, 1e-9).unwrap();
        let err = (o - first) as f64 - k as f64 * spacing;
        // ±2 frames = ±0.04 ms at 48 kHz, under ±1.5 ms callback jitter (CoreAudio is far steadier).
        assert!(err.abs() <= 2.0, "hit {k} off by {err} frames");
        prev = o;
    }
    assert_eq!(m.late_hits, 0);
}

#[test]
fn closed_hat_chokes_open_hat() {
    let mut m = Mixer::new(SR, 0.0);
    let mut buf = vec![0.0f32; 512];
    m.process(&mut buf, 1, 0.0, None);
    m.schedule(Hit { at: 512.0 / f64::from(SR), params: VoiceParams::new(VoiceModel::OpenHat), velocity: 100 });
    m.process(&mut buf, 1, 512.0 / f64::from(SR), None);
    assert_eq!(m.active_voices(), 1);
    let t = 1024.0 / f64::from(SR);
    let mut closed = VoiceParams::new(VoiceModel::ClosedHat);
    closed.decay = 0.0;
    m.schedule(Hit { at: t, params: closed, velocity: 100 });
    for i in 0..40 {
        m.process(&mut buf, 1, t + f64::from(i * 512) / f64::from(SR), None);
    }
    assert_eq!(m.active_voices(), 0, "open hat should be choked within ~20 ms");
}
