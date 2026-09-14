# fix_tmp_daemon

Android 后台守护进程：监控 `com.suseoaa.locationspoofer` 运行状态，在其退出后自动将 `/data/local/tmp` 权限恢复为 `771`（所有者 `shell:shell`）。

## 特性

- 纯 Rust 实现，仅依赖 `libc`，二进制体积 < 300KB
- double-fork daemonize，完全脱离终端运行
- 直接读取 `/proc` 判断进程状态，0 子进程开销
- libc 系统调用修改权限，不经过 Shell 解析
- 每 5 秒巡检，线程级挂起，极致省电
- 内置文件日志，便于排查问题

## 快速开始

1. 推送代码到 GitHub，Actions 自动编译，下载 Artifact
2. 将 `fix_tmp_daemon` 和 `99_start_rust.sh` 放入 `/data/adb/service.d/`
3. 权限设为 755，重启手机

详细说明请参阅 [部署说明.md](部署说明.md)。

## 项目结构

```
fix_tmp_daemon/
├── Cargo.toml                  # 项目配置 + release 体积优化
├── src/main.rs                 # 守护进程完整源码
├── 99_start_rust.sh            # KernelSU 开机自启脚本
├── 部署说明.md                  # 详细部署文档
├── .gitignore
└── .github/workflows/build.yml # GitHub Actions 自动交叉编译
```
