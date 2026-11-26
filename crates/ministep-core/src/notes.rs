//! MIDI note naming in Ableton Live's convention: MIDI 60 is `C3`.

pub const NOTE_NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];

/// Ableton Live-style note name (MIDI 60 is C3).
pub fn note_name(note: u8) -> String {
    let note = note.min(127);
    format!("{}{}", NOTE_NAMES[(note % 12) as usize], i32::from(note / 12) - 2)
}

/// Inverse of [`note_name`]: `"C3"` is 60, `"F#1"` is 42, `"Bb1"` is 46.
pub fn parse_note_name(name: &str) -> Result<u8, String> {
    let s = name.trim();
    let mut chars = s.chars();
    let letter = chars.next().ok_or_else(|| format!("Unknown note name {name:?}"))?;
    let base = NOTE_NAMES
        .iter()
        .position(|n| n.len() == 1 && n.eq_ignore_ascii_case(&letter.to_string()))
        .ok_or_else(|| format!("Unknown note name {name:?}"))? as i32;
    let rest: &str = chars.as_str();
    let (accidental, octave) = match rest.chars().next() {
        Some('#') => (1, &rest[1..]),
        Some('b') => (-1, &rest[1..]),
        _ => (0, rest),
    };
    let valid_octave = !octave.is_empty()
        && octave.trim_start_matches('-').chars().all(|c| c.is_ascii_digit())
        && !octave.trim_start_matches('-').is_empty()
        && octave.matches('-').count() <= 1
        && (!octave.contains('-') || octave.starts_with('-'));
    if !valid_octave {
        return Err(format!("Unknown note name {name:?}"));
    }
    let octave: i32 = octave.parse().map_err(|_| format!("Unknown note name {name:?}"))?;
    let note = base + accidental + (octave + 2) * 12;
    if !(0..=127).contains(&note) {
        return Err(format!("Note {name:?} is outside the MIDI range"));
    }
    Ok(note as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip() {
        assert_eq!(note_name(60), "C3");
        assert_eq!(note_name(36), "C1");
        assert_eq!(note_name(0), "C-2");
        assert_eq!(parse_note_name("C3").unwrap(), 60);
        assert_eq!(parse_note_name("F#1").unwrap(), 42);
        assert_eq!(parse_note_name("Bb1").unwrap(), 46);
        assert_eq!(parse_note_name(" c-2 ").unwrap(), 0);
        for n in 0..=127u8 {
            assert_eq!(parse_note_name(&note_name(n)).unwrap(), n);
        }
        assert!(parse_note_name("H3").is_err());
        assert!(parse_note_name("C").is_err());
        assert!(parse_note_name("G9").is_err());
    }
}
