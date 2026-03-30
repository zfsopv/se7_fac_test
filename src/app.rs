use crate::network;
use crate::ssh_ops;
use crate::types::*;
use eframe::egui;
use egui_extras::Column;
use std::net::Ipv4Addr;
use std::sync::mpsc::{Receiver, Sender};

pub struct FacTestApp {
    interfaces: Vec<NetworkInterface>,
    selected_iface_idx: usize,
    scan_start: String,
    scan_end: String,
    test_program: String,
    duration: AgingDuration,

    devices: Vec<DeviceInfo>,
    log_lines: Vec<String>,

    scanning: bool,
    testing: bool,
    show_confirm: bool,
    log_scroll_to_bottom: bool,

    msg_tx: Sender<WorkerMsg>,
    msg_rx: Receiver<WorkerMsg>,
}

impl FacTestApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        setup_fonts(&cc.egui_ctx);

        let interfaces = network::get_interfaces();
        let (scan_start, scan_end) = if !interfaces.is_empty() {
            network::compute_scan_range(&interfaces[0])
        } else {
            (String::new(), String::new())
        };

        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.to_path_buf()))
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());

        let default_program = exe_dir.join("bm1684x_soc_aging_v3_0_0.tgz");
        let test_program = if default_program.exists() {
            default_program.to_string_lossy().to_string()
        } else {
            String::new()
        };

        let (msg_tx, msg_rx) = std::sync::mpsc::channel();

        Self {
            interfaces,
            selected_iface_idx: 0,
            scan_start,
            scan_end,
            test_program,
            duration: AgingDuration::Min5,
            devices: Vec::new(),
            log_lines: Vec::new(),
            scanning: false,
            testing: false,
            show_confirm: false,
            log_scroll_to_bottom: false,
            msg_tx,
            msg_rx,
        }
    }

    fn process_messages(&mut self) {
        while let Ok(msg) = self.msg_rx.try_recv() {
            match msg {
                WorkerMsg::Log(text) => {
                    let ts = chrono::Local::now().format("%H:%M:%S");
                    self.log_lines.push(format!("[{}] {}", ts, text));
                    self.log_scroll_to_bottom = true;
                }
                WorkerMsg::DeviceFound(device) => {
                    self.devices.push(device);
                }
                WorkerMsg::ScanComplete => {
                    self.scanning = false;
                    let count = self.devices.len();
                    let ts = chrono::Local::now().format("%H:%M:%S");
                    self.log_lines
                        .push(format!("[{}] 扫描完成, 发现 {} 台设备", ts, count));
                    if count > 0 {
                        self.show_confirm = true;
                    }
                }
                WorkerMsg::DeviceStateChanged { ip, state } => {
                    if let Some(dev) = self.devices.iter_mut().find(|d| d.ip == ip) {
                        dev.state = state;
                    }
                    self.check_all_tests_done();
                }
            }
        }
    }

    fn check_all_tests_done(&mut self) {
        if !self.testing {
            return;
        }
        let all_done = self
            .devices
            .iter()
            .filter(|d| d.selected)
            .all(|d| {
                matches!(
                    d.state,
                    DeviceState::Passed | DeviceState::Failed | DeviceState::Error(_)
                )
            });
        if all_done {
            self.testing = false;
            let ts = chrono::Local::now().format("%H:%M:%S");
            self.log_lines
                .push(format!("[{}] 所有选中设备测试已完成", ts));
        }
    }

    fn start_scan(&mut self) {
        self.devices.clear();
        self.scanning = true;

        let start: Ipv4Addr = match self.scan_start.parse() {
            Ok(ip) => ip,
            Err(_) => {
                self.add_log("错误: 无效的起始 IP 地址");
                self.scanning = false;
                return;
            }
        };
        let end: Ipv4Addr = match self.scan_end.parse() {
            Ok(ip) => ip,
            Err(_) => {
                self.add_log("错误: 无效的结束 IP 地址");
                self.scanning = false;
                return;
            }
        };

        let tx = self.msg_tx.clone();
        std::thread::spawn(move || {
            network::scan_network(start, end, tx);
        });
    }

    fn start_tests(&mut self) {
        if self.test_program.is_empty() {
            self.add_log("错误: 请先选择产测程序");
            return;
        }
        if !std::path::Path::new(&self.test_program).exists() {
            self.add_log("错误: 产测程序文件不存在");
            return;
        }

        let selected_count = self.devices.iter().filter(|d| d.selected).count();
        if selected_count == 0 {
            self.add_log("错误: 未选择任何设备");
            return;
        }

        self.testing = true;

        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.to_path_buf()))
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        let log_dir = exe_dir.join("logs");
        std::fs::create_dir_all(&log_dir).ok();

        self.add_log(&format!("开始测试 {} 台设备...", selected_count));

        for device in &self.devices {
            if !device.selected {
                continue;
            }
            let dev = device.clone();
            let tx = self.msg_tx.clone();
            let program = self.test_program.clone();
            let log_dir = log_dir.clone();
            let duration = self.duration;

            std::thread::spawn(move || {
                ssh_ops::device_workflow(dev, &program, duration, tx, &log_dir);
            });
        }
    }

    fn add_log(&mut self, msg: &str) {
        let ts = chrono::Local::now().format("%H:%M:%S");
        self.log_lines.push(format!("[{}] {}", ts, msg));
        self.log_scroll_to_bottom = true;
    }

    fn render_settings(&mut self, ui: &mut egui::Ui) {
        egui::Grid::new("settings_grid")
            .num_columns(2)
            .spacing([12.0, 8.0])
            .show(ui, |ui| {
                ui.label("网络接口:");
                let iface_text = if self.interfaces.is_empty() {
                    "无可用接口".to_string()
                } else {
                    self.interfaces[self.selected_iface_idx].to_string()
                };
                egui::ComboBox::from_id_salt("iface_combo")
                    .selected_text(&iface_text)
                    .width(280.0)
                    .show_ui(ui, |ui| {
                        let mut changed = false;
                        for (i, iface) in self.interfaces.iter().enumerate() {
                            if ui
                                .selectable_value(
                                    &mut self.selected_iface_idx,
                                    i,
                                    iface.to_string(),
                                )
                                .changed()
                            {
                                changed = true;
                            }
                        }
                        if changed && !self.interfaces.is_empty() {
                            let (s, e) = network::compute_scan_range(
                                &self.interfaces[self.selected_iface_idx],
                            );
                            self.scan_start = s;
                            self.scan_end = e;
                        }
                    });
                ui.end_row();

                ui.label("扫描范围:");
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.scan_start).desired_width(130.0),
                    );
                    ui.label(" 至 ");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.scan_end).desired_width(130.0),
                    );
                });
                ui.end_row();

                ui.label("产测程序:");
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.test_program).desired_width(320.0),
                    );
                    if ui.button("浏览...").clicked() {
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("压缩包", &["tgz", "gz"])
                            .pick_file()
                        {
                            self.test_program = path.to_string_lossy().to_string();
                        }
                    }
                });
                ui.end_row();

                ui.label("老化时间:");
                egui::ComboBox::from_id_salt("duration_combo")
                    .selected_text(self.duration.to_string())
                    .width(120.0)
                    .show_ui(ui, |ui| {
                        for &d in AgingDuration::ALL {
                            ui.selectable_value(&mut self.duration, d, d.to_string());
                        }
                    });
                ui.end_row();
            });
    }

    fn render_buttons(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let can_scan = !self.scanning && !self.testing && !self.interfaces.is_empty();
            if ui
                .add_enabled(can_scan, egui::Button::new("开始扫描"))
                .clicked()
            {
                self.start_scan();
            }

            let has_selected = self.devices.iter().any(|d| d.selected);
            let can_test = !self.scanning && !self.testing && has_selected;
            if ui
                .add_enabled(can_test, egui::Button::new("开始测试"))
                .clicked()
            {
                self.start_tests();
            }

            ui.separator();

            if !self.devices.is_empty() && !self.testing {
                if ui.button("全选").clicked() {
                    for d in &mut self.devices {
                        d.selected = true;
                    }
                }
                if ui.button("全不选").clicked() {
                    for d in &mut self.devices {
                        d.selected = false;
                    }
                }
            }

            if self.scanning {
                ui.spinner();
                ui.label("正在扫描网络...");
            }
            if self.testing {
                ui.spinner();
                ui.label("测试进行中...");
            }
        });
    }

    fn render_confirm_dialog(&mut self, ctx: &egui::Context) {
        if !self.show_confirm {
            return;
        }

        let mut open = true;
        egui::Window::new("扫描结果")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(700.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(format!(
                    "已发现 {} 台可连接设备，请在下方列表中选择需要测试的设备后点击「开始测试」",
                    self.devices.len()
                ));
                ui.separator();

                egui::ScrollArea::vertical()
                    .max_height(350.0)
                    .show(ui, |ui| {
                        egui::Grid::new("confirm_grid")
                            .num_columns(5)
                            .spacing([12.0, 4.0])
                            .striped(true)
                            .show(ui, |ui| {
                                ui.strong("SN");
                                ui.strong("IP 地址");
                                ui.strong("MAC 地址");
                                ui.strong("用户名");
                                ui.strong("密码");
                                ui.end_row();

                                for dev in &self.devices {
                                    ui.label(&dev.sn);
                                    ui.label(&dev.ip);
                                    ui.label(&dev.mac);
                                    ui.label(&dev.username);
                                    ui.label(&dev.password);
                                    ui.end_row();
                                }
                            });
                    });

                ui.separator();
                if ui
                    .button(egui::RichText::new("确认").strong())
                    .clicked()
                {
                    self.show_confirm = false;
                }
            });

        if !open {
            self.show_confirm = false;
        }
    }

    fn render_device_table(&mut self, ui: &mut egui::Ui) {
        ui.heading("设备测试状态");
        ui.separator();

        if self.devices.is_empty() {
            ui.label("暂无设备, 请先扫描网络");
            return;
        }

        let testing = self.testing;
        let available_height = ui.available_height();
        let devices = &mut self.devices;

        egui::ScrollArea::vertical()
            .max_height(available_height)
            .show(ui, |ui| {
                egui_extras::TableBuilder::new(ui)
                    .striped(true)
                    .resizable(true)
                    .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                    .column(Column::auto().at_least(30.0))   // 选择
                    .column(Column::auto().at_least(36.0))   // 序号
                    .column(Column::auto().at_least(160.0))  // SN
                    .column(Column::auto().at_least(120.0))  // IP
                    .column(Column::auto().at_least(140.0))  // MAC
                    .column(Column::auto().at_least(110.0))  // SSH 账户
                    .column(Column::remainder().at_least(100.0)) // 状态
                    .header(22.0, |mut header| {
                        header.col(|ui| { ui.strong(""); });
                        header.col(|ui| { ui.strong("#"); });
                        header.col(|ui| { ui.strong("SN"); });
                        header.col(|ui| { ui.strong("IP 地址"); });
                        header.col(|ui| { ui.strong("MAC 地址"); });
                        header.col(|ui| { ui.strong("SSH 账户"); });
                        header.col(|ui| { ui.strong("测试状态"); });
                    })
                    .body(|mut body| {
                        for i in 0..devices.len() {
                            body.row(20.0, |mut row| {
                                let sn = devices[i].sn.clone();
                                let ip = devices[i].ip.clone();
                                let mac = devices[i].mac.clone();
                                let creds = format!(
                                    "{}:{}",
                                    devices[i].username, devices[i].password
                                );
                                let state_label = devices[i].state.label();
                                let state_color = state_color(&devices[i].state);

                                row.col(|ui| {
                                    ui.add_enabled(
                                        !testing,
                                        egui::Checkbox::new(&mut devices[i].selected, ""),
                                    );
                                });
                                row.col(|ui| {
                                    ui.label(format!("{}", i + 1));
                                });
                                row.col(|ui| {
                                    ui.label(&sn);
                                });
                                row.col(|ui| {
                                    ui.label(&ip);
                                });
                                row.col(|ui| {
                                    ui.label(&mac);
                                });
                                row.col(|ui| {
                                    ui.label(&creds);
                                });
                                row.col(|ui| {
                                    ui.colored_label(state_color, state_label);
                                });
                            });
                        }
                    });
            });
    }
    fn render_log_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("运行日志");
        ui.separator();

        let mut scroll = egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            .max_height(ui.available_height());

        if self.log_scroll_to_bottom {
            scroll = scroll.stick_to_bottom(true);
            self.log_scroll_to_bottom = false;
        }

        scroll.show(ui, |ui| {
            for line in &self.log_lines {
                ui.label(line);
            }
        });
    }
}

fn state_color(state: &DeviceState) -> egui::Color32 {
    match state {
        DeviceState::Passed => egui::Color32::from_rgb(0, 200, 0),
        DeviceState::Failed => egui::Color32::from_rgb(255, 60, 60),
        DeviceState::Error(_) => egui::Color32::from_rgb(255, 140, 0),
        DeviceState::Testing => egui::Color32::from_rgb(255, 220, 50),
        DeviceState::Transferring(_) => egui::Color32::from_rgb(100, 180, 255),
        DeviceState::Extracting => egui::Color32::from_rgb(180, 140, 255),
        DeviceState::Discovered => egui::Color32::from_rgb(200, 200, 200),
    }
}

impl eframe::App for FacTestApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.process_messages();

        if self.scanning || self.testing {
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
        }

        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            ui.add_space(4.0);
            ui.heading("SE7局域网批量老化程序");
            ui.separator();
            self.render_settings(ui);
            ui.separator();
            self.render_buttons(ui);
            ui.add_space(4.0);
        });

        egui::TopBottomPanel::bottom("bottom_panel")
            .min_height(150.0)
            .resizable(true)
            .show(ctx, |ui| {
                self.render_log_panel(ui);
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            self.render_device_table(ui);
        });

        self.render_confirm_dialog(ctx);
    }
}

const EMBEDDED_FONT: &[u8] = include_bytes!("../fonts/NotoSansSC-Regular.otf");

fn setup_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();

    fonts
        .font_data
        .insert("cjk".to_owned(), egui::FontData::from_static(EMBEDDED_FONT));

    if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
        family.insert(0, "cjk".to_owned());
    }
    if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Monospace) {
        family.push("cjk".to_owned());
    }

    ctx.set_fonts(fonts);
}
