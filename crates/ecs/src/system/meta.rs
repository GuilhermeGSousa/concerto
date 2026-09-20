/// Properties of a system that affect how an executor may run it, gathered
/// from its inputs alongside [`SystemAccess`](crate::system::access::SystemAccess).
#[derive(Clone, Debug)]
pub struct SystemMetadata {
    is_send: bool,
}

impl Default for SystemMetadata {
    fn default() -> Self {
        Self { is_send: true }
    }
}

impl SystemMetadata {
    /// Marks the system as unable to run off the thread driving the schedule.
    pub fn set_non_send(&mut self) {
        self.is_send = false;
    }

    pub fn is_send(&self) -> bool {
        self.is_send
    }
}
