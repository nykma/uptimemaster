use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;

use notify::{Event, RecursiveMode, Watcher};
use tracing::{error, info};

pub struct ConfigWatcher {
    _watcher: notify::RecommendedWatcher,
    rx: mpsc::Receiver<Result<Event, notify::Error>>,
}

impl ConfigWatcher {
    pub fn new(path: &str) -> Result<Self, String> {
        let (tx, rx) = mpsc::channel();

        let mut watcher = notify::RecommendedWatcher::new(
            move |res: Result<Event, notify::Error>| {
                let _ = tx.send(res);
            },
            notify::Config::default().with_poll_interval(Duration::from_secs(1)),
        )
        .map_err(|e| format!("failed to create file watcher: {}", e))?;

        let dir = Path::new(path);
        if !dir.is_dir() {
            return Err(format!("config path '{}' is not a directory", dir.display()));
        }

        watcher
            .watch(dir, RecursiveMode::NonRecursive)
            .map_err(|e| format!("failed to watch config directory: {}", e))?;

        info!("Watching config directory: {}", dir.display());

        Ok(Self {
            _watcher: watcher,
            rx,
        })
    }

    pub fn wait_for_change(&mut self) -> bool {
        match self.rx.recv() {
            Ok(Ok(event)) => {
                let is_toml_change = event.paths.iter().any(|p| {
                    p.extension().is_some_and(|ext| ext == "toml")
                });
                if !is_toml_change {
                    return false;
                }
                use notify::EventKind;
                match event.kind {
                    EventKind::Modify(_) | EventKind::Create(_) | EventKind::Remove(_) => {
                        info!("Config file change detected: {:?}", event.kind);
                        true
                    }
                    _ => false,
                }
            }
            Ok(Err(e)) => {
                error!("File watcher error: {}", e);
                false
            }
            Err(_) => {
                error!("File watcher channel disconnected");
                false
            }
        }
    }
}