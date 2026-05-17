#![cfg(unix)]

pub(crate) struct ShutdownSignal {
    sigterm: Option<tokio::signal::unix::Signal>,
    sigint: Option<tokio::signal::unix::Signal>,
}

impl ShutdownSignal {
    pub(crate) fn arm() -> Self {
        let sigterm =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).ok();
        let sigint = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt()).ok();
        Self { sigterm, sigint }
    }

    pub(crate) async fn recv(&mut self) {
        match (&mut self.sigterm, &mut self.sigint) {
            (Some(sigterm), Some(sigint)) => {
                tokio::select! {
                    _ = sigterm.recv() => {}
                    _ = sigint.recv() => {}
                }
            }
            (Some(sigterm), None) => {
                let _ = sigterm.recv().await;
            }
            (None, Some(sigint)) => {
                let _ = sigint.recv().await;
            }
            (None, None) => std::future::pending::<()>().await,
        }
    }
}
