//! Controls for a finite Sound Blaster PCM buffer on the machine timeline.
use crate::device::sb::{DacState, SbControl};
use crate::{DmaChannel, Tick};
mod mixer;
pub use mixer::{SfxMixer, SfxVoiceHandle};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

struct SfxState {
    stopped: AtomicBool,
    finished: AtomicBool,
    position: AtomicUsize,
    handles: AtomicUsize,
}

/// Cloneable controls for one unsigned 8-bit mono PCM buffer.
///
/// Position and completion follow the machine clock as `DosAudio` advances,
/// ahead of audio output by its configured latency. Dropping the last handle
/// requests a stop. Stop takes effect at the current machine time on the next
/// successful time advance or port access; it cannot retract samples already queued.
pub struct SfxHandle {
    state: Arc<SfxState>,
    period_micros: u16,
}

impl SfxHandle {
    pub(crate) fn new(period_micros: u16) -> Self {
        Self {
            state: Arc::new(SfxState {
                stopped: AtomicBool::new(false),
                finished: AtomicBool::new(false),
                position: AtomicUsize::new(0),
                handles: AtomicUsize::new(1),
            }),
            period_micros,
        }
    }

    /// Stop this buffer on the next successful time advance or port access.
    pub fn stop(&self) {
        self.state.stopped.store(true, Ordering::Release);
    }

    /// Whether a stop was requested, the buffer ended, or ownership was replaced.
    pub fn is_finished(&self) -> bool {
        self.state.finished.load(Ordering::Acquire) || self.state.stopped.load(Ordering::Acquire)
    }

    /// Number of bytes delivered to the DSP, independent of output latency.
    pub fn samples_played(&self) -> usize {
        self.state.position.load(Ordering::Acquire)
    }

    /// Actual DAC rate after quantizing the requested rate to a DSP time constant.
    pub fn sample_rate(&self) -> f64 {
        1_000_000.0 / f64::from(self.period_micros)
    }
}

impl Clone for SfxHandle {
    fn clone(&self) -> Self {
        self.state.handles.fetch_add(1, Ordering::Relaxed);
        Self {
            state: Arc::clone(&self.state),
            period_micros: self.period_micros,
        }
    }
}

impl Drop for SfxHandle {
    fn drop(&mut self) {
        if self.state.handles.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.stop();
        }
    }
}

pub(crate) struct PcmPlayback {
    samples: Arc<[u8]>,
    state: Arc<SfxState>,
    index: usize,
    end: Tick,
}

#[derive(Clone, Copy)]
pub(crate) struct PcmPlan {
    pub sb: SbControl,
    pub commands: usize,
    index: usize,
    finished: bool,
    stopped: bool,
}

impl PcmPlayback {
    pub fn new(samples: Arc<[u8]>, handle: &SfxHandle, end: Tick) -> Self {
        Self {
            samples,
            state: Arc::clone(&handle.state),
            index: 0,
            end,
        }
    }

    pub fn next(&self, sb: &SbControl, now: Tick) -> Tick {
        if self.state.stopped.load(Ordering::Acquire) {
            now
        } else if self.index == self.samples.len() {
            self.end
        } else {
            sb.next_dma_tick().expect("buffer owns active DSP DMA")
        }
    }

    pub fn end(&self, now: Tick) -> Tick {
        if self.state.stopped.load(Ordering::Acquire) {
            now
        } else {
            self.end
        }
    }

    // Walk twice: first reserve the entire transaction, then emit into the SPSC
    // queue. Only the control thread owns samples and handles; render never drops
    // either, including the final Arc reference.
    fn walk(
        &self,
        sb: &mut SbControl,
        now: Tick,
        at: Tick,
        stopped: bool,
        mut emit: impl FnMut(Tick, DacState),
    ) -> (usize, bool) {
        if stopped {
            emit(now, sb.stop_pcm());
            return (self.index, true);
        }
        let mut index = self.index;
        let channel = DmaChannel::new(sb.resources.dma).expect("validated DMA");
        while index < self.samples.len() {
            let next = sb.next_dma_tick().expect("buffer owns active DSP DMA");
            if next > at {
                break;
            }
            let state = sb
                .dma_write(next, channel, self.samples[index])
                .expect("on-time buffer DMA");
            emit(next, state);
            index += 1;
        }
        let finished = index == self.samples.len() && self.end <= at;
        if finished {
            emit(self.end, sb.stop_pcm());
        }
        (index, finished)
    }

    pub fn plan(&self, mut sb: SbControl, now: Tick, at: Tick) -> PcmPlan {
        let stopped = self.state.stopped.load(Ordering::Acquire);
        let mut commands = 0;
        let (index, finished) = self.walk(&mut sb, now, at, stopped, |_, _| commands += 1);
        PcmPlan {
            sb,
            commands,
            index,
            finished,
            stopped,
        }
    }

    pub fn commit(
        &mut self,
        plan: PcmPlan,
        mut sb: SbControl,
        now: Tick,
        at: Tick,
        emit: impl FnMut(Tick, DacState),
    ) -> bool {
        self.walk(&mut sb, now, at, plan.stopped, emit);
        self.index = plan.index;
        self.state.position.store(self.index, Ordering::Release);
        self.state.finished.store(plan.finished, Ordering::Release);
        plan.finished
    }
}

impl Drop for PcmPlayback {
    fn drop(&mut self) {
        self.state.finished.store(true, Ordering::Release);
    }
}
