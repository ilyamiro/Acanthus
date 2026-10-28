use acanthus_core::Color;

/// Where an `ext-session-lock-v1` lock stands.
///
/// The compositor, not the client, keeps the session locked: when the locking process dies the session stays locked until a new client takes the lock and unlocks it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LockPhase {
    /// No lock is held.
    #[default]
    Unlocked,
    /// The lock was requested and the compositor has not confirmed it yet.
    Locking,
    /// The compositor confirmed the lock; lock surfaces cover every output.
    Locked,
    /// The compositor refused the lock, for instance because another client already holds it.
    Denied,
    /// The compositor ended a confirmed lock without the client asking.
    Lost,
}

/// What to do right after the compositor confirmed the lock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OnLocked {
    Keep,
    UnlockNow,
}

#[derive(Debug, Default)]
pub(crate) struct LockMachine {
    phase: LockPhase,
    unlock_when_locked: bool,
}

impl LockMachine {
    pub fn phase(&self) -> LockPhase {
        self.phase
    }

    pub fn request(&mut self) -> bool {
        match self.phase {
            LockPhase::Locking | LockPhase::Locked => false,
            LockPhase::Unlocked | LockPhase::Denied | LockPhase::Lost => {
                self.phase = LockPhase::Locking;
                self.unlock_when_locked = false;
                true
            }
        }
    }

    pub fn locked(&mut self) -> OnLocked {
        if self.phase != LockPhase::Locking {
            return OnLocked::Keep;
        }
        self.phase = LockPhase::Locked;
        if self.unlock_when_locked { OnLocked::UnlockNow } else { OnLocked::Keep }
    }

    pub fn finished(&mut self) {
        self.phase = match self.phase {
            LockPhase::Locked => LockPhase::Lost,
            LockPhase::Locking => LockPhase::Denied,
            other => other,
        };
    }

    pub fn unlock(&mut self) -> bool {
        match self.phase {
            LockPhase::Locked => {
                self.phase = LockPhase::Unlocked;
                true
            }
            LockPhase::Locking => {
                self.unlock_when_locked = true;
                false
            }
            _ => false,
        }
    }
}

/// Appearance of the lock surfaces.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LockConfig {
    /// Colour each lock surface is cleared to before drawing.
    pub clear: Color,
}

impl LockConfig {
    /// Opaque black lock surfaces.
    pub fn new() -> Self {
        LockConfig { clear: Color::BLACK }
    }

    /// Sets the clear colour.
    pub fn clear(self, clear: Color) -> Self {
        LockConfig { clear }
    }
}

impl Default for LockConfig {
    fn default() -> Self {
        Self::new()
    }
}

pub(crate) fn reconcile(existing: &[u32], wanted: &[u32]) -> (Vec<u32>, Vec<u32>) {
    let add = wanted.iter().copied().filter(|o| !existing.contains(o)).collect();
    let remove = existing.iter().copied().filter(|o| !wanted.contains(o)).collect();
    (add, remove)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_goes_through_locking_to_locked() {
        let mut m = LockMachine::default();
        assert!(m.request());
        assert_eq!(m.phase(), LockPhase::Locking);
        assert_eq!(m.locked(), OnLocked::Keep);
        assert_eq!(m.phase(), LockPhase::Locked);
    }

    #[test]
    fn second_request_while_held_is_refused() {
        let mut m = LockMachine::default();
        assert!(m.request());
        assert!(!m.request());
        m.locked();
        assert!(!m.request());
    }

    #[test]
    fn finished_before_locked_means_denied() {
        let mut m = LockMachine::default();
        m.request();
        m.finished();
        assert_eq!(m.phase(), LockPhase::Denied);
        assert!(m.request());
    }

    #[test]
    fn finished_after_locked_means_lost() {
        let mut m = LockMachine::default();
        m.request();
        m.locked();
        m.finished();
        assert_eq!(m.phase(), LockPhase::Lost);
        assert!(!m.unlock());
    }

    #[test]
    fn unlock_when_locked_releases() {
        let mut m = LockMachine::default();
        m.request();
        m.locked();
        assert!(m.unlock());
        assert_eq!(m.phase(), LockPhase::Unlocked);
    }

    #[test]
    fn unlock_while_locking_waits_for_confirmation() {
        let mut m = LockMachine::default();
        m.request();
        assert!(!m.unlock());
        assert_eq!(m.phase(), LockPhase::Locking);
        assert_eq!(m.locked(), OnLocked::UnlockNow);
    }

    #[test]
    fn unlock_without_lock_does_nothing() {
        let mut m = LockMachine::default();
        assert!(!m.unlock());
        assert_eq!(m.phase(), LockPhase::Unlocked);
    }

    #[test]
    fn reconcile_lists_added_and_removed_outputs() {
        assert_eq!(reconcile(&[1, 2], &[2, 3]), (vec![3], vec![1]));
        assert_eq!(reconcile(&[], &[1]), (vec![1], vec![]));
        assert_eq!(reconcile(&[1], &[1]), (vec![], vec![]));
    }
}
