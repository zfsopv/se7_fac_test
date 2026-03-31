use crate::network;
use crate::ssh_ops;
use crate::types::*;
use serde_json::json;
use std::net::Ipv4Addr;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use tiny_http::{Header, Method, Request, Response, Server};

const INDEX_HTML: &str = include_str!("web_ui.html");

pub struct SharedState {
    pub interfaces: Vec<NetworkInterface>,
    pub selected_iface_idx: usize,
    pub scan_start: String,
    pub scan_end: String,
    pub test_program: String,
    pub duration: AgingDuration,
    pub upload_buf_kb: u32,
    pub devices: Vec<DeviceInfo>,
    pub log_lines: Vec<String>,
    pub scanning: bool,
    pub testing: bool,
    pub show_confirm: bool,
}

impl SharedState {
    pub fn new() -> Self {
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

        Self {
            interfaces,
            selected_iface_idx: 0,
            scan_start,
            scan_end,
            test_program,
            duration: AgingDuration::Min5,
            upload_buf_kb: 64,
            devices: Vec::new(),
            log_lines: Vec::new(),
            scanning: false,
            testing: false,
            show_confirm: false,
        }
    }

    fn add_log(&mut self, msg: &str) {
        let ts = chrono::Local::now().format("%H:%M:%S");
        self.log_lines.push(format!("[{}] {}", ts, msg));
    }

    fn process_message(&mut self, msg: WorkerMsg) {
        match msg {
            WorkerMsg::Log(text) => {
                let ts = chrono::Local::now().format("%H:%M:%S");
                self.log_lines.push(format!("[{}] {}", ts, text));
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

    fn to_json(&self) -> serde_json::Value {
        json!({
            "interfaces": self.interfaces.iter().map(|i| json!({
                "name": i.name,
                "ip": i.ip.to_string(),
                "display": i.to_string(),
            })).collect::<Vec<_>>(),
            "selected_iface_idx": self.selected_iface_idx,
            "scan_start": self.scan_start,
            "scan_end": self.scan_end,
            "test_program": self.test_program,
            "duration": format!("{}", self.duration),
            "duration_value": self.duration.to_value(),
            "upload_buf_kb": self.upload_buf_kb,
            "devices": self.devices.iter().enumerate().map(|(i, d)| json!({
                "idx": i,
                "sn": d.sn,
                "ip": d.ip,
                "mac": d.mac,
                "username": d.username,
                "password": d.password,
                "state_label": d.state.label(),
                "state_color": state_color_css(&d.state),
                "selected": d.selected,
            })).collect::<Vec<_>>(),
            "log_lines": self.log_lines,
            "scanning": self.scanning,
            "testing": self.testing,
            "show_confirm": self.show_confirm,
        })
    }
}

fn state_color_css(state: &DeviceState) -> &'static str {
    match state {
        DeviceState::Passed => "#16a34a",
        DeviceState::Failed => "#dc2626",
        DeviceState::Error(_) => "#ea580c",
        DeviceState::Testing(_) => "#d97706",
        DeviceState::Transferring(_) => "#1a73e8",
        DeviceState::Extracting => "#7c3aed",
        DeviceState::Discovered => "#666666",
    }
}

pub fn run_server(
    state: Arc<Mutex<SharedState>>,
    msg_tx: Sender<WorkerMsg>,
    msg_rx: Receiver<WorkerMsg>,
) {
    let state_msg = state.clone();
    std::thread::spawn(move || {
        for msg in msg_rx {
            let mut s = state_msg.lock().unwrap();
            s.process_message(msg);
        }
    });

    let preferred = std::env::args()
        .nth(1)
        .and_then(|s| s.parse::<u16>().ok());

    let (server, port) = if let Some(p) = preferred {
        let addr = format!("0.0.0.0:{}", p);
        match Server::http(&addr) {
            Ok(s) => (s, p),
            Err(e) => {
                eprintln!("无法监听指定端口 {}: {}", p, e);
                std::process::exit(1);
            }
        }
    } else {
        let mut result = None;
        for p in 8000..=9000 {
            match Server::http(&format!("0.0.0.0:{}", p)) {
                Ok(s) => {
                    result = Some((s, p));
                    break;
                }
                Err(_) => continue,
            }
        }
        match result {
            Some(r) => r,
            None => {
                eprintln!("无法在 8000-9000 端口范围内找到可用端口");
                std::process::exit(1);
            }
        }
    };

    let interfaces = network::get_interfaces();

    println!("========================================");
    println!("  SE7局域网批量老化程序 (Web UI)");
    println!("========================================");
    println!("  监听端口: {}", port);
    println!("  可通过以下地址访问:");
    println!("    http://localhost:{}", port);
    for iface in &interfaces {
        println!("    http://{}:{}", iface.ip, port);
    }
    println!("  按 Ctrl+C 退出");
    println!("========================================");

    open_browser(&format!("http://localhost:{}", port));

    for request in server.incoming_requests() {
        let state = state.clone();
        let msg_tx = msg_tx.clone();
        std::thread::spawn(move || {
            handle_request(request, &state, &msg_tx);
        });
    }
}

fn handle_request(
    mut request: Request,
    state: &Arc<Mutex<SharedState>>,
    msg_tx: &Sender<WorkerMsg>,
) {
    let url = request.url().to_string();
    let method = request.method().clone();

    let body = {
        let mut buf = String::new();
        request.as_reader().read_to_string(&mut buf).ok();
        buf
    };

    let response = route(&url, &method, &body, state, msg_tx);
    request.respond(response).ok();
}

fn route(
    url: &str,
    method: &Method,
    body: &str,
    state: &Arc<Mutex<SharedState>>,
    msg_tx: &Sender<WorkerMsg>,
) -> Response<std::io::Cursor<Vec<u8>>> {
    match (method, url) {
        (&Method::Get, "/") => html_response(INDEX_HTML),

        (&Method::Get, "/api/state") => {
            let s = state.lock().unwrap();
            json_response(&s.to_json().to_string())
        }

        (&Method::Post, "/api/set-interface") => {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(body) {
                if let Some(idx) = v["idx"].as_u64() {
                    let mut s = state.lock().unwrap();
                    let idx = idx as usize;
                    if idx < s.interfaces.len() {
                        s.selected_iface_idx = idx;
                        let (start, end) =
                            network::compute_scan_range(&s.interfaces[idx]);
                        s.scan_start = start;
                        s.scan_end = end;
                    }
                }
            }
            ok_response()
        }

        (&Method::Post, "/api/scan") => {
            let v = serde_json::from_str::<serde_json::Value>(body).unwrap_or(json!({}));
            let mut s = state.lock().unwrap();

            if s.scanning || s.testing {
                return error_response("正在扫描或测试中");
            }
            if s.interfaces.is_empty() {
                return error_response("无可用网络接口");
            }
            if let Some(start) = v["scan_start"].as_str() {
                s.scan_start = start.to_string();
            }
            if let Some(end) = v["scan_end"].as_str() {
                s.scan_end = end.to_string();
            }

            let start: Ipv4Addr = match s.scan_start.parse() {
                Ok(ip) => ip,
                Err(_) => {
                    s.add_log("错误: 无效的起始 IP 地址");
                    return error_response("无效的起始 IP 地址");
                }
            };
            let end: Ipv4Addr = match s.scan_end.parse() {
                Ok(ip) => ip,
                Err(_) => {
                    s.add_log("错误: 无效的结束 IP 地址");
                    return error_response("无效的结束 IP 地址");
                }
            };

            s.devices.clear();
            s.scanning = true;

            let tx = msg_tx.clone();
            std::thread::spawn(move || {
                network::scan_network(start, end, tx);
            });

            ok_response()
        }

        (&Method::Post, "/api/test") => {
            let v = serde_json::from_str::<serde_json::Value>(body).unwrap_or(json!({}));
            let mut s = state.lock().unwrap();

            if let Some(program) = v["test_program"].as_str() {
                s.test_program = program.to_string();
            }
            if let Some(dur) = v["duration"].as_str() {
                if let Some(d) = AgingDuration::from_value(dur) {
                    s.duration = d;
                }
            }
            if let Some(bk) = v["upload_buf_kb"].as_u64() {
                let bk = bk as u32;
                if bk >= 1 && bk <= 1024 {
                    s.upload_buf_kb = bk;
                }
            }

            if s.test_program.is_empty() {
                s.add_log("错误: 请先设置产测程序路径");
                return error_response("请先设置产测程序路径");
            }
            if !std::path::Path::new(&s.test_program).exists() {
                s.add_log("错误: 产测程序文件不存在");
                return error_response("产测程序文件不存在");
            }

            let selected_count = s.devices.iter().filter(|d| d.selected).count();
            if selected_count == 0 {
                s.add_log("错误: 未选择任何设备");
                return error_response("未选择任何设备");
            }

            s.testing = true;

            let exe_dir = std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(|p| p.to_path_buf()))
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
            let log_dir = exe_dir.join("logs");
            std::fs::create_dir_all(&log_dir).ok();

            s.add_log(&format!("开始测试 {} 台设备...", selected_count));

            let devices_to_test: Vec<_> =
                s.devices.iter().filter(|d| d.selected).cloned().collect();
            let program = s.test_program.clone();
            let duration = s.duration;
            let buf_size = s.upload_buf_kb as usize * 1024;

            for device in devices_to_test {
                let tx = msg_tx.clone();
                let program = program.clone();
                let log_dir = log_dir.clone();
                std::thread::spawn(move || {
                    ssh_ops::device_workflow(device, &program, duration, buf_size, tx, &log_dir);
                });
            }

            ok_response()
        }

        (&Method::Post, "/api/toggle-device") => {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(body) {
                if let Some(idx) = v["idx"].as_u64() {
                    let mut s = state.lock().unwrap();
                    let idx = idx as usize;
                    if idx < s.devices.len() && !s.testing {
                        s.devices[idx].selected = !s.devices[idx].selected;
                    }
                }
            }
            ok_response()
        }

        (&Method::Post, "/api/select-all") => {
            let mut s = state.lock().unwrap();
            if !s.testing {
                for d in &mut s.devices {
                    d.selected = true;
                }
            }
            ok_response()
        }

        (&Method::Post, "/api/deselect-all") => {
            let mut s = state.lock().unwrap();
            if !s.testing {
                for d in &mut s.devices {
                    d.selected = false;
                }
            }
            ok_response()
        }

        (&Method::Post, "/api/update-settings") => {
            let v = serde_json::from_str::<serde_json::Value>(body).unwrap_or(json!({}));
            let mut s = state.lock().unwrap();
            if let Some(p) = v["test_program"].as_str() {
                s.test_program = p.to_string();
            }
            if let Some(start) = v["scan_start"].as_str() {
                s.scan_start = start.to_string();
            }
            if let Some(end) = v["scan_end"].as_str() {
                s.scan_end = end.to_string();
            }
            if let Some(dur) = v["duration"].as_str() {
                if let Some(d) = AgingDuration::from_value(dur) {
                    s.duration = d;
                }
            }
            if let Some(bk) = v["upload_buf_kb"].as_u64() {
                let bk = bk as u32;
                if bk >= 1 && bk <= 1024 {
                    s.upload_buf_kb = bk;
                }
            }
            ok_response()
        }

        (&Method::Post, "/api/confirm") => {
            let mut s = state.lock().unwrap();
            s.show_confirm = false;
            ok_response()
        }

        _ => {
            Response::from_string("Not Found")
                .with_status_code(404)
                .with_header(content_type_header("text/plain; charset=utf-8"))
        }
    }
}

fn content_type_header(ct: &str) -> Header {
    Header::from_bytes(&b"Content-Type"[..], ct.as_bytes()).unwrap()
}

fn html_response(html: &str) -> Response<std::io::Cursor<Vec<u8>>> {
    Response::from_string(html).with_header(content_type_header("text/html; charset=utf-8"))
}

fn json_response(json: &str) -> Response<std::io::Cursor<Vec<u8>>> {
    Response::from_string(json).with_header(content_type_header("application/json"))
}

fn ok_response() -> Response<std::io::Cursor<Vec<u8>>> {
    json_response(r#"{"ok":true}"#)
}

fn error_response(msg: &str) -> Response<std::io::Cursor<Vec<u8>>> {
    json_response(&json!({"ok": false, "error": msg}).to_string())
}

fn open_browser(url: &str) {
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(url)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .ok();
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(&["/c", "start", url])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .ok();
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(url)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .ok();
    }
}
