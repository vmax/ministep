//! MIDI port selection at runtime (input callback → engine, outputs → engine).

use std::sync::Arc;

use ministep_core::engine::{Engine, ProfileRouter};
use ministep_core::output::SharedOutput;
use ministep_core::profiles::ControlProfile;
use ministep_core::transport::Outputs;
use ministep_midi::InputHandle;

pub const VIRTUAL_OUT: &str = "MiniStep Out";
pub const VIRTUAL_LABEL: &str = "MiniStep Out (virtual)";

#[derive(Default)]
pub struct Ports {
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
    pub input: Option<InputHandle>,
    pub melodic: Option<(String, SharedOutput)>,
    pub drums: Option<(String, SharedOutput)>,
    virtual_out: Option<SharedOutput>,
}

impl Ports {
    pub fn refresh(&mut self) {
        self.inputs = ministep_midi::input_ports();
        self.outputs = ministep_midi::output_ports();
        // Our own virtual source shows up as an output of other apps, not ours.
        self.outputs.retain(|p| p != VIRTUAL_OUT);
        self.outputs.insert(0, VIRTUAL_LABEL.into());
        self.inputs.retain(|p| p != VIRTUAL_OUT);
    }

    pub fn input_name(&self) -> Option<&str> {
        self.input.as_ref().map(|h| h.name.as_str())
    }

    pub fn set_input(&mut self, engine: &Arc<Engine>, name: Option<&str>) -> Result<(), String> {
        self.input = None;
        let Some(name) = name else {
            engine.edit(|s| s.selected_input = None);
            return Ok(());
        };
        let target = Arc::clone(engine);
        let handle = ministep_midi::open_input(name, move |m| target.handle_midi(m)).map_err(|e| e.to_string())?;
        self.input = Some(handle);
        engine.edit(|s| s.selected_input = Some(name.to_string()));
        Ok(())
    }

    fn open(&mut self, name: &str) -> Result<SharedOutput, String> {
        if name == VIRTUAL_LABEL || name == VIRTUAL_OUT {
            if let Some(v) = &self.virtual_out {
                return Ok(Arc::clone(v));
            }
            let v = ministep_midi::open_virtual_output(VIRTUAL_OUT).map_err(|e| e.to_string())?;
            self.virtual_out = Some(Arc::clone(&v));
            return Ok(v);
        }
        if let Some((n, o)) = self.melodic.iter().chain(self.drums.iter()).find(|(n, _)| n == name) {
            let _ = n;
            return Ok(Arc::clone(o));
        }
        ministep_midi::open_output(name).map_err(|e| e.to_string())
    }

    /// Apply output choices; `drums == None` shares the melodic port.
    pub fn set_outputs(&mut self, engine: &Engine, melodic: Option<&str>, drums: Option<&str>) -> Result<(), String> {
        let m = melodic.map(|n| self.open(n).map(|o| (n.to_string(), o))).transpose()?;
        let d = match drums {
            Some(n) if Some(n) != melodic => Some(self.open(n).map(|o| (n.to_string(), o))?),
            _ => None,
        };
        self.melodic = m;
        self.drums = d;
        engine.set_outputs(
            Outputs { melodic: self.melodic.as_ref().map(|x| Arc::clone(&x.1)), drums: self.drums.as_ref().map(|x| Arc::clone(&x.1)) },
            self.melodic.as_ref().map(|x| x.0.clone()),
            self.drums.as_ref().map(|x| x.0.clone()),
        );
        Ok(())
    }

    /// Bind a synth profile to its own output, the profile's `output`, or the melodic port.
    pub fn router_for(&mut self, profile: ControlProfile, control_output: Option<&str>) -> Result<ProfileRouter, String> {
        let requested = control_output.map(str::to_string).or_else(|| profile.output.clone());
        let output = match requested {
            Some(q) => {
                let all = ministep_midi::output_ports();
                let name = ministep_midi::find_port(&q, &all).ok_or(format!("no output matches {q:?}"))?;
                self.open(&name)?
            }
            None => self.melodic.as_ref().map(|x| Arc::clone(&x.1)).ok_or("choose a MIDI output for the synth profile")?,
        };
        Ok(ProfileRouter::new(profile, output))
    }
}
