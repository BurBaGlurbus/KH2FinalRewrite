//! The HUD's slot-height table (0x140abeaa0): popups register a height in a
//! (slot, sub-slot) cell when they are created (FUN_140182b10) and clear it
//! when they are destroyed (FUN_140182b80); an object's stacking position is
//! the sum of the cells of every lower slot in its sub-slot (FUN_140182ba0).
//! The Scan gauge is slot 18, sub-slot 1.
//!
//! Who registers in the executable (FUN_140182b10 callers): the radar takes
//! slot 18 / sub-slot 1 with height 0, so it never moves the gauge; the
//! information window (slot 1), the NEXT_EXP window (slot 14) and the
//! GET_MUNNY window (slot 17) all use sub-slot 0, which the gauge does not
//! stack on; only the script-driven counter, mission gauge and information
//! pop-ups (FUN_14018be20, FUN_14018c990, FUN_14018d1b0) take their slot and
//! sub-slot from event-script arguments, and the rewrite plays none of those
//! scripts. The offset therefore stays zero in every supported scene.

/// Slots 0 through 18 fit the executable's 59-short table.
pub const SLOTS: usize = 19;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HudSlots {
    cells: [[i16; 3]; SLOTS],
}

impl HudSlots {
    /// Registers a height; an out-of-range cell is ignored.
    pub fn register(&mut self, slot: usize, sub: usize, height: i16) {
        if let Some(cell) = self.cells.get_mut(slot).and_then(|row| row.get_mut(sub)) {
            *cell = height;
        }
    }

    /// FUN_140182b80.
    pub fn release(&mut self, slot: usize, sub: usize) {
        self.register(slot, sub, 0);
    }

    /// FUN_140182c40: every cell cleared.
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// FUN_140182ba0: the sum of the sub-slot's cells below `slot`.
    pub fn stack(&self, slot: usize, sub: usize) -> i32 {
        self.cells
            .iter()
            .take(slot)
            .map(|row| row.get(sub).copied().unwrap_or(0) as i32)
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_slot_stacks_the_heights_below_it_in_its_sub_slot() {
        let mut slots = HudSlots::default();
        slots.register(1, 1, 40);
        slots.register(3, 1, 25);
        slots.register(3, 0, 99);
        slots.register(18, 1, 7);
        assert_eq!(slots.stack(18, 1), 65);
        assert_eq!(slots.stack(3, 1), 40);
        assert_eq!(slots.stack(18, 0), 99);
        slots.release(1, 1);
        assert_eq!(slots.stack(18, 1), 25);
        slots.register(40, 0, 5);
        slots.clear();
        assert_eq!(slots.stack(18, 1), 0);
    }
}
