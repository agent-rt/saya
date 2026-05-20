use std::sync::{Arc, Mutex, Weak};
use std::thread;
use std::time::{Duration, Instant};

use candle_core::{D, DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::bert::{BertModel, Config};
use hf_hub::api::sync::Api;
use tokenizers::{PaddingParams, Tokenizer};

use super::EMBEDDING_DIM;

pub const MODEL_REPO: &str = "sentence-transformers/all-MiniLM-L6-v2";
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const IDLE_CHECK_INTERVAL: Duration = Duration::from_secs(60);

/// Heavyweight model + tokenizer state. Construction downloads weights on
/// first use (cached by hf-hub) and binds them to a Metal device when
/// available, falling back to CPU otherwise.
pub struct Embedder {
    model: BertModel,
    tokenizer: Tokenizer,
    device: Device,
}

impl Embedder {
    pub fn load() -> crate::Result<Self> {
        let started = Instant::now();
        let api = Api::new().map_err(|e| other(format!("hf api init: {e}")))?;
        let repo = api.model(MODEL_REPO.to_string());

        let config_path = repo
            .get("config.json")
            .map_err(|e| other(format!("fetch config.json: {e}")))?;
        let tokenizer_path = repo
            .get("tokenizer.json")
            .map_err(|e| other(format!("fetch tokenizer.json: {e}")))?;
        let weights_path = repo
            .get("model.safetensors")
            .map_err(|e| other(format!("fetch model.safetensors: {e}")))?;

        let config_bytes = std::fs::read(&config_path)?;
        let config: Config = serde_json::from_slice(&config_bytes)
            .map_err(|e| other(format!("parse config.json: {e}")))?;

        let mut tokenizer =
            Tokenizer::from_file(&tokenizer_path).map_err(|e| other(format!("tokenizer: {e}")))?;
        tokenizer.with_padding(Some(PaddingParams::default()));

        let device = match Device::new_metal(0) {
            Ok(d) => {
                tracing::info!("embedder using Metal device");
                d
            }
            Err(e) => {
                tracing::warn!("Metal unavailable ({e}), falling back to CPU");
                Device::Cpu
            }
        };

        // SAFETY: mmap is held by Candle for the duration of tensors derived
        // from it. For Metal we copy to GPU, but Candle still requires
        // unsafe here because it's a memory mapping.
        let vb = unsafe {
            VarBuilder::from_mmaped_safetensors(&[weights_path], DType::F32, &device)
        }
        .map_err(|e| other(format!("varbuilder: {e}")))?;
        let model =
            BertModel::load(vb, &config).map_err(|e| other(format!("bert load: {e}")))?;

        tracing::info!("embedder loaded in {:?}", started.elapsed());
        Ok(Self { model, tokenizer, device })
    }

    pub fn embed(&self, texts: &[&str]) -> crate::Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let encodings = self
            .tokenizer
            .encode_batch(texts.to_vec(), true)
            .map_err(|e| other(format!("tokenize: {e}")))?;

        let batch = encodings.len();
        let seq_len = encodings[0].len();

        let mut input_ids = Vec::with_capacity(batch * seq_len);
        let mut attention_mask = Vec::with_capacity(batch * seq_len);
        let mut token_type_ids = Vec::with_capacity(batch * seq_len);
        for enc in &encodings {
            input_ids.extend(enc.get_ids().iter().map(|&u| u as i64));
            attention_mask.extend(enc.get_attention_mask().iter().map(|&u| u as i64));
            token_type_ids.extend(enc.get_type_ids().iter().map(|&u| u as i64));
        }

        let me = |e: candle_core::Error| other(format!("candle: {e}"));
        let input_ids = Tensor::from_vec(input_ids, (batch, seq_len), &self.device).map_err(me)?;
        let attention_mask =
            Tensor::from_vec(attention_mask, (batch, seq_len), &self.device).map_err(me)?;
        let token_type_ids =
            Tensor::from_vec(token_type_ids, (batch, seq_len), &self.device).map_err(me)?;

        let hidden = self
            .model
            .forward(&input_ids, &token_type_ids, Some(&attention_mask))
            .map_err(me)?;

        let mask_f = attention_mask
            .to_dtype(DType::F32)
            .map_err(me)?
            .unsqueeze(2)
            .map_err(me)?;
        let masked = hidden.broadcast_mul(&mask_f).map_err(me)?;
        let summed = masked.sum(1).map_err(me)?;
        let counts = mask_f.sum(1).map_err(me)?;
        let mean = summed.broadcast_div(&counts).map_err(me)?;

        let norm = mean
            .sqr()
            .map_err(me)?
            .sum_keepdim(D::Minus1)
            .map_err(me)?
            .sqrt()
            .map_err(me)?;
        let normalized = mean.broadcast_div(&norm).map_err(me)?;

        let vecs: Vec<Vec<f32>> = normalized.to_vec2::<f32>().map_err(me)?;
        if let Some(first) = vecs.first()
            && first.len() != EMBEDDING_DIM
        {
            return Err(other(format!(
                "expected {EMBEDDING_DIM}-dim, got {}",
                first.len()
            )));
        }
        Ok(vecs)
    }
}

/// Thread-safe handle that lazy-loads the heavy `Embedder` on first use and
/// unloads it after `IDLE_TIMEOUT` of inactivity, releasing the Metal context.
#[derive(Clone)]
pub struct EmbedderHandle {
    inner: Arc<Mutex<EmbedderInner>>,
}

struct EmbedderInner {
    embedder: Option<Embedder>,
    last_used: Instant,
}

impl EmbedderHandle {
    pub fn new() -> Self {
        let inner = Arc::new(Mutex::new(EmbedderInner {
            embedder: None,
            last_used: Instant::now(),
        }));
        let watcher = Arc::downgrade(&inner);
        thread::Builder::new()
            .name("saya-embedder-idle".into())
            .spawn(move || idle_watcher(watcher))
            .ok();
        Self { inner }
    }

    pub fn embed(&self, texts: &[&str]) -> crate::Result<Vec<Vec<f32>>> {
        let mut guard = self.inner.lock().expect("embedder mutex poisoned");
        if guard.embedder.is_none() {
            guard.embedder = Some(Embedder::load()?);
        }
        let result = guard.embedder.as_ref().unwrap().embed(texts);
        guard.last_used = Instant::now();
        result
    }

    pub fn embed_one(&self, text: &str) -> crate::Result<Vec<f32>> {
        let mut v = self.embed(&[text])?;
        v.pop()
            .ok_or_else(|| other("empty embed result".into()))
    }

    pub fn is_loaded(&self) -> bool {
        self.inner.lock().expect("embedder mutex poisoned").embedder.is_some()
    }

    pub fn unload(&self) {
        let mut guard = self.inner.lock().expect("embedder mutex poisoned");
        guard.embedder = None;
    }
}

impl Default for EmbedderHandle {
    fn default() -> Self {
        Self::new()
    }
}

fn idle_watcher(weak: Weak<Mutex<EmbedderInner>>) {
    loop {
        thread::sleep(IDLE_CHECK_INTERVAL);
        let Some(inner) = weak.upgrade() else {
            break;
        };
        let mut guard = match inner.lock() {
            Ok(g) => g,
            Err(_) => break,
        };
        if guard.embedder.is_some() && guard.last_used.elapsed() > IDLE_TIMEOUT {
            tracing::info!("unloading embedder after {:?} idle", guard.last_used.elapsed());
            guard.embedder = None;
        }
    }
}

fn other(msg: String) -> crate::Error {
    crate::Error::Other(msg)
}
