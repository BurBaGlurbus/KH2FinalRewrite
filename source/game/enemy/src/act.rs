//! The per-object act manager, translated from the executable (Steam build
//! 15194255): the table and queue of FUN_1403ca0e0 / FUN_1403ca790 /
//! FUN_1403ca700, start FUN_1403cae60 / FUN_1403cad00, push FUN_1403cab00,
//! clear FUN_1403ca440 / FUN_1403caec0, the queue pop FUN_1403ca2c0 and the
//! per-tick loop FUN_1403ca550. It lives at `sysobj + 0x390`.
//!
//! An *act* is a named script routine (`appear`, `mode_battle`, `atk_craw_l`
//! ...). Only one act runs at a time; starting one discards the running act and
//! the queue. The manager never touches the script or the object itself: the
//! caller turns [`Action`]s into script threads and callbacks, which keeps the
//! manager testable without an interpreter.

/// The queue is a ring of sixteen entries that keeps one slot empty.
pub const QUEUE_SLOTS: usize = 16;
pub const TABLE_SLOTS: usize = 128;

/// One registered act (32 bytes in the executable's table).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Entry {
    pub name: Vec<u8>,
    /// +4: priority, compared with the running act's.
    pub priority: u16,
    /// +6: bit 0 set = the first run is not gated on the object being
    /// grounded and controllable; bit 1 = locked while running.
    pub flag_bits: u16,
    /// +8: script word index of the act routine.
    pub entry: u16,
    /// +0xa: end callback.
    pub end: u16,
    /// +0x10: damage callback (two arguments).
    pub damage: u16,
    /// +0x14: timeout callback.
    pub timeout: u16,
    /// +0x16: start callback.
    pub start: u16,
    /// +0x18: three-argument event callback.
    pub event: u16,
    /// +0xc, +0xe, +0x12: registered but not read by any caller found so far.
    pub unused: [u16; 3],
}

/// Manager flags at +0x12c.
pub const LOCKED: u32 = 1;
/// Set by every start; cleared when the loop picks the new act up.
pub const JUST_STARTED: u32 = 2;
/// The act's first run waits for the object to be on the ground and in control.
pub const GATED: u32 = 4;

/// Work for the caller, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Run a registered callback routine (`word` of the script) with these
    /// argument kinds on a throw-away thread.
    Callback { word: u16, argument: Argument },
    /// Replace the act thread by a fresh one at `entry` (the AI object is its
    /// argument).
    Spawn { entry: u16 },
    /// Discard the running act thread.
    Kill,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Argument {
    /// The AI object only.
    Object,
}

#[derive(Clone, Debug)]
pub struct Manager {
    pub table: Vec<Entry>,
    write: usize,
    read: usize,
    queue: [Option<usize>; QUEUE_SLOTS],
    /// +0x118: the running act (table index).
    pub current: Option<usize>,
    /// +0x128: priority of the running act.
    pub priority: u16,
    pub flags: u32,
    pub has_table: bool,
    /// +0x108: the script's AI object (the first argument of its act traps).
    pub ai_object: u32,
    /// +0x130..+0x140: a shout waiting out its random delay.
    pub pending_shout: Option<PendingShout>,
}

/// A shout (`act_shout`) that reached an object with a delay: after the timer
/// runs out the object calls the timeout routine of its act of that name.
#[derive(Clone, Debug, PartialEq)]
pub struct PendingShout {
    pub name: Vec<u8>,
    /// The shouter's AI object.
    pub shouter: u32,
    /// Ticks left.
    pub timer: f32,
}

impl Default for Manager {
    fn default() -> Self {
        Self {
            table: Vec::new(),
            write: 0,
            read: 0,
            queue: [None; QUEUE_SLOTS],
            current: None,
            priority: 0,
            flags: 0,
            has_table: false,
            ai_object: 0,
            pending_shout: None,
        }
    }
}

impl Manager {
    /// FUN_1403ca790 (`trap_act_table_init`).
    pub fn init_table(&mut self) {
        self.table.clear();
        self.has_table = true;
    }

    /// FUN_1403ca700: the first table entry with this name.
    pub fn find(&self, name: &[u8]) -> Option<usize> {
        self.table.iter().position(|e| e.name == name)
    }

    /// FUN_1403ca0e0: a same-named entry is overwritten, else the first free
    /// slot (at most 128 entries) is used.
    pub fn add(&mut self, entry: Entry) {
        if let Some(i) = self.find(&entry.name) {
            self.table[i] = entry;
        } else if self.table.len() < TABLE_SLOTS {
            self.table.push(entry);
        }
    }

    pub fn locked(&self) -> bool {
        self.flags & LOCKED != 0
    }

    /// `trap_obj_is_act_exec`: false only when the queue is empty and no act
    /// runs (sysobj+0x394 == +0x390 and +0x4a8 == 0).
    pub fn is_exec(&self) -> bool {
        !(self.read == self.write && self.current.is_none())
    }

    pub fn queued(&self) -> usize {
        (self.write + QUEUE_SLOTS - self.read) % QUEUE_SLOTS
    }

    /// FUN_1403cab00 (`trap_obj_act_push`): ignored when locked or the name is
    /// unknown; the ring holds fifteen entries.
    pub fn push(&mut self, name: &[u8]) {
        if self.locked() {
            return;
        }
        let Some(index) = self.find(name) else {
            return;
        };
        if self.queued() < QUEUE_SLOTS - 1 {
            self.queue[self.write] = Some(index);
            self.write = (self.write + 1) % QUEUE_SLOTS;
        }
    }

    /// FUN_1403caec0: end the running act. Returns the end callback to run,
    /// then the thread is discarded.
    fn finish(&mut self, out: &mut Vec<Action>) {
        if let Some(i) = self.current.take() {
            let end = self.table[i].end;
            if end != 0 {
                out.push(Action::Callback {
                    word: end,
                    argument: Argument::Object,
                });
            }
            out.push(Action::Kill);
        }
    }

    /// FUN_1403ca440 (`trap_obj_act_clear`): kill the running act, drop the
    /// queue and the priority.
    pub fn clear(&mut self) -> Vec<Action> {
        let mut out = Vec::new();
        if !self.locked() {
            self.finish(&mut out);
            self.write = 0;
            self.read = 0;
            self.priority = 0;
        }
        out
    }

    /// FUN_1403cad00: begin an act. The previous one is finished first.
    fn begin(&mut self, index: usize, out: &mut Vec<Action>) {
        self.finish(out);
        let e = &self.table[index];
        let (entry, start, bits, priority) = (e.entry, e.start, e.flag_bits, e.priority);
        out.push(Action::Spawn { entry });
        self.current = Some(index);
        self.priority = priority;
        let mut flags = 0;
        if bits & 2 != 0 {
            flags |= LOCKED;
        }
        if bits & 1 == 0 {
            flags |= GATED;
        }
        self.flags = flags | JUST_STARTED;
        if start != 0 {
            out.push(Action::Callback {
                word: start,
                argument: Argument::Object,
            });
        }
    }

    /// FUN_1403cae60 (`trap_obj_act_start`): ignored when locked or unknown.
    /// The running act is killed, the queue cleared and the priority zeroed
    /// before the new act is created.
    pub fn start(&mut self, name: &[u8]) -> Vec<Action> {
        let mut out = Vec::new();
        if self.locked() {
            return out;
        }
        let Some(index) = self.find(name) else {
            return out;
        };
        self.finish(&mut out);
        self.write = 0;
        self.read = 0;
        self.priority = 0;
        self.begin(index, &mut out);
        out
    }

    /// FUN_1403caa90: start `name` only when the running act's priority is not
    /// above the new act's (a priority-gated restart).
    pub fn start_if_not_outranked(&mut self, name: &[u8]) -> Vec<Action> {
        let mut out = Vec::new();
        if self.locked() {
            return out;
        }
        let Some(index) = self.find(name) else {
            return out;
        };
        if self.priority <= self.table[index].priority {
            self.finish(&mut out);
            self.write = 0;
            self.read = 0;
            self.priority = 0;
            self.begin(index, &mut out);
        }
        out
    }

    /// FUN_1403ca2c0: when no act runs, take the next queued entry and start
    /// it. Returns whether an act was started.
    pub fn pop(&mut self, out: &mut Vec<Action>) -> bool {
        while self.read != self.write {
            if self.priority != 0 {
                return false;
            }
            let index = self.queue[self.read];
            self.read = (self.read + 1) % QUEUE_SLOTS;
            if let Some(index) = index {
                self.begin(index, out);
                return true;
            }
        }
        false
    }

    /// FUN_1403ca550's end-of-thread handling: the act thread ended and no new
    /// act was started meanwhile.
    pub fn thread_ended(&mut self, out: &mut Vec<Action>) {
        if self.flags & JUST_STARTED == 0 {
            self.finish(out);
            self.priority = 0;
        }
    }
}

#[cfg(test)]
mod tests;
