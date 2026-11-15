use fb_sim::canon::Field;
use fb_sim::{Input, World};

/// What the rollback machinery needs from a simulation.
///
/// The real implementation is [`fb_sim::World`]. The trait exists so the
/// session logic can be fuzzed against a trivial simulation thousands of
/// times a second.
pub trait Simulation: Clone {
    fn step(&mut self, inputs: &[Input]);
    fn checksum(&self) -> u64;
    /// The canonical field list, for naming what differs after a desync.
    fn fields(&self) -> Vec<Field>;
}

impl Simulation for World {
    fn step(&mut self, inputs: &[Input]) {
        World::step(self, inputs);
    }

    fn checksum(&self) -> u64 {
        World::checksum(self)
    }

    fn fields(&self) -> Vec<Field> {
        World::fields(self)
    }
}
