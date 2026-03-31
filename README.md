# BM1684X 局域网批量老化工具

用于 BM1684X SoC 设备的局域网批量老化工具，支持批量扫描局域网设备、自动上传并运行老化测试程序。

## 功能

- 支持 Windows / Linux 双平台，Web 浏览器界面（兼容 Ubuntu 16.04、Windows 7 等老系统）
- 多网口选择，自动计算扫描范围
- 自动扫描局域网 SSH 设备（端口 22）
- 自动尝试多组凭据登录 (linaro/root/admin)
- SFTP 上传老化程序（含失败重试）
- 远程解压并执行老化测试
- 实时显示各设备测试进度和结果 (QA_AGING_PASS / QA_AGING_FAILED)
- 按设备 IP 独立保存测试日志

## 构建(docker容器中编译)

### Linux

```bash
./build-linux.sh
```

### Windows

在linux下交叉编译
```bash
./build-windows.sh
```

## 使用

1. 将默认老化程序 `bm1684x_soc_aging_v3_0_0.tgz` 放到可执行文件同级目录
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
