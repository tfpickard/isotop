//! The rasterizer behind a frame: the GPU when one was found and keeps working, else the CPU.
//!
//! A frame is rasterized in one of two ways. `rasterize` returns when the pixels are there, which
//! is what a headless run (`--output`, `--benchmark`) and the tests use. `overlap` is for the
//! interactive loop: the GPU draws one frame while the caller records the next, so the frame it
//! hands back is the one before, whole (pixels, picks, labels and the rest of what was recorded
//! with it), and what the user sees is one frame behind what the scene has just recorded.

use crate::cli::{Power, RendererChoice};
use crate::gpu::{Flight, Gpu, Picks, Stats};
use crate::render::Frame;

/// A frame the GPU has been given and has not finished.
struct Flying {
    flight: Flight,
    frame: Frame,
}

/// Turns a recorded frame into pixels, and says which rasterizer did.
pub(crate) struct Renderer {
    gpu: Option<Gpu>,
    /// Which rasterizer draws the frames, for the status strip.
    kind: String,
    /// Whether the GPU brings back every pick of a frame, not only those around the pointer.
    every_pick: bool,
    /// Whether `overlap` lets the GPU run a frame behind.
    pipelined: bool,
    /// The frame `overlap` has submitted and not yet handed back.
    flying: Option<Flying>,
}

impl Renderer {
    /// The CPU rasterizer.
    pub(crate) fn cpu() -> Self {
        Self::with(None, "cpu".into())
    }

    fn with(gpu: Option<Gpu>, kind: String) -> Self {
        Self {
            gpu,
            kind,
            every_pick: false,
            pipelined: false,
            flying: None,
        }
    }

    /// The rasterizer `choice` asks for. `Auto` falls back to the CPU when no GPU can be opened,
    /// saying why in `kind`; `Gpu` fails instead.
    pub(crate) fn choose(choice: RendererChoice, power: Power) -> Result<Self, String> {
        if choice == RendererChoice::Cpu {
            return Ok(Self::cpu());
        }
        let preference = match power {
            Power::Low => wgpu::PowerPreference::LowPower,
            Power::High => wgpu::PowerPreference::HighPerformance,
        };
        match Gpu::new(preference) {
            Ok(gpu) => {
                let kind = format!("gpu {}", gpu.name);
                Ok(Self::with(Some(gpu), kind))
            }
            Err(error) if choice == RendererChoice::Auto => {
                Ok(Self::with(None, format!("cpu ({error})")))
            }
            Err(error) => Err(error),
        }
    }

    /// Reads back every pick of a GPU frame instead of a window around the pointer: for a run
    /// that checks picks or writes them down, where there is no pointer.
    pub(crate) fn reading_every_pick(mut self, every_pick: bool) -> Self {
        self.every_pick = every_pick;
        self
    }

    /// Lets `overlap` run the GPU a frame behind (`true`) or finish each frame at once (`false`,
    /// which is how a renderer starts). A frame still in flight is finished first.
    pub(crate) fn set_pipelined(&mut self, pipelined: bool) -> Option<Frame> {
        let flushed = self.flush();
        self.pipelined = pipelined;
        flushed
    }

    /// What drew the last frame: `cpu`, `gpu <adapter>`, or a CPU fallback with its reason.
    pub(crate) fn kind(&self) -> &str {
        &self.kind
    }

    /// The draw calls of the last frame the GPU drew, None on the CPU.
    pub(crate) fn draw_stats(&self) -> Option<Stats> {
        self.gpu.as_ref().map(Gpu::stats)
    }

    /// Which of the pick buffer the GPU reads back for a pointer at `pointer`.
    fn picks(&self, pointer: Option<[f32; 2]>) -> Picks {
        match pointer {
            _ if self.every_pick => Picks::Everywhere,
            Some(pointer) => Picks::Around(pointer),
            None => Picks::Nowhere,
        }
    }

    /// Fills `frame`'s colour, depth and pick buffers. A GPU that fails is given up for the CPU,
    /// which then draws this frame; `pointer` is the frame position the GPU reads the pick under.
    pub(crate) fn rasterize(&mut self, frame: &mut Frame, pointer: Option<[f32; 2]>) {
        let picks = self.picks(pointer);
        let failure = match self.gpu.as_mut() {
            Some(gpu) => gpu.render(frame, picks).err(),
            None => {
                frame.rasterize();
                None
            }
        };
        if let Some(error) = failure {
            self.fail(&error);
            frame.rasterize();
        }
    }

    /// Gives `frame` to the GPU and returns the frame it was given before, finished: with the CPU,
    /// or with the GPU not allowed to run behind, this frame itself, at once. None for the first
    /// frame the GPU is given, which has nothing before it. The frame handed back is whole, so
    /// its picks and its overlay data agree with its pixels; the next call hands back this one.
    pub(crate) fn overlap(&mut self, mut frame: Frame, pointer: Option<[f32; 2]>) -> Option<Frame> {
        if self.gpu.is_none() || !self.pipelined {
            let earlier = self.flush();
            self.rasterize(&mut frame, pointer);
            // A frame flushed here is older than this one, and is dropped for it.
            drop(earlier);
            return Some(frame);
        }
        let picks = self.picks(pointer);
        let gpu = self.gpu.as_mut()?;
        let submitted = gpu.submit(&frame, picks);
        let before = self.flying.take();
        let mut failure = None;
        let ready = match submitted {
            Ok(flight) => {
                let ready = before.map(
                    |Flying {
                         flight: old,
                         frame: mut ready,
                     }| {
                        if let Err(error) = gpu.collect(old, &mut ready) {
                            failure = Some(error);
                            ready.rasterize();
                        }
                        ready
                    },
                );
                self.flying = Some(Flying { flight, frame });
                ready
            }
            Err(error) => {
                failure = Some(error);
                before.map(
                    |Flying {
                         flight,
                         frame: mut ready,
                     }| {
                        drop(flight);
                        ready.rasterize();
                        ready
                    },
                )
            }
        };
        if let Some(error) = failure {
            self.fail(&error);
        }
        ready
    }

    /// Finishes the frame `overlap` has in flight, if any, and hands it back.
    pub(crate) fn flush(&mut self) -> Option<Frame> {
        let Flying { flight, mut frame } = self.flying.take()?;
        match self.gpu.as_mut().map(|gpu| gpu.collect(flight, &mut frame)) {
            Some(Ok(())) => {}
            Some(Err(error)) => {
                self.fail(&error);
                frame.rasterize();
            }
            None => frame.rasterize(),
        }
        Some(frame)
    }

    /// Gives the GPU up for the CPU, and lets go of a frame it still holds.
    fn fail(&mut self, error: &str) {
        self.gpu = None;
        self.flying = None;
        self.kind = format!("cpu (GPU failed: {error})");
    }
}

impl Default for Renderer {
    fn default() -> Self {
        Self::cpu()
    }
}
