use std::fmt;
use std::net::Ipv4Addr;

#[derive(Debug, Clone)]
pub struct NetworkInterface {
    pub name: String,
    pub ip: Ipv4Addr,
    pub netmask: Ipv4Addr,
}

impl fmt::Display for NetworkInterface {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.name, self.ip)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum DeviceState {
    Discovered,
    Transferring(f32),
    Extracting,
    Testing,
    Passed,
    Failed,
    Error(String),
}

impl DeviceState {
    pub fn label(&self) -> String {
        match self {
            Self::Discovered => "已发现".into(),
            Self::Transferring(p) => format!("传输中 {:.0}%", p * 100.0),
            Self::Extracting => "解压中".into(),
            Self::Testing => "测试中".into(),
            Self::Passed => "通过".into(),
            Self::Failed => "失败".into(),
            Self::Error(e) => format!("错误: {}", e),
        }
    }
}

#[derive(Debug, Clone)]
pub struct DeviceInfo {
    pub sn: String,
    pub ip: String,
    pub mac: String,
    pub username: String,
    pub password: String,
    pub state: DeviceState,
    pub selected: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AgingDuration {
    Min1,
    Min5,
    Min10,
    Min20,
    Min30,
    Hour1,
    Hour3,
    Hour6,
    Hour24,
}

impl AgingDuration {
    pub fn minutes(&self) -> u32 {
        match self {
            Self::Min1 => 1,
            Self::Min5 => 5,
            Self::Min10 => 10,
            Self::Min20 => 20,
            Self::Min30 => 30,
            Self::Hour1 => 60,
            Self::Hour3 => 180,
            Self::Hour6 => 360,
            Self::Hour24 => 1440,
        }
    }

    pub const ALL: &'static [Self] = &[
        Self::Min1,
        Self::Min5,
        Self::Min10,
        Self::Min20,
        Self::Min30,
        Self::Hour1,
        Self::Hour3,
        Self::Hour6,
        Self::Hour24,
    ];
}

impl fmt::Display for AgingDuration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Min1 => write!(f, "1 分钟"),
            Self::Min5 => write!(f, "5 分钟"),
            Self::Min10 => write!(f, "10 分钟"),
            Self::Min20 => write!(f, "20 分钟"),
            Self::Min30 => write!(f, "30 分钟"),
            Self::Hour1 => write!(f, "1 小时"),
            Self::Hour3 => write!(f, "3 小时"),
            Self::Hour6 => write!(f, "6 小时"),
            Self::Hour24 => write!(f, "24 小时"),
        }
    }
}

pub enum WorkerMsg {
    Log(String),
    DeviceFound(DeviceInfo),
    ScanComplete,
    DeviceStateChanged { ip: String, state: DeviceState },
}
