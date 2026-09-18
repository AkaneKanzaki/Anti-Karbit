//! Multi-engine reverse image search.
//!
//! Engines are **not** run sequentially. Every active engine is fired at once
//! and results stream back as each one finishes, so the listener can start
//! claiming with the fastest result instead of waiting for the sum of all
//! engine latencies.
//!
//! Measured on `test_waifu.png`: 7.40 s sequential -> 2.78 s in parallel.
//!
//! All five engines are always constructed. Which ones actually run is decided
//! **at call time** from current configuration, so enabling SauceNAO, Ascii2d,
//! or Google Lens from the web dashboard takes effect immediately.

pub mod anilist;
pub mod ascii2d;
pub mod base;
pub mod iqdb;
pub mod lens;
pub mod saucenao;
pub mod tracemoe;

pub use base::CharacterInfo;

use std::sync::Arc;

use futures_util::StreamExt;
use tokio::sync::mpsc;
use tracing::info;

use crate::config;

/// Engine identity, used for log labels and ordering.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EngineKind {
    Iqdb,
    SauceNao,
    Ascii2d,
    TraceMoe,
    Lens,
}

impl EngineKind {
    pub fn label(self) -> &'static str {
        match self {
            EngineKind::Iqdb => "IQDB",
            EngineKind::SauceNao => "SauceNAO",
            EngineKind::Ascii2d => "Ascii2d",
            EngineKind::TraceMoe => "Trace.moe",
            EngineKind::Lens => "Google Lens",
        }
    }
}

/// A single search engine. An enum is used instead of `dyn Trait` so there is
/// no boxing or virtual dispatch on the hot path.
pub enum Engine {
    Iqdb(iqdb::IqdbRecognizer),
    SauceNao(saucenao::SauceNaoRecognizer),
    Ascii2d(ascii2d::Ascii2dRecognizer),
    TraceMoe(tracemoe::TraceMoeRecognizer),
    Lens(lens::GoogleLensRecognizer),
}

impl Engine {
    pub fn kind(&self) -> EngineKind {
        match self {
            Engine::Iqdb(_) => EngineKind::Iqdb,
            Engine::SauceNao(_) => EngineKind::SauceNao,
            Engine::Ascii2d(_) => EngineKind::Ascii2d,
            Engine::TraceMoe(_) => EngineKind::TraceMoe,
            Engine::Lens(_) => EngineKind::Lens,
        }
    }

    pub fn label(&self) -> &'static str {
        self.kind().label()
    }

    pub async fn identify(&self, image_bytes: &[u8]) -> Option<CharacterInfo> {
        match self {
            Engine::Iqdb(e) => e.identify(image_bytes).await,
            Engine::SauceNao(e) => e.identify(image_bytes).await,
            Engine::Ascii2d(e) => e.identify(image_bytes).await,
            Engine::TraceMoe(e) => e.identify(image_bytes).await,
            Engine::Lens(e) => e.identify(image_bytes).await,
        }
    }
}

/// Every engine the application knows about.
pub struct MultiEngine {
    all: Vec<Arc<Engine>>,
}

impl MultiEngine {
    pub fn new() -> Self {
        let all = vec![
            Arc::new(Engine::Iqdb(iqdb::IqdbRecognizer)),
            Arc::new(Engine::SauceNao(saucenao::SauceNaoRecognizer)),
            Arc::new(Engine::Ascii2d(ascii2d::Ascii2dRecognizer)),
            Arc::new(Engine::TraceMoe(tracemoe::TraceMoeRecognizer)),
            Arc::new(Engine::Lens(lens::GoogleLensRecognizer)),
        ];

        let labels: Vec<&str> = all.iter().map(|e| e.label()).collect();
        info!("Engines available: {}", labels.join(", "));

        Self { all }
    }

    /// The engines that are active under current configuration.
    ///
    /// SauceNAO only when an API key is set; Google Lens and Ascii2d follow
    /// their own enable flags. IQDB and Trace.moe are always on.
    pub fn active(&self) -> Vec<Arc<Engine>> {
        let cfg = config::get();
        self.all
            .iter()
            .filter(|engine| match engine.kind() {
                EngineKind::SauceNao => !cfg.saucenao_api_key.trim().is_empty(),
                EngineKind::Lens => cfg.lens_enabled,
                EngineKind::Ascii2d => cfg.ascii2d_enabled,
                EngineKind::Iqdb | EngineKind::TraceMoe => true,
            })
            .cloned()
            .collect()
    }

    /// Labels of the active engines, for logging.
    pub fn active_labels(&self) -> Vec<&'static str> {
        self.active().iter().map(|e| e.label()).collect()
    }

    /// Jalankan **semua engine aktif secara paralel**.
    ///
    /// Returns a receiver that yields `(engine, result)` as each engine
    /// finishes, not once they all have. That lets the listener claim with the
    /// fastest result while the others are still running as backup.
    pub fn spawn_parallel(
        &self,
        image_bytes: Arc<Vec<u8>>,
    ) -> mpsc::Receiver<(EngineKind, Option<CharacterInfo>)> {
        let engines = self.active();
        let (tx, rx) = mpsc::channel(engines.len().max(1));

        tokio::spawn(async move {
            let mut tasks = futures_util::stream::FuturesUnordered::new();

            for engine in engines {
                let bytes = image_bytes.clone();
                let tx = tx.clone();
                tasks.push(tokio::spawn(async move {
                    let kind = engine.kind();
                    let result = engine.identify(&bytes).await;
                    let _ = tx.send((kind, result)).await;
                }));
            }

            drop(tx);
            while tasks.next().await.is_some() {}
        });

        rx
    }

    /// Wait for every active engine and return the results that were found,
    /// ordered by priority. Used by the test-image command.
    pub async fn identify_all(&self, image_bytes: &[u8]) -> Vec<(EngineKind, CharacterInfo)> {
        let shared = Arc::new(image_bytes.to_vec());
        let mut rx = self.spawn_parallel(shared);

        let mut results: Vec<(EngineKind, CharacterInfo)> = Vec::new();
        while let Some((kind, result)) = rx.recv().await {
            if let Some(info) = result {
                results.push((kind, info));
            }
        }

        results.sort_by_key(|(kind, _)| *kind as u8);
        results
    }
}

impl Default for MultiEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iqdb_dan_tracemoe_selalu_aktif() {
        let engine = MultiEngine::new();
        let labels = engine.active_labels();
        assert!(labels.contains(&"IQDB"));
        assert!(labels.contains(&"Trace.moe"));
    }

    #[test]
    fn semua_engine_selalu_dibangun() {
        // All five engines are built once; filtering happens at call time so
        // dashboard changes take effect immediately.
        let engine = MultiEngine::new();
        assert_eq!(engine.all.len(), 5);
    }

    #[test]
    fn ascii2d_ikut_aktif_saat_dinyalakan() {
        let engine = MultiEngine::new();
        let labels = engine.active_labels();
        assert!(
            labels.contains(&"Ascii2d"),
            "Ascii2d harus aktif selama ASCII2D_ENABLED tidak dimatikan"
        );
    }
}
