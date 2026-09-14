use std::ffi::CString;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::thread;
use std::time::Duration;

// ============================================================
// 常量配置
// ============================================================
const TARGET_PKG: &str = "com.suseoaa.locationspoofer";
const TMP_DIR: &str = "/data/local/tmp";
const LOG_FILE: &str = "/data/adb/service.d/fix_tmp_daemon.log";
const POLL_INTERVAL: u64 = 5;     // 巡检间隔（秒）
const EXIT_CONFIRM_DELAY: u64 = 3; // 退出确认延迟（秒）
const BOOT_POLL_INTERVAL: u64 = 2; // 开机等待轮询间隔（秒）
const BOOT_STABILIZE_DELAY: u64 = 10; // 开机后稳定等待（秒）

// ============================================================
// 日志工具 —— 守护进程没有 stdout，写入文件
// ============================================================
fn log(msg: &str) {
    if let Ok(mut f) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(LOG_FILE)
    {
        let ts = format_epoch();
        let _ = writeln!(f, "[{}] {}", ts, msg);
    }
}

fn format_epoch() -> String {
    // 用 libc 获取系统时间，避免引入 time crate
    let mut tv: libc::timeval = unsafe { std::mem::zeroed() };
    unsafe { libc::gettimeofday(&mut tv, std::ptr::null_mut()) };
    format!("{}", tv.tv_sec)
}

// ============================================================
// 守护进程化（double-fork + setsid）
// ============================================================
fn daemonize() -> Result<(), String> {
    // 第一次 fork
    let pid = unsafe { libc::fork() };
    if pid < 0 {
        return Err("第一次 fork 失败".to_string());
    }
    if pid > 0 {
        // 父进程退出
        unsafe { libc::_exit(0) };
    }

    // 创建新会话，脱离控制终端
    if unsafe { libc::setsid() } < 0 {
        return Err("setsid 失败".to_string());
    }

    // 第二次 fork，确保进程无法重新获取控制终端
    let pid = unsafe { libc::fork() };
    if pid < 0 {
        return Err("第二次 fork 失败".to_string());
    }
    if pid > 0 {
        unsafe { libc::_exit(0) };
    }

    // 更改工作目录到根目录，避免占用挂载点
    let root = CString::new("/").unwrap();
    unsafe { libc::chdir(root.as_ptr()) };

    // 重定向 stdin/stdout/stderr 到 /dev/null
    let devnull = CString::new("/dev/null").unwrap();
    unsafe {
        let fd = libc::open(devnull.as_ptr(), libc::O_RDWR);
        if fd >= 0 {
            libc::dup2(fd, libc::STDIN_FILENO);
            libc::dup2(fd, libc::STDOUT_FILENO);
            libc::dup2(fd, libc::STDERR_FILENO);
            if fd > 2 {
                libc::close(fd);
            }
        }
    }

    // 设置 umask
    unsafe { libc::umask(0) };

    Ok(())
}

// ============================================================
// 等待系统开机完成
// ============================================================
fn wait_for_boot() {
    loop {
        let boot_completed = std::process::Command::new("getprop")
            .arg("sys.boot_completed")
            .output()
            .map(|out| String::from_utf8_lossy(&out.stdout).trim() == "1")
            .unwrap_or(false);

        if boot_completed {
            log("系统开机完成，等待稳定...");
            break;
        }
        thread::sleep(Duration::from_secs(BOOT_POLL_INTERVAL));
    }
    thread::sleep(Duration::from_secs(BOOT_STABILIZE_DELAY));
    log("系统已稳定，开始监控循环");
}

// ============================================================
// 通过 /proc 判断目标应用是否在运行（0 子进程开销）
// ============================================================
fn is_app_running() -> bool {
    let proc_dir = match fs::read_dir("/proc") {
        Ok(d) => d,
        Err(e) => {
            log(&format!("读取 /proc 失败: {}", e));
            return false;
        }
    };

    for entry in proc_dir.flatten() {
        // 过滤出纯数字目录（PID）
        let file_name = match entry.file_name().into_string() {
            Ok(s) => s,
            Err(_) => continue,
        };

        if !file_name.chars().all(char::is_numeric) {
            continue;
        }

        let cmdline_path = format!("/proc/{}/cmdline", file_name);
        if let Ok(cmdline_bytes) = fs::read(&cmdline_path) {
            // cmdline 中参数以 \0 分隔，直接在字节层面搜索包名
            if cmdline_bytes
                .windows(TARGET_PKG.len())
                .any(|w| w == TARGET_PKG.as_bytes())
            {
                return true;
            }
        }
    }
    false
}

// ============================================================
// 通过 libc 直接系统调用修改权限和所有者
// ============================================================
fn fix_permissions() -> Result<(), String> {
    let path_c = CString::new(TMP_DIR).map_err(|_| "路径转换失败".to_string())?;

    // chmod 771 = rwxrwx--x
    let chmod_ret = unsafe { libc::chmod(path_c.as_ptr(), 0o771) };
    if chmod_ret != 0 {
        return Err(format!("chmod 失败，errno: {}", unsafe { *libc::__errno() }));
    }

    // chown shell:shell (UID 2000, GID 2000)
    let chown_ret = unsafe { libc::chown(path_c.as_ptr(), 2000, 2000) };
    if chown_ret != 0 {
        return Err(format!("chown 失败，errno: {}", unsafe { *libc::__errno() }));
    }

    Ok(())
}

// ============================================================
// 验证权限是否已正确设置
// ============================================================
fn verify_permissions() -> bool {
    match fs::metadata(TMP_DIR) {
        Ok(meta) => {
            use std::os::unix::fs::MetadataExt;
            let mode = meta.mode() & 0o777;
            let uid = meta.uid();
            let gid = meta.gid();
            mode == 0o771 && uid == 2000 && gid == 2000
        }
        Err(_) => false,
    }
}

// ============================================================
// 主函数
// ============================================================
fn main() {
    // 1. 守护进程化
    if let Err(e) = daemonize() {
        // daemonize 失败时退化为前台运行（不影响功能）
        eprintln!("daemonize 失败（{}），将以前台模式运行", e);
        log(&format!("daemonize 失败（{}），以前台模式运行", e));
    }

    log("===== fix_tmp_daemon 启动 =====");
    log(&format!("监控目标: {}", TARGET_PKG));
    log(&format!("目标目录: {}", TMP_DIR));

    // 2. 等待系统开机完成
    wait_for_boot();

    // 3. 启动时先确保权限正确（应对应用在开机前就改过的情况）
    if !is_app_running() {
        match fix_permissions() {
            Ok(_) => log("启动时权限修复成功"),
            Err(e) => log(&format!("启动时权限修复失败: {}", e)),
        }
    }

    // 4. 进入无限监控循环
    let mut was_running = is_app_running();
    if was_running {
        log("检测到目标应用正在运行");
    }

    loop {
        let running = is_app_running();

        if running {
            if !was_running {
                // 应用刚启动，标记状态，不干扰
                log("目标应用已启动，暂停权限修复");
                was_running = true;
            }
        } else {
            if was_running {
                // 应用刚退出，等待确认
                log("目标应用已退出，等待 3 秒确认...");
                thread::sleep(Duration::from_secs(EXIT_CONFIRM_DELAY));

                if !is_app_running() {
                    // 确认完全退出，执行修复
                    log("确认应用已完全退出，开始修复权限...");
                    match fix_permissions() {
                        Ok(_) => {
                            if verify_permissions() {
                                log("权限修复成功: 771 shell:shell");
                            } else {
                                log("权限修复执行成功，但验证未通过");
                            }
                        }
                        Err(e) => log(&format!("权限修复失败: {}", e)),
                    }
                    was_running = false;
                } else {
                    log("应用仍有残留进程，保持监控");
                    // was_running 保持 true，下一轮继续判断
                }
            }
        }

        thread::sleep(Duration::from_secs(POLL_INTERVAL));
    }
}
