use crate::types::*;
use socket2::{SockRef, TcpKeepalive};
use std::io::{BufReader, Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::time::Duration;

const MAX_UPLOAD_RETRIES: u32 = 7;

pub fn device_workflow(
    device: DeviceInfo,
    program_path: &str,
    duration: AgingDuration,
    tx: Sender<WorkerMsg>,
    log_dir: &Path,
) {
    let ip = device.ip.clone();
    let sn = device.sn.clone();
    let mut flog = FileLogger::new(&log_dir.join(format!("{}_{}.log", ip, sn)));

    // --- 连接 ---
    let sess = match ssh_connect(&ip, &device.username, &device.password) {
        Ok(s) => s,
        Err(e) => {
            let msg = format!("SSH 连接失败: {}", e);
            flog.log(&msg);
            send_log(&tx, &ip, &msg);
            send_state(&tx, &ip, DeviceState::Error(msg));
            return;
        }
    };
    send_log(&tx, &ip, "SSH 连接成功");
    flog.log("SSH 连接成功");

    // --- 创建 /data 目录 ---
    if let Err(e) = ssh_exec(&sess, "mkdir -p /data") {
        let msg = format!("创建 /data 目录失败: {}", e);
        flog.log(&msg);
        send_state(&tx, &ip, DeviceState::Error(msg));
        return;
    }

    // --- 上传产测程序 ---
    let file_name = Path::new(program_path)
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let remote_path = format!("/data/{}", file_name);

    send_state(&tx, &ip, DeviceState::Transferring(0.0));

    let mut uploaded = false;
    for attempt in 1..=MAX_UPLOAD_RETRIES {
        send_log(&tx, &ip, &format!("开始上传文件 (第{}次)", attempt));
        flog.log(&format!("开始上传文件 (第{}次尝试)", attempt));

        let upload_sess = ssh_connect(&ip, &device.username, &device.password);

        let upload_sess = match upload_sess {
            Ok(s) => s,
            Err(e) => {
                flog.log(&format!("上传重连失败: {}", e));
                if attempt < MAX_UPLOAD_RETRIES {
                    std::thread::sleep(Duration::from_secs(5 * attempt as u64));
                }
                continue;
            }
        };

        match sftp_upload(&upload_sess, program_path, &remote_path, &tx, &ip) {
            Ok(()) => {
                send_log(&tx, &ip, "文件上传完成");
                flog.log("文件上传完成");
                uploaded = true;
                break;
            }
            Err(e) => {
                let msg = format!("上传失败: {}", e);
                flog.log(&msg);
                send_log(&tx, &ip, &msg);
                if attempt < MAX_UPLOAD_RETRIES {
                    std::thread::sleep(Duration::from_secs(5 * attempt as u64));
                }
            }
        }
    }

    if !uploaded {
        let msg = format!("文件上传失败, 已重试{}次", MAX_UPLOAD_RETRIES);
        flog.log(&msg);
        send_state(&tx, &ip, DeviceState::Error(msg));
        return;
    }

    // --- 解压 (后台执行 + 轮询) ---
    send_state(&tx, &ip, DeviceState::Extracting);
    send_log(&tx, &ip, "开始解压文件...");
    flog.log("开始解压文件");

    let extract_sess = match ssh_connect(&ip, &device.username, &device.password) {
        Ok(s) => s,
        Err(e) => {
            let msg = format!("解压阶段 SSH 连接失败: {}", e);
            flog.log(&msg);
            send_state(&tx, &ip, DeviceState::Error(msg));
            return;
        }
    };

    let extract_cmd = format!(
        concat!(
            "rm -f /tmp/tar_done /tmp/tar.log; ",
            "nohup sh -c '",
            ". /etc/profile 2>/dev/null; ",
            "cd /data && tar -xaf {} > /tmp/tar.log 2>&1; ",
            "echo $? > /tmp/tar_done",
            "' </dev/null >/dev/null 2>&1 & echo BG_OK"
        ),
        file_name
    );
    match ssh_exec(&extract_sess, &extract_cmd) {
        Ok(output) => {
            if !output.contains("BG_OK") {
                let msg = "启动后台解压失败".to_string();
                flog.log(&msg);
                send_state(&tx, &ip, DeviceState::Error(msg));
                return;
            }
        }
        Err(e) => {
            let msg = format!("启动解压命令失败: {}", e);
            flog.log(&msg);
            send_state(&tx, &ip, DeviceState::Error(msg));
            return;
        }
    }
    drop(extract_sess);

    let extract_timeout = Duration::from_secs(600);
    let extract_start = std::time::Instant::now();
    let mut extract_ok = false;

    loop {
        std::thread::sleep(Duration::from_secs(3));

        if extract_start.elapsed() > extract_timeout {
            let msg = "解压超时 (超过10分钟)".to_string();
            flog.log(&msg);
            send_state(&tx, &ip, DeviceState::Error(msg));
            return;
        }

        let poll_sess = match ssh_connect(&ip, &device.username, &device.password) {
            Ok(s) => s,
            Err(_) => continue,
        };

        match ssh_exec(&poll_sess, "cat /tmp/tar_done 2>/dev/null") {
            Ok(output) => {
                let code = output.trim();
                if !code.is_empty() {
                    if code == "0" {
                        send_log(&tx, &ip, "解压完成");
                        flog.log("解压完成");
                        extract_ok = true;
                    } else {
                        let tar_log =
                            ssh_exec(&poll_sess, "tail -20 /tmp/tar.log 2>/dev/null")
                                .unwrap_or_default();
                        flog.log(&format!("解压失败 (exit={}): {}", code, tar_log));
                        send_log(&tx, &ip, &format!("解压失败 (exit={})", code));
                    }
                    break;
                }
            }
            Err(_) => {}
        }
    }

    if !extract_ok {
        send_state(&tx, &ip, DeviceState::Error("文件解压失败".to_string()));
        return;
    }

    // 验证解压结果
    let verify_sess = match ssh_connect(&ip, &device.username, &device.password) {
        Ok(s) => s,
        Err(e) => {
            let msg = format!("验证阶段 SSH 连接失败: {}", e);
            flog.log(&msg);
            send_state(&tx, &ip, DeviceState::Error(msg));
            return;
        }
    };
    match ssh_exec_with_profile(
        &verify_sess,
        "test -x /data/bm1684x_soc_aging/bm1684x_soc_aging && echo OK || echo MISSING",
    ) {
        Ok(output) => {
            if output.trim() != "OK" {
                let msg = "解压后未找到老化测试程序".to_string();
                flog.log(&msg);
                send_state(&tx, &ip, DeviceState::Error(msg));
                return;
            }
        }
        Err(e) => {
            let msg = format!("验证测试程序失败: {}", e);
            flog.log(&msg);
            send_state(&tx, &ip, DeviceState::Error(msg));
            return;
        }
    }

    // --- 启动老化测试 (后台执行) ---
    send_state(&tx, &ip, DeviceState::Testing(std::time::Instant::now()));
    let minutes = duration.minutes();
    send_log(&tx, &ip, &format!("启动老化测试 ({}分钟)...", minutes));
    flog.log(&format!("启动老化测试, 时长: {}分钟", minutes));

    let start_sess = match ssh_connect(&ip, &device.username, &device.password) {
        Ok(s) => s,
        Err(e) => {
            let msg = format!("启动测试 SSH 连接失败: {}", e);
            flog.log(&msg);
            send_state(&tx, &ip, DeviceState::Error(msg));
            return;
        }
    };

    let test_cmd = format!(
        concat!(
            "rm -f /data/aging_done; ",
            "nohup sh -c '",
            ". /etc/profile 2>/dev/null; ",
            "cd /data/bm1684x_soc_aging && ",
            "./bm1684x_soc_aging --restart_all --run_time {} ",
            ">/data/aging_output.log 2>&1; ",
            "echo $? > /data/aging_done",
            "' </dev/null >/dev/null 2>&1 & echo BG_OK"
        ),
        minutes
    );

    flog.log(&format!("执行命令: {}", test_cmd));
    match ssh_exec(&start_sess, &test_cmd) {
        Ok(output) => {
            if !output.contains("BG_OK") {
                let msg = "启动后台测试进程失败".to_string();
                flog.log(&msg);
                send_state(&tx, &ip, DeviceState::Error(msg));
                return;
            }
        }
        Err(e) => {
            let msg = format!("启动测试失败: {}", e);
            flog.log(&msg);
            send_state(&tx, &ip, DeviceState::Error(msg));
            return;
        }
    }
    drop(start_sess);

    std::thread::sleep(Duration::from_secs(2));
    let check_sess = match ssh_connect(&ip, &device.username, &device.password) {
        Ok(s) => s,
        Err(e) => {
            flog.log(&format!("验证进程启动连接失败: {}", e));
            send_log(&tx, &ip, "测试进程已提交, 无法确认启动状态");
            // 继续轮询，不中断
            ssh_connect(&ip, &device.username, &device.password).ok();
            ssh2::Session::new().unwrap()
        }
    };
    match ssh_exec(&check_sess, "pgrep -f 'bm1684x_soc_aging --restart_all'") {
        Ok(output) if !output.trim().is_empty() => {
            flog.log(&format!("测试进程 PID: {}", output.trim()));
        }
        _ => {
            flog.log("警告: 未检测到测试进程, 可能启动延迟");
            send_log(&tx, &ip, "警告: 未检测到测试进程");
        }
    }
    drop(check_sess);

    // --- 轮询等待完成 (通过标记文件) ---
    let poll_interval = Duration::from_secs(30);
    let max_wait = Duration::from_secs((minutes as u64 + 15) * 60);
    let start_time = std::time::Instant::now();

    loop {
        std::thread::sleep(poll_interval);

        if start_time.elapsed() > max_wait {
            let msg = format!(
                "测试超时 (已等待{}分钟, 预期{}分钟)",
                start_time.elapsed().as_secs() / 60,
                minutes
            );
            flog.log(&msg);
            send_state(&tx, &ip, DeviceState::Error(msg));
            return;
        }

        let poll_sess = match ssh_connect(&ip, &device.username, &device.password) {
            Ok(s) => s,
            Err(e) => {
                flog.log(&format!("轮询重连失败: {}, 继续等待...", e));
                continue;
            }
        };
        poll_sess.set_timeout(30_000);

        match ssh_exec(&poll_sess, "cat /data/aging_done 2>/dev/null") {
            Ok(output) => {
                let code = output.trim();
                if code.is_empty() {
                    continue;
                }
                flog.log(&format!("测试进程已结束 (exit={}), 读取结果...", code));
                send_log(&tx, &ip, "测试完成, 检查结果...");

                let result_sess = match ssh_connect(&ip, &device.username, &device.password) {
                    Ok(s) => s,
                    Err(e) => {
                        let msg = format!("读取结果连接失败: {}", e);
                        flog.log(&msg);
                        send_state(&tx, &ip, DeviceState::Error(msg));
                        return;
                    }
                };

                match ssh_exec(
                    &result_sess,
                    "grep -o 'QA_AGING_PASS\\|QA_AGING_FAILED\\|QA_AGING_FAIL' /data/aging_output.log | tail -1",
                ) {
                    Ok(marker) => {
                        let marker = marker.trim();
                        if marker.contains("QA_AGING_PASS") {
                            send_log(&tx, &ip, "老化测试通过!");
                            send_state(&tx, &ip, DeviceState::Passed);
                        } else if marker.contains("QA_AGING_FAILED") || marker.contains("QA_AGING_FAIL") {
                            send_log(&tx, &ip, "老化测试失败!");
                            send_state(&tx, &ip, DeviceState::Failed);
                        } else {
                            let tail = ssh_exec(
                                &result_sess,
                                "tail -30 /data/aging_output.log 2>/dev/null",
                            )
                            .unwrap_or_default();
                            flog.log(&format!("日志尾部:\n{}", tail));
                            let msg = "未检测到 QA_AGING_PASS/FAILED/FAIL 标识".to_string();
                            send_log(&tx, &ip, &msg);
                            send_state(&tx, &ip, DeviceState::Error(msg));
                        }
                    }
                    Err(e) => {
                        let msg = format!("读取测试日志失败: {}", e);
                        flog.log(&msg);
                        send_state(&tx, &ip, DeviceState::Error(msg));
                    }
                }
                return;
            }
            Err(e) => {
                flog.log(&format!("轮询检查失败: {}", e));
            }
        }
    }
}

fn ssh_connect(ip: &str, user: &str, pass: &str) -> Result<ssh2::Session, String> {
    let addr = SocketAddr::new(
        ip.parse::<IpAddr>().map_err(|e| e.to_string())?,
        22,
    );

    let mut last_err = String::new();
    for attempt in 0..3 {
        if attempt > 0 {
            std::thread::sleep(Duration::from_secs(2 * attempt as u64));
        }

        let tcp = match TcpStream::connect_timeout(&addr, Duration::from_secs(10)) {
            Ok(t) => t,
            Err(e) => {
                last_err = format!("TCP 连接失败: {}", e);
                continue;
            }
        };
        tcp.set_nodelay(true).ok();
        tcp.set_read_timeout(Some(Duration::from_secs(600))).ok();
        tcp.set_write_timeout(Some(Duration::from_secs(600))).ok();
        let sock_ref = SockRef::from(&tcp);
        let keepalive = TcpKeepalive::new()
            .with_time(Duration::from_secs(15))
            .with_interval(Duration::from_secs(15));
        sock_ref.set_tcp_keepalive(&keepalive).ok();

        let mut sess = match ssh2::Session::new() {
            Ok(s) => s,
            Err(e) => {
                last_err = format!("创建会话失败: {}", e);
                continue;
            }
        };
        sess.set_tcp_stream(tcp);
        sess.set_timeout(0);
        sess.set_blocking(true);

        if let Err(e) = sess.handshake() {
            last_err = format!("SSH 握手失败(第{}次): {}", attempt + 1, e);
            continue;
        }

        // 握手成功后设置 keepalive 保持连接活跃，不设超时
        sess.set_keepalive(true, 10);

        if let Err(e) = sess.userauth_password(user, pass) {
            return Err(format!("认证失败: {}", e));
        }

        if !sess.authenticated() {
            return Err("SSH 认证失败".into());
        }

        return Ok(sess);
    }

    Err(last_err)
}

fn ssh_exec_with_profile(sess: &ssh2::Session, cmd: &str) -> Result<String, String> {
    let wrapped = format!("source /etc/profile 2>/dev/null; {}", cmd);
    ssh_exec(sess, &wrapped)
}

fn ssh_exec(sess: &ssh2::Session, cmd: &str) -> Result<String, String> {
    let mut channel = sess.channel_session().map_err(|e| e.to_string())?;
    channel.exec(cmd).map_err(|e| e.to_string())?;

    let mut stdout = String::new();
    channel
        .read_to_string(&mut stdout)
        .map_err(|e| e.to_string())?;

    let mut stderr_str = String::new();
    channel
        .stderr()
        .read_to_string(&mut stderr_str)
        .map_err(|e| e.to_string())?;

    channel.wait_close().ok();
    let exit_code = channel.exit_status().unwrap_or(-1);

    if exit_code != 0 && !stderr_str.is_empty() {
        stdout.push_str(&format!("\n[stderr exit={}] {}", exit_code, stderr_str));
    }

    Ok(stdout)
}

fn sftp_upload(
    sess: &ssh2::Session,
    local_path: &str,
    remote_path: &str,
    tx: &Sender<WorkerMsg>,
    ip: &str,
) -> Result<(), String> {
    sess.set_timeout(0);

    let sftp = sess.sftp().map_err(|e| format!("SFTP 初始化失败: {}", e))?;

    let local_file = std::fs::File::open(local_path)
        .map_err(|e| format!("打开本地文件失败: {}", e))?;
    let file_size = local_file
        .metadata()
        .map_err(|e| format!("获取文件信息失败: {}", e))?
        .len();

    let mut remote_file = sftp
        .create(Path::new(remote_path))
        .map_err(|e| format!("创建远程文件失败: {}", e))?;

    let mut reader = BufReader::new(local_file);
    let mut buf = vec![0u8; 256 * 1024];
    let mut transferred = 0u64;
    let mut last_progress = -1.0f32;

    loop {
        let n = reader
            .read(&mut buf)
            .map_err(|e| format!("读取本地文件失败: {}", e))?;
        if n == 0 {
            break;
        }

        remote_file
            .write_all(&buf[..n])
            .map_err(|e| format!("写入远程文件失败: {}", e))?;
        transferred += n as u64;

        let progress = if file_size > 0 {
            transferred as f32 / file_size as f32
        } else {
            1.0
        };

        if progress - last_progress > 0.02 {
            last_progress = progress;
            tx.send(WorkerMsg::DeviceStateChanged {
                ip: ip.to_string(),
                state: DeviceState::Transferring(progress),
            })
            .ok();
        }
    }

    Ok(())
}

fn send_log(tx: &Sender<WorkerMsg>, ip: &str, msg: &str) {
    tx.send(WorkerMsg::Log(format!("{}: {}", ip, msg))).ok();
}

fn send_state(tx: &Sender<WorkerMsg>, ip: &str, state: DeviceState) {
    tx.send(WorkerMsg::DeviceStateChanged {
        ip: ip.to_string(),
        state,
    })
    .ok();
}

struct FileLogger {
    path: PathBuf,
}

impl FileLogger {
    fn new(path: &Path) -> Self {
        Self {
            path: path.to_path_buf(),
        }
    }

    fn log(&mut self, msg: &str) {
        let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
        {
            writeln!(f, "[{}] {}", timestamp, msg).ok();
        }
    }
}
