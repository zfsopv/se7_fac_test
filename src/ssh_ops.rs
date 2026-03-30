use crate::types::*;
use std::io::{BufReader, Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::time::Duration;

const MAX_UPLOAD_RETRIES: u32 = 3;

pub fn device_workflow(
    device: DeviceInfo,
    program_path: &str,
    duration: AgingDuration,
    tx: Sender<WorkerMsg>,
    log_dir: &Path,
) {
    let ip = device.ip.clone();
    let mut flog = FileLogger::new(&log_dir.join(format!("{}.log", ip)));

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

    // --- 解压 ---
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
    extract_sess.set_timeout(0); // 解压可能很慢，不设超时

    let extract_cmd = format!("cd /data && tar -xavf {}", file_name);
    match ssh_exec_with_profile(&extract_sess, &extract_cmd) {
        Ok(output) => {
            flog.log(&format!("解压输出:\n{}", output));
            send_log(&tx, &ip, "解压完成");
        }
        Err(e) => {
            flog.log(&format!("解压命令报错: {}", e));
            send_log(&tx, &ip, &format!("解压警告: {}", e));
        }
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

    // --- 启动老化测试 ---
    send_state(&tx, &ip, DeviceState::Testing);
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
        "cd /data/bm1684x_soc_aging && nohup ./bm1684x_soc_aging --restart_all --run_time {} </dev/null >/data/aging_output.log 2>&1 & pid=$!; disown $pid 2>/dev/null; echo $pid",
        minutes
    );

    match ssh_exec_with_profile(&start_sess, &test_cmd) {
        Ok(output) => {
            flog.log(&format!("测试进程 PID: {}", output.trim()));
        }
        Err(e) => {
            let msg = format!("启动测试失败: {}", e);
            flog.log(&msg);
            send_state(&tx, &ip, DeviceState::Error(msg));
            return;
        }
    }
    drop(start_sess);

    // --- 轮询等待完成 ---
    let poll_interval = Duration::from_secs(30);
    let max_wait = Duration::from_secs((minutes as u64 + 10) * 60);
    let start_time = std::time::Instant::now();

    loop {
        std::thread::sleep(poll_interval);

        if start_time.elapsed() > max_wait {
            let msg = "测试超时, 超过预期运行时间".to_string();
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

        let check_cmd =
            "pgrep -f 'bm1684x_soc_aging --restart_all' > /dev/null 2>&1 && echo RUNNING || echo DONE";
        match ssh_exec(&poll_sess, check_cmd) {
            Ok(output) => {
                let status = output.trim();
                if status.contains("DONE") {
                    flog.log("测试进程已结束, 读取结果...");
                    send_log(&tx, &ip, "测试完成, 检查结果...");

                    match ssh_exec(&poll_sess, "cat /data/aging_output.log") {
                        Ok(log_output) => {
                            flog.log(&format!("测试输出:\n{}", log_output));

                            if log_output.contains("QA_AGING_PASS") {
                                send_log(&tx, &ip, "老化测试通过!");
                                send_state(&tx, &ip, DeviceState::Passed);
                            } else if log_output.contains("QA_AGING_FAILED") {
                                send_log(&tx, &ip, "老化测试失败!");
                                send_state(&tx, &ip, DeviceState::Failed);
                            } else {
                                let msg = "未检测到 QA_AGING_PASS/FAILED 标识".to_string();
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
        sess.set_keepalive(true, 15);

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
    sess.set_timeout(0); // 大文件传输不设超时

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
    let mut buf = vec![0u8; 64 * 1024];
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
