use std::sync::Mutex;

use cosmic_text::CacheKeyFlags;

/// One variable-font axis: a four-byte tag such as `wdth` and its value.
pub type Axis = ([u8; 4], f32);

pub const AXES_SHIFT: u32 = 8;
const AXES_MASK: u32 = 0xff << AXES_SHIFT;

static PROFILES: Mutex<Vec<Vec<Axis>>> = Mutex::new(Vec::new());

pub fn register(axes: &[Axis]) -> u8 {
    if axes.is_empty() {
        return 0;
    }
    let mut profiles = PROFILES.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(index) = profiles.iter().position(|p| p == axes) {
        return (index + 1) as u8;
    }
    if profiles.len() >= u8::MAX as usize {
        return 0;
    }
    profiles.push(axes.to_vec());
    profiles.len() as u8
}

/// Cache-key flags that select the registered axis profile `profile` when shaping text.
pub fn flags_for(profile: u8) -> CacheKeyFlags {
    CacheKeyFlags::from_bits_retain((profile as u32) << AXES_SHIFT)
}

pub fn profile_of(flags: CacheKeyFlags) -> u8 {
    ((flags.bits() & AXES_MASK) >> AXES_SHIFT) as u8
}

/// The axis list registered for the profile encoded in `flags`, or `None` when the flags carry no profile.
pub fn lookup(flags: CacheKeyFlags) -> Option<Vec<Axis>> {
    let profile = profile_of(flags) as usize;
    if profile == 0 {
        return None;
    }
    PROFILES.lock().unwrap_or_else(|e| e.into_inner()).get(profile - 1).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profiles_dedupe_and_round_trip_through_flags() {
        let axes = [(*b"ROND", 100.0), (*b"wdth", 100.0)];
        let id = register(&axes);
        assert_ne!(id, 0);
        assert_eq!(register(&axes), id);
        assert_eq!(lookup(flags_for(id)).as_deref(), Some(&axes[..]));
        assert_eq!(lookup(CacheKeyFlags::empty()), None);
        assert_eq!(register(&[]), 0);
    }
}
