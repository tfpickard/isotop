//! The rasterizer behind a frame: the GPU when one was found and keeps working, else the CPU.

use crate::cli::{Power, RendererChoice};
use crate::gpu::Gpu;
use crate::render::Frame;

/// Turns a recorded frame into pixels, and says which rasterizer did.
pub(crate) struct Renderer {
    gpu: Option<Gpu>,
    /// Which rasterizer draws the frames, for the status strip.
    kind: String,
}

impl Renderer {
    /// The CPU rasterizer.
    pub(crate) fn cpu() -> Self {
        Self {
            gpu: None,
            kind: "cpu".into(),
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
            Ok(gpu) => Ok(Self {
                kind: format!("gpu {}", gpu.name),
                gpu: Some(gpu),
            }),
            Err(error) if choice == RendererChoice::Auto => Ok(Self {
                gpu: None,
                kind: format!("cpu ({error})"),
            }),
            Err(error) => Err(error),
        }
    }

    /// What drew the last frame: `cpu`, `gpu <adapter>`, or a CPU fallback with its reason.
    pub(crate) fn kind(&self) -> &str {
        &self.kind
    }

    /// Fills `frame`'s colour, depth and pick buffers. A GPU that fails is given up for the CPU,
    /// which then draws this frame; `pointer` is the frame position the GPU reads the pick under.
    pub(crate) fn rasterize(&mut self, frame: &mut Frame, pointer: Option<[f32; 2]>) {
        let failure = match self.gpu.as_mut() {
            Some(gpu) => gpu.render(frame, pointer).err(),
            None => {
                frame.rasterize();
                None
            }
        };
        if let Some(error) = failure {
            self.gpu = None;
            self.kind = format!("cpu (GPU failed: {error})");
            frame.rasterize();
        }
    }
}

impl Default for Renderer {
    fn default() -> Self {
        Self::cpu()
    }
}
