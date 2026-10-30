//! Differential testing: compare what the real client puts on the wire with
//! what Seihai puts on the wire for the same scripted session.
//!
//! * [`pcap`]: read a capture and rebuild the TCP streams.
//! * [`decode`]: split the streams into frames and messages with Seihai's
//!   own codecs and layouts, mask the session-specific values, and produce a
//!   [`trace::Trace`].
//! * [`diff`]: compare a real-client trace with a Seihai trace and rank the
//!   differences.
//!
//! The `difftest` binary wraps both steps; `tools/difftest/lab.sh` runs the
//! scenarios. See `documentation/difftest.md`.

pub mod decode;
pub mod diff;
pub mod names;
pub mod pcap;
pub mod trace;
