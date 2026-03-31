# BM1684X 工厂产测工具

用于 BM1684X SoC 设备的工厂老化测试工具，支持批量扫描局域网设备、自动上传并运行老化测试程序。

## 功能

- 支持 Windows / Linux 双平台，Web 浏览器界面（兼容 Ubuntu 16.04、Windows 7 等老系统）
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
sudo apt install -y build-essential cmake pkg-config libssl-dev
```

### Linux (Arch)

```bash
sudo pacman -S base-devel cmake openssl
```

### Windows

需要安装 [Visual Studio Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/) 和 [CMake](https://cmake.org/)。

## 编译

### 本机编译

```bash
cargo build --release
```

编译产物位于 `target/release/fac_test`。

如需生成静态链接版本（需安装 musl 工具链）：

```bash
# Arch: sudo pacman -S musl
# Debian/Ubuntu: sudo apt install musl-tools
rustup target add x86_64-unknown-linux-musl
cargo build --release --target x86_64-unknown-linux-musl
```

静态链接产物位于 `target/x86_64-unknown-linux-musl/release/fac_test`。

### Docker 容器编译（推荐）

#### Linux (Alpine musl 静态链接, 兼容所有 Linux 发行版)

```bash
./build-linux.sh
```

基于 Alpine 容器使用 musl 编译，生成完全静态链接的二进制文件。无需依赖任何系统动态库，可直接在 Ubuntu 16.04、CentOS 7、Debian 8 等任意 Linux 发行版上运行。

#### Windows (32位版本)

```bash
./build-windows.sh
```

## 使用

1. 将默认产测程序 `bm1684x_soc_aging_v3_0_0.tgz` 放到可执行文件同级目录
2. 运行程序（默认端口 8080，可通过参数指定: `./fac_test 9090`）
3. 程序会自动打开浏览器，也可手动访问 `http://localhost:8080`
4. 选择网络接口，调整扫描 IP 范围
5. 选择老化时间
6. 点击「开始扫描」发现设备
7. 确认设备列表后开始测试
8. 测试日志保存在可执行文件同级 `logs/` 目录下

## 浏览器兼容性

Web UI 使用基础 HTML/CSS/JavaScript，兼容以下浏览器：

- Chrome 30+
- Firefox 30+
- IE 11
- Edge (所有版本)
