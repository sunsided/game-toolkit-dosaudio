use super::{Capture, PortWrite};
use crate::{Error, OplChip, Result, Tick};
use rtrb::{Consumer, Producer};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

#[derive(Clone, Copy)]
struct RegisterWrite {
    at: Tick,
    register: u16,
    value: u8,
}

struct State {
    stop: AtomicBool,
    finished: AtomicBool,
    overflowed: AtomicBool,
    end: AtomicU64,
}

/// Control-thread reader for synthesis register writes applied by the audio engine.
///
/// Poll [`Self::try_finish`] regularly to drain the bounded queue. Capture includes
/// manual, prepared and live OPL writes and note releases at their actual output
/// times. It records registers rather than PCM: fades, post-chip gain, analog
/// mixing, MIDI and PC speaker operations are not representable in DRO.
/// Start before programming instruments for a self-contained replay.
///
/// Stopping is acknowledged at an output-frame boundary. Keep rendering until
/// [`Self::is_finished`] becomes true, then collect the result on this thread.
/// Overflow permanently invalidates the capture and is reported explicitly.
/// Dropping this reader requests a stop; callback-owned resources remain owned
/// by the engine until it is safely dropped or reconfigured outside rendering.
pub struct MusicCapture {
    incoming: Consumer<RegisterWrite>,
    state: Arc<State>,
    chip: OplChip,
    start: Tick,
    writes: Vec<PortWrite>,
    consumed: bool,
}

pub(crate) struct CaptureWriter {
    outgoing: Producer<RegisterWrite>,
    state: Arc<State>,
}

pub(crate) fn stream(
    chip: OplChip,
    start: Tick,
    capacity: usize,
) -> Result<(MusicCapture, CaptureWriter)> {
    if capacity == 0 || capacity > 1_048_576 {
        return Err(Error::Config(
            "capture queue capacity must be 1..=1,048,576",
        ));
    }
    let (outgoing, incoming) = rtrb::RingBuffer::new(capacity);
    let state = Arc::new(State {
        stop: AtomicBool::new(false),
        finished: AtomicBool::new(false),
        overflowed: AtomicBool::new(false),
        end: AtomicU64::new(start.0),
    });
    Ok((
        MusicCapture {
            incoming,
            state: Arc::clone(&state),
            chip,
            start,
            writes: Vec::new(),
            consumed: false,
        },
        CaptureWriter { outgoing, state },
    ))
}

impl MusicCapture {
    /// Request capture stop at the next output-frame boundary. Idempotent.
    pub fn stop(&self) {
        self.state.stop.store(true, Ordering::Release);
    }

    /// Whether the callback has acknowledged stop or capture overflow.
    pub fn is_finished(&self) -> bool {
        self.state.finished.load(Ordering::Acquire)
    }

    /// Whether the bounded queue has lost an event, invalidating this capture.
    pub fn is_overflowed(&self) -> bool {
        self.state.overflowed.load(Ordering::Acquire)
    }

    /// Drain pending writes on this thread and take an acknowledged capture once.
    ///
    /// Returns `None` until a frame boundary acknowledges stop, and after the
    /// result has been consumed. Queue overflow returns [`Error::CaptureOverflow`]
    /// once instead of a truncated capture. All conversion allocations occur here.
    pub fn try_finish(&mut self) -> Result<Option<Capture>> {
        if self.consumed {
            return Ok(None);
        }
        // Observe the completion publication before draining: once observed, the
        // producer cannot append more writes and all its final pushes are visible.
        let finished = self.is_finished();
        while let Ok(write) = self.incoming.pop() {
            if !self.is_overflowed() {
                let address = if write.register & 0x100 == 0 {
                    0x388
                } else {
                    0x38a
                };
                self.writes.extend([
                    PortWrite {
                        at: write.at,
                        port: address,
                        value: write.register as u8,
                    },
                    PortWrite {
                        at: write.at,
                        port: address + 1,
                        value: write.value,
                    },
                ]);
            }
        }
        if !finished {
            return Ok(None);
        }
        self.consumed = true;
        if self.is_overflowed() {
            self.writes.clear();
            return Err(Error::CaptureOverflow);
        }
        Ok(Some(Capture {
            start: self.start,
            end: Tick(self.state.end.load(Ordering::Acquire)),
            chip: Some(self.chip),
            address: [0; 2],
            card: None,
            writes: std::mem::take(&mut self.writes),
        }))
    }
}

impl Drop for MusicCapture {
    fn drop(&mut self) {
        self.stop();
    }
}

impl CaptureWriter {
    #[cfg(feature = "opl")]
    pub(crate) fn record(&mut self, at: Tick, register: u16, value: u8) {
        if self.is_finished()
            || self.state.overflowed.load(Ordering::Acquire)
            || self.outgoing.is_abandoned()
        {
            return;
        }
        if self
            .outgoing
            .push(RegisterWrite {
                at,
                register,
                value,
            })
            .is_err()
        {
            self.state.overflowed.store(true, Ordering::Release);
        }
    }

    pub(crate) fn boundary(&mut self, at: Tick) {
        if self.is_finished() {
            return;
        }
        self.state.end.store(at.0, Ordering::Relaxed);
        if self.state.stop.load(Ordering::Acquire)
            || self.state.overflowed.load(Ordering::Acquire)
            || self.outgoing.is_abandoned()
        {
            self.state.finished.store(true, Ordering::Release);
        }
    }

    pub(crate) fn is_finished(&self) -> bool {
        self.state.finished.load(Ordering::Acquire)
    }
}
