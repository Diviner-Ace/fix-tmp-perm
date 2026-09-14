#!/system/bin/sh
# ============================================================
# fix_tmp_daemon 开机自启脚本
# 放置路径: /data/adb/service.d/99_start_rust.sh
# 权限要求: 755 (rwxr-xr-x)
# 换行符:   必须为 LF (Unix)，不能是 CRLF (Windows)
# ============================================================

# 等待系统属性服务就绪
while [ "$(getprop sys.boot_completed)" != "1" ]; do
    sleep 1
done

# 后台启动 Rust 守护进程（Rust 内部会再次 daemonize）
/data/adb/service.d/fix_tmp_daemon &

# 记录启动日志
echo "$(date): fix_tmp_daemon started" >> /data/adb/service.d/fix_tmp_daemon.log
