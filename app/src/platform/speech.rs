//! The models on macOS: ONNX Runtime loaded from the library inside the app,
//! Silero VAD driven directly, and Parakeet TDT 0.6B v3 through parakeet-rs. The models are read
//! from the models folder; nothing is fetched here.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use anyhow::{Context, Result, anyhow};
use parakeet_rs::ort::environment::Environment;
use parakeet_rs::ort::session::Session;
use parakeet_rs::ort::value::Tensor;
use parakeet_rs::{ExecutionConfig, ExecutionProvider, ParakeetTDT, Transcriber};

use sextant_core::audio::FRAME;

use crate::audio::{Runs, Speech, Vad};

const RUNTIME: &str = "Frameworks/libonnxruntime.dylib";
const SILERO: &str = "vad/silero_vad.onnx";
const PARAKEET: &str = "parakeet";
const COREML_CACHE: &str = "coreml-cache";

/// Samples of the previous frame that Silero wants in front of each new one.
const CONTEXT: usize = 64;
const STATE: usize = 2 * 128;

/// The library signed into the app bundle (`Sextant.app/Contents/Frameworks`), so the hardened
/// runtime accepts it. `ORT_DYLIB_PATH` points dev builds, which have no bundle, at the copy
/// `scripts/fetch-onnxruntime.sh` leaves in `app/frameworks`.
fn runtime() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("ORT_DYLIB_PATH") {
        return Ok(path.into());
    }
    let exe = std::env::current_exe().context("find the app")?;
    let contents = exe.parent().and_then(Path::parent);
    contents
        .map(|contents| contents.join(RUNTIME))
        .ok_or_else(|| anyhow!("the app is not inside a bundle"))
}

/// ONNX Runtime allows one environment per process, so it is made on first use and shared.
fn environment() -> Result<Environment> {
    static ENVIRONMENT: OnceLock<Mutex<Option<Environment>>> = OnceLock::new();
    let mut slot = ENVIRONMENT
        .get_or_init(|| Mutex::new(None))
        .lock()
        .map_err(|_| anyhow!("the ONNX Runtime state is poisoned"))?;
    if let Some(environment) = slot.as_ref() {
        return Ok(environment.clone());
    }
    let environment = parakeet_rs::ort::init_from(runtime()?)
        .context("load ONNX Runtime")?
        .build()
        .context("start ONNX Runtime")?;
    *slot = Some(environment.clone());
    Ok(environment)
}

pub struct Parakeet {
    models: PathBuf,
    runs: Runs,
    loaded: Option<ParakeetTDT>,
}

impl Parakeet {
    pub fn new(models: &Path, runs: Runs) -> Self {
        Self {
            models: models.to_path_buf(),
            runs,
            loaded: None,
        }
    }
}

impl Speech for Parakeet {
    fn load(&mut self) -> Result<()> {
        let provider = match self.runs {
            Runs::Cpu => ExecutionProvider::Cpu,
            Runs::CoreMl => ExecutionProvider::CoreML,
        };
        let threads = std::thread::available_parallelism().map_or(4, |n| n.get().min(4));
        let config = ExecutionConfig::new()
            .with_execution_provider(provider)
            .with_intra_threads(threads)
            .with_coreml_cache_dir(self.models.join(COREML_CACHE))
            .with_custom_configure(|builder| Ok(builder.with_prepacking(false)?))
            .with_environment(environment()?);
        let model = ParakeetTDT::from_pretrained(self.models.join(PARAKEET), Some(config))
            .map_err(|err| anyhow!("load Parakeet: {err}"))?;
        self.loaded = Some(model);
        Ok(())
    }

    fn unload(&mut self) {
        self.loaded = None;
    }

    fn transcribe(&mut self, audio: &[f32]) -> Result<String> {
        let model = self.loaded.as_mut().context("Parakeet is not loaded")?;
        let result = model
            .transcribe_samples(audio.to_vec(), sextant_core::audio::RATE, 1, None)
            .map_err(|err| anyhow!("transcribe: {err}"))?;
        Ok(result.text)
    }
}

/// Silero VAD for one stream: the model plus the state it carries from frame to frame.
pub struct Silero {
    session: Session,
    state: Vec<f32>,
    context: Vec<f32>,
    failing: bool,
}

/// Makes a Silero session per stream. The model file is small and ONNX Runtime shares the rest.
pub fn silero(models: &Path) -> impl FnMut() -> Result<Silero> + Send + 'static + use<> {
    let models = models.to_path_buf();
    move || {
        let session = Session::builder(&environment()?)
            .map_err(|err| anyhow!("{err}"))?
            .with_intra_threads(1)
            .map_err(|err| anyhow!("{err}"))?
            .commit_from_file(models.join(SILERO))
            .map_err(|err| anyhow!("load Silero: {err}"))?;
        Ok(Silero {
            session,
            state: vec![0.0; STATE],
            context: vec![0.0; CONTEXT],
            failing: false,
        })
    }
}

impl Vad for Silero {
    fn probability(&mut self, frame: &[f32]) -> f32 {
        match self.run(frame) {
            Ok(probability) => {
                self.failing = false;
                probability
            }
            Err(err) => {
                if !self.failing {
                    eprintln!("voice detection failed: {err:#}");
                }
                self.failing = true;
                0.0
            }
        }
    }
}

impl Silero {
    fn run(&mut self, frame: &[f32]) -> Result<f32> {
        let mut input = Vec::with_capacity(CONTEXT + FRAME);
        input.extend_from_slice(&self.context);
        input.extend_from_slice(frame);
        let outputs = self
            .session
            .run(parakeet_rs::ort::inputs! {
                "input" => Tensor::from_array(([1, input.len()], input.clone()))?,
                "state" => Tensor::from_array(([2, 1, 128], self.state.clone()))?,
                "sr" => Tensor::from_array(((), vec![sextant_core::audio::RATE as i64]))?,
            })
            .map_err(|err| anyhow!("{err}"))?;
        let (_, probability) = outputs["output"].try_extract_tensor::<f32>()?;
        let (_, state) = outputs["stateN"].try_extract_tensor::<f32>()?;
        self.state.copy_from_slice(state);
        self.context
            .copy_from_slice(&input[input.len() - CONTEXT..]);
        Ok(probability.first().copied().unwrap_or(0.0))
    }
}
