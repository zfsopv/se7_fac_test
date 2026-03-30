# BM1684X 工厂产测工具

用于 BM1684X SoC 设备的工厂老化测试工具，支持批量扫描局域网设备、自动上传并运行老化测试程序。

## 功能

- 支持 Windows / Linux 双平台，中文图形界面
- 多网口选择，自动计算扫描范围
- 自动扫描局域网 SSH 设备（端口 22）
- 自动尝试多组凭据登录 (linaro/root/admin)
- SFTP 上传产测程序（含失败重试）
- 远程解压并执行老化测试
- 实时显示各设备测试进度和结果 (QA_AGING_PASS / QA_AGING_FAILED)
- 按设备 IP 独立保存测试日志

## 构建依赖

### Linux (Debian/Ubuntu)

```bash
sudo apt install -y build-essential cmake pkg-config \
    libssl-dev libssh2-1-dev \
    libxkbcommon-dev libwayland-dev libgtk-3-dev
```

### Linux (Arch)

```bash
sudo pacman -S base-devel cmake openssl libssh2 \
    wayland libxkbcommon gtk3
```

### Windows

需要安装 [Visual Studio Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/) 和 [CMake](https://cmake.org/)。OpenSSL 可通过 [vcpkg](https://vcpkg.io/) 安装。

## 编译

```bash
cargo build --release
```

编译产物位于 `target/release/fac_test`。

## 使用

1. 将默认产测程序 `bm1684x_soc_aging_v3_0_0.tgz` 放到可执行文件同级目录
2. 运行程序
3. 选择网络接口，调整扫描 IP 范围
4. 选择老化时间
5. 点击「开始扫描」发现设备
6. 确认设备列表后开始测试
7. 测试日志保存在可执行文件同级 `logs/` 目录下

## 中文字体

程序会自动加载系统 CJK 字体。如果中文显示为方块，请安装以下任一字体包：

- **Noto Sans CJK**: `noto-fonts-cjk` (Arch) / `fonts-noto-cjk` (Debian)
- **文泉驿微米黑**: `wqy-microhei` (Arch) / `fonts-wqy-microhei` (Debian)
- **Windows**: 已自带微软雅黑
