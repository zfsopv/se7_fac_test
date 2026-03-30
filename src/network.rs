use crate::types::*;
use std::io::Read;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream};
use std::sync::mpsc::Sender;
use std::time::Duration;

pub fn get_interfaces() -> Vec<NetworkInterface> {
    let mut interfaces = Vec::new();
    if let Ok(addrs) = if_addrs::get_if_addrs() {
        for iface in addrs {
            if let if_addrs::IfAddr::V4(v4) = &iface.addr {
                if !v4.ip.is_loopback() {
                    interfaces.push(NetworkInterface {
                        name: iface.name.clone(),
                        ip: v4.ip,
                        netmask: v4.netmask,
                    });
                }
            }
        }
    }
    interfaces
}

pub fn compute_scan_range(iface: &NetworkInterface) -> (String, String) {
    let ip = u32::from(iface.ip);
    let mask = u32::from(iface.netmask);
    let network = ip & mask;
    let broadcast = network | !mask;
    let range = broadcast - network;

    if range > 1024 {
        let subnet_base = ip & 0xFFFF_FF00;
        let start = Ipv4Addr::from(subnet_base + 1);
        let end = Ipv4Addr::from(subnet_base + 254);
        (start.to_string(), end.to_string())
    } else {
        let start = Ipv4Addr::from(network + 1);
        let end = Ipv4Addr::from(broadcast - 1);
        (start.to_string(), end.to_string())
    }
}

pub fn scan_network(start: Ipv4Addr, end: Ipv4Addr, tx: Sender<WorkerMsg>) {
    let start_u32 = u32::from(start);
    let end_u32 = u32::from(end);

    if start_u32 > end_u32 {
        tx.send(WorkerMsg::Log("错误: 起始 IP 大于结束 IP".into()))
            .ok();
        tx.send(WorkerMsg::ScanComplete).ok();
        return;
    }

    let total = (end_u32 - start_u32 + 1) as usize;
    tx.send(WorkerMsg::Log(format!(
        "开始扫描 {} - {} (共 {} 个地址)",
        start, end, total
    )))
    .ok();

    let ips: Vec<Ipv4Addr> = (start_u32..=end_u32).map(Ipv4Addr::from).collect();
    let chunk_size = 32;

    for (chunk_idx, chunk) in ips.chunks(chunk_size).enumerate() {
        let mut handles = Vec::new();

        for &ip in chunk {
            let tx = tx.clone();
            let handle = std::thread::spawn(move || {
                scan_host(ip, &tx);
            });
            handles.push(handle);
        }

        for handle in handles {
            handle.join().ok();
        }

        let scanned = ((chunk_idx + 1) * chunk_size).min(total);
        tx.send(WorkerMsg::Log(format!("扫描进度: {}/{}", scanned, total)))
            .ok();
    }

    tx.send(WorkerMsg::ScanComplete).ok();
}

fn scan_host(ip: Ipv4Addr, tx: &Sender<WorkerMsg>) {
    let addr = SocketAddr::new(IpAddr::V4(ip), 22);
    let ip_str = ip.to_string();

    // 快速探测端口
    let first_tcp = match TcpStream::connect_timeout(&addr, Duration::from_secs(2)) {
        Ok(t) => t,
        Err(_) => return,
    };

    tx.send(WorkerMsg::Log(format!("{}: SSH 端口开放", ip_str)))
        .ok();

    let creds = [("linaro", "linaro"), ("root", "ematech"), ("admin", "admin")];
    let mut first_tcp = Some(first_tcp);

    for (user, pass) in &creds {
        // 第一组凭据复用端口探测的 TCP 连接，后续每次新建（带延时）
        let tcp = if let Some(t) = first_tcp.take() {
            t
        } else {
            std::thread::sleep(Duration::from_secs(1));
            match TcpStream::connect_timeout(&addr, Duration::from_secs(5)) {
                Ok(t) => t,
                Err(e) => {
                    tx.send(WorkerMsg::Log(format!(
                        "{}: 重连失败: {}", ip_str, e
                    ))).ok();
                    continue;
                }
            }
        };

        // 握手
        let sess = match try_handshake(tcp) {
            Ok(s) => s,
            Err(e) => {
                tx.send(WorkerMsg::Log(format!(
                    "{}: {}:{} 握手失败: {}", ip_str, user, pass, e
                ))).ok();
                continue;
            }
        };

        // 认证
        if let Err(e) = sess.userauth_password(user, pass) {
            tx.send(WorkerMsg::Log(format!(
                "{}: {}:{} 认证失败 - {}", ip_str, user, pass, e
            ))).ok();
            continue;
        }

        if !sess.authenticated() {
            tx.send(WorkerMsg::Log(format!(
                "{}: {}:{} 未通过认证", ip_str, user, pass
            ))).ok();
            continue;
        }

        tx.send(WorkerMsg::Log(format!(
            "{}: 使用 {}:{} 登录成功", ip_str, user, pass
        ))).ok();

        let sn = match get_device_sn(&sess) {
            Some(sn) => sn,
            None => {
                tx.send(WorkerMsg::Log(format!(
                    "{}: bm_get_basic_info 未返回有效 SN, 跳过", ip_str
                ))).ok();
                return;
            }
        };

        let mac = get_device_mac(&sess).unwrap_or_else(|| "unknown".to_string());

        tx.send(WorkerMsg::Log(format!(
            "{}: SN={}, MAC={}", ip_str, sn, mac
        ))).ok();

        tx.send(WorkerMsg::DeviceFound(DeviceInfo {
            sn,
            ip: ip_str,
            mac,
            username: user.to_string(),
            password: pass.to_string(),
            state: DeviceState::Discovered,
            selected: true,
        })).ok();
        return;
    }

    tx.send(WorkerMsg::Log(format!("{}: 所有凭据均失败", ip_str)))
        .ok();
}

fn try_handshake(tcp: TcpStream) -> Result<ssh2::Session, String> {
    tcp.set_nodelay(true).ok();
    tcp.set_read_timeout(Some(Duration::from_secs(30))).ok();
    tcp.set_write_timeout(Some(Duration::from_secs(30))).ok();

    let mut sess = ssh2::Session::new().map_err(|e| e.to_string())?;
    sess.set_tcp_stream(tcp);
    // 握手期间不设 libssh2 层超时，完全依赖 TCP 层的 30 秒超时
    sess.set_timeout(0);
    sess.set_blocking(true);
    sess.handshake().map_err(|e| format!("{}", e))?;
    // 握手成功后再设置会话参数
    sess.set_timeout(30_000);
    sess.set_keepalive(true, 15);
    Ok(sess)
}

fn get_device_sn(sess: &ssh2::Session) -> Option<String> {
    let mut channel = sess.channel_session().ok()?;
    channel
        .exec("source /etc/profile 2>/dev/null; bm_get_basic_info")
        .ok()?;
    let mut output = String::new();
    channel.read_to_string(&mut output).ok()?;
    channel.wait_close().ok();
    parse_sn(&output)
}

fn get_device_mac(sess: &ssh2::Session) -> Option<String> {
    let mut channel = sess.channel_session().ok()?;
    channel
        .exec("cat /sys/class/net/eth0/address 2>/dev/null || ip link show | grep ether | head -1 | awk '{print $2}'")
        .ok()?;
    let mut mac = String::new();
    channel.read_to_string(&mut mac).ok()?;
    channel.wait_close().ok();
    let mac = mac.trim().to_string();
    if mac.is_empty() { None } else { Some(mac) }
}

/// 解析 bm_get_basic_info 输出，优先使用 device sn，fallback 到 chip sn
fn parse_sn(output: &str) -> Option<String> {
    let mut device_sn = None;
    let mut chip_sn = None;

    for line in output.lines() {
        let line = line.trim();
        if let Some(val) = line.strip_prefix("device sn:") {
            let val = val.trim();
            if !val.is_empty() {
                device_sn = Some(val.to_string());
            }
        } else if let Some(val) = line.strip_prefix("chip sn:") {
            let val = val.trim();
            if !val.is_empty() {
                chip_sn = Some(val.to_string());
            }
        }
    }

    device_sn.or(chip_sn)
}
