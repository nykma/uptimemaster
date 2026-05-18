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

        let path = Path::new(path);
        let parent = path
            .parent()
            .ok_or_else(|| format!("config path '{}' has no parent directory", path.display()))?;

        watcher
            .watch(parent, RecursiveMode::NonRecursive)
            .map_err(|e| format!("failed to watch config directory: {}", e))?;

        info!("Watching config file: {}", path.display());

        Ok(Self {
            _watcher: watcher,
            rx,
        })
    }

    /// Blocks until a config change event is detected. Returns true if a reload should happen.
    pub fn wait_for_change(&mut self) -> bool {
        match self.rx.recv() {
            Ok(Ok(event)) => {
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