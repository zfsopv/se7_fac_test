mod app;
mod network;
mod ssh_ops;
mod types;

use std::sync::{Arc, Mutex};

fn main() {
    let state = Arc::new(Mutex::new(app::SharedState::new()));
    let (msg_tx, msg_rx) = std::sync::mpsc::channel();
    app::run_server(state, msg_tx, msg_rx);
}
