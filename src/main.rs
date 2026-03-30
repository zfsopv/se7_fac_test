mod app;
mod network;
mod ssh_ops;
mod types;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([950.0, 720.0])
            .with_min_inner_size([700.0, 500.0]),
        ..Default::default()
    };

    eframe::run_native(
        "SE7局域网批量老化程序",
        options,
        Box::new(|cc| Ok(Box::new(app::FacTestApp::new(cc)))),
    )
}
